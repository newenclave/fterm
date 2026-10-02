//! The pty read and write loop. It runs on its own thread.
//!
//! This is a port of `alacritty_terminal::event_loop` (Apache-2.0, the Alacritty authors) with two changes:
//! - the bytes go through our OSC scanner before the parser, so fterm sees shell integration,
//!   notifications, and agent states (alacritty drops these sequences);
//! - after the child process ends, we read until the output is quiet, so the last output is not lost
//!   (ConPTY can send it a bit after the process ends).

use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::{self, ErrorKind, Read, Write};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{self, Event, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::tty;
#[cfg(windows)]
use alacritty_terminal::tty::{PTY_CHILD_EVENT_TOKEN, PTY_READ_WRITE_TOKEN};
use alacritty_terminal::vte::ansi;
use polling::{Event as PollingEvent, Events, PollMode, Poller};

use crate::osc::{OscEvent, PromptMark, Scanner};

// alacritty registers the pty with these poll keys. On Unix they are not public, so here is a copy
// (alacritty_terminal 0.26, `tty/unix.rs`).
#[cfg(not(windows))]
const PTY_READ_WRITE_TOKEN: usize = 0;
#[cfg(not(windows))]
const PTY_CHILD_EVENT_TOKEN: usize = 1;

/// Max bytes to read from the pty before the terminal is drawn again.
const READ_BUFFER_SIZE: usize = 0x10_0000;
/// Max bytes to read while the terminal is locked.
const MAX_LOCKED_READ: usize = u16::MAX as usize;
/// After the child ends: stop reading when no new output came for this long ...
const DRAIN_QUIET: Duration = Duration::from_millis(150);
/// ... or after this long in total.
const DRAIN_MAX: Duration = Duration::from_secs(2);
/// The longest wait for pty events. A child that ends before the pty is registered sends its exit
/// event to nobody, so we also look for it after each wait.
const MAX_WAIT: Duration = Duration::from_millis(200);

/// Messages to the loop.
#[derive(Debug)]
pub enum Msg {
    /// Bytes to write to the pty.
    Input(Cow<'static, [u8]>),
    Shutdown,
    Resize(WindowSize),
}

/// Gets the OSC events that the scanner found (on the pty thread).
pub type OscSink = Arc<dyn Fn(Vec<OscEvent>) + Send + Sync>;

pub struct IoLoop<T: tty::EventedPty, U: EventListener> {
    poll: Arc<Poller>,
    pty: T,
    rx: PeekableReceiver<Msg>,
    tx: Sender<Msg>,
    terminal: Arc<FairMutex<Term<U>>>,
    event_proxy: U,
    scanner: Scanner,
    osc_sink: OscSink,
    /// `FTERM_RECORD`: what the program writes goes here too.
    recorder: crate::record::Shared,
}

impl<T, U> IoLoop<T, U>
where
    T: tty::EventedPty + event::OnResize + Send + 'static,
    U: EventListener + Send + 'static,
{
    pub fn new(
        terminal: Arc<FairMutex<Term<U>>>,
        event_proxy: U,
        pty: T,
        osc_sink: OscSink,
        recorder: crate::record::Shared,
    ) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        Ok(Self {
            poll: Poller::new()?.into(),
            pty,
            tx,
            rx: PeekableReceiver::new(rx),
            terminal,
            event_proxy,
            scanner: Scanner::default(),
            osc_sink,
            recorder,
        })
    }

    pub fn channel(&self) -> LoopSender {
        LoopSender {
            sender: self.tx.clone(),
            poller: self.poll.clone(),
        }
    }

    /// Reads the channel. Returns `false` when a shutdown message came.
    fn drain_recv_channel(&mut self, state: &mut State) -> bool {
        while let Some(msg) = self.rx.recv() {
            match msg {
                Msg::Input(input) => state.write_list.push_back(input),
                Msg::Resize(window_size) => self.pty.on_resize(window_size),
                Msg::Shutdown => return false,
            }
        }
        true
    }

    /// Reads what the pty has now. Returns the number of bytes read.
    fn pty_read(&mut self, state: &mut State, buf: &mut [u8]) -> io::Result<usize> {
        let mut unprocessed = 0;
        let mut processed = 0;
        let mut osc = Vec::new();
        let mut found = Vec::new();

        // Reserve the next terminal lock for pty reading.
        let _terminal_lease = Some(self.terminal.lease());
        let mut terminal = None;

        loop {
            match self.pty.reader().read(&mut buf[unprocessed..]) {
                // Windows and macOS give this when there is nothing more to read.
                Ok(0) if unprocessed == 0 => break,
                Ok(got) => {
                    let new = &buf[unprocessed..unprocessed + got];
                    crate::record::with(&self.recorder, |r| r.output(new, Instant::now()));
                    unprocessed += got;
                }
                Err(err) => match err.kind() {
                    ErrorKind::Interrupted | ErrorKind::WouldBlock => {
                        if unprocessed == 0 {
                            break;
                        }
                    }
                    _ => return Err(err),
                },
            }

            let terminal = match &mut terminal {
                Some(terminal) => terminal,
                None => terminal.insert(match self.terminal.try_lock_unfair() {
                    // Block when the buffer is full.
                    None if unprocessed >= READ_BUFFER_SIZE => self.terminal.lock_unfair(),
                    None => continue,
                    Some(terminal) => terminal,
                }),
            };

            // Our OSC sequences first (the scanner only reads), then the parser. At 133;B the parser
            // runs only up to the end of that sequence, so we can read where the typed text starts.
            found.clear();
            self.scanner.feed_at(&buf[..unprocessed], &mut found);
            let mut from = 0;
            for (end, event) in found.drain(..) {
                // At 133;B the typed text starts; at 133;C and 133;D the output starts and ends.
                let mark = match &event {
                    OscEvent::Prompt(PromptMark::CommandStart) => Some(true),
                    OscEvent::Prompt(
                        PromptMark::CommandExecuted | PromptMark::CommandFinished(_),
                    ) => Some(false),
                    _ => None,
                };
                osc.push(event);
                if let Some(input) = mark {
                    state.parser.advance(&mut **terminal, &buf[from..end]);
                    from = end;
                    let grid = terminal.grid();
                    let cursor = grid.cursor.point;
                    let line = (grid.history_size() as i32 + cursor.line.0).max(0) as usize;
                    let column = cursor.column.0;
                    osc.push(if input {
                        OscEvent::InputStart { line, column }
                    } else {
                        OscEvent::OutputMark { line, column }
                    });
                }
            }
            state
                .parser
                .advance(&mut **terminal, &buf[from..unprocessed]);

            processed += unprocessed;
            unprocessed = 0;
            if processed >= MAX_LOCKED_READ {
                break;
            }
        }

        if !osc.is_empty() {
            (self.osc_sink)(osc);
        }
        if state.parser.sync_bytes_count() < processed && processed > 0 {
            self.event_proxy.send_event(Event::Wakeup);
        }
        Ok(processed)
    }

    fn pty_write(&mut self, state: &mut State) -> io::Result<()> {
        state.ensure_next();

        'write_many: while let Some(mut current) = state.take_current() {
            'write_one: loop {
                match self.pty.writer().write(current.remaining_bytes()) {
                    Ok(0) => {
                        state.set_current(Some(current));
                        break 'write_many;
                    }
                    Ok(n) => {
                        current.advance(n);
                        if current.finished() {
                            state.goto_next();
                            break 'write_one;
                        }
                    }
                    Err(err) => {
                        state.set_current(Some(current));
                        match err.kind() {
                            ErrorKind::Interrupted | ErrorKind::WouldBlock => break 'write_many,
                            _ => return Err(err),
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// After the child ended: read until the output is quiet.
    fn drain(&mut self, state: &mut State, buf: &mut [u8]) {
        let start = Instant::now();
        let mut last_data = start;
        while start.elapsed() < DRAIN_MAX && last_data.elapsed() < DRAIN_QUIET {
            match self.pty_read(state, buf) {
                Ok(0) => std::thread::sleep(Duration::from_millis(10)),
                Ok(_) => last_data = Instant::now(),
                Err(_) => break,
            }
        }
    }

    fn child_exited(
        &mut self,
        state: &mut State,
        buf: &mut [u8],
        status: Option<std::process::ExitStatus>,
    ) {
        if let Some(status) = status {
            self.event_proxy.send_event(Event::ChildExit(status));
        }
        self.drain(state, buf);
        self.terminal.lock().exit();
        self.event_proxy.send_event(Event::Wakeup);
    }

    pub fn spawn(mut self) -> JoinHandle<()> {
        std::thread::Builder::new()
            .name("pty".into())
            .spawn(move || {
                let mut state = State::default();
                let mut buf = vec![0u8; READ_BUFFER_SIZE];

                let poll_opts = PollMode::Level;
                let mut interest = PollingEvent::readable(0);

                // SAFETY: the pty is registered once and deregistered before this thread ends,
                // while `self.pty` (the source) is still alive. This is what alacritty does.
                #[allow(unsafe_code)]
                let registered = unsafe { self.pty.register(&self.poll, interest, poll_opts) };
                if let Err(err) = registered {
                    tracing::error!("pty loop registration error: {err}");
                    return;
                }

                let mut events = Events::with_capacity(NonZeroUsize::new(1024).unwrap());

                'event_loop: loop {
                    // Wake up when a synchronized update times out.
                    let handler = state.parser.sync_timeout();
                    let timeout = handler
                        .sync_timeout()
                        .map(|st| st.saturating_duration_since(Instant::now()))
                        .map_or(MAX_WAIT, |t| t.min(MAX_WAIT));

                    events.clear();
                    if let Err(err) = self.poll.wait(&mut events, Some(timeout)) {
                        match err.kind() {
                            ErrorKind::Interrupted => continue,
                            _ => {
                                tracing::error!("pty loop polling error: {err}");
                                break 'event_loop;
                            }
                        }
                    }

                    // The child may have ended without a poll event (see MAX_WAIT).
                    if let Some(tty::ChildEvent::Exited(status)) = self.pty.next_child_event() {
                        self.child_exited(&mut state, &mut buf, status);
                        break 'event_loop;
                    }

                    if events.is_empty() && self.rx.peek().is_none() {
                        if handler.sync_timeout().is_some() {
                            state.parser.stop_sync(&mut *self.terminal.lock());
                            self.event_proxy.send_event(Event::Wakeup);
                        }
                        continue;
                    }

                    if !self.drain_recv_channel(&mut state) {
                        break;
                    }

                    for event in events.iter() {
                        match event.key {
                            PTY_CHILD_EVENT_TOKEN => {
                                if let Some(tty::ChildEvent::Exited(status)) =
                                    self.pty.next_child_event()
                                {
                                    self.child_exited(&mut state, &mut buf, status);
                                    break 'event_loop;
                                }
                            }
                            PTY_READ_WRITE_TOKEN => {
                                if event.is_interrupt() {
                                    continue;
                                }
                                if event.readable
                                    && let Err(err) = self.pty_read(&mut state, &mut buf)
                                {
                                    // On Linux, a read can fail with EIO when the child hangs up.
                                    // The `Exited` event comes next.
                                    #[cfg(target_os = "linux")]
                                    if err.raw_os_error() == Some(libc_eio()) {
                                        continue;
                                    }
                                    tracing::error!("pty read error: {err}");
                                    break 'event_loop;
                                }
                                if event.writable
                                    && let Err(err) = self.pty_write(&mut state)
                                {
                                    tracing::error!("pty write error: {err}");
                                    break 'event_loop;
                                }
                            }
                            _ => {}
                        }
                    }

                    let needs_write = state.needs_write();
                    if needs_write != interest.writable {
                        interest.writable = needs_write;
                        if let Err(err) = self.pty.reregister(&self.poll, interest, poll_opts) {
                            tracing::error!("pty loop reregister error: {err}");
                            break 'event_loop;
                        }
                    }
                }

                let _ = self.pty.deregister(&self.poll);
            })
            .expect("cannot start the pty thread")
    }
}

#[cfg(target_os = "linux")]
fn libc_eio() -> i32 {
    5
}

/// Sends messages to the loop.
#[derive(Clone)]
pub struct LoopSender {
    sender: Sender<Msg>,
    poller: Arc<Poller>,
}

impl LoopSender {
    pub fn send(&self, msg: Msg) {
        if self.sender.send(msg).is_ok() {
            let _ = self.poller.notify();
        }
    }
}

/// Writes to the pty and resizes it, like alacritty's `Notifier`.
pub struct Notifier(pub LoopSender);

impl event::Notify for Notifier {
    fn notify<B>(&self, bytes: B)
    where
        B: Into<Cow<'static, [u8]>>,
    {
        let bytes = bytes.into();
        // The terminal hangs if we send 0 bytes.
        if !bytes.is_empty() {
            self.0.send(Msg::Input(bytes));
        }
    }
}

impl event::OnResize for Notifier {
    fn on_resize(&mut self, window_size: WindowSize) {
        self.0.send(Msg::Resize(window_size));
    }
}

/// A buffer that is being written, and how much of it is written.
struct Writing {
    source: Cow<'static, [u8]>,
    written: usize,
}

impl Writing {
    fn new(source: Cow<'static, [u8]>) -> Self {
        Self { source, written: 0 }
    }
    fn advance(&mut self, n: usize) {
        self.written += n;
    }
    fn remaining_bytes(&self) -> &[u8] {
        &self.source[self.written..]
    }
    fn finished(&self) -> bool {
        self.written >= self.source.len()
    }
}

#[derive(Default)]
struct State {
    write_list: VecDeque<Cow<'static, [u8]>>,
    writing: Option<Writing>,
    parser: ansi::Processor,
}

impl State {
    fn ensure_next(&mut self) {
        if self.writing.is_none() {
            self.goto_next();
        }
    }
    fn goto_next(&mut self) {
        self.writing = self.write_list.pop_front().map(Writing::new);
    }
    fn take_current(&mut self) -> Option<Writing> {
        self.writing.take()
    }
    fn needs_write(&self) -> bool {
        self.writing.is_some() || !self.write_list.is_empty()
    }
    fn set_current(&mut self, new: Option<Writing>) {
        self.writing = new;
    }
}

struct PeekableReceiver<T> {
    rx: Receiver<T>,
    peeked: Option<T>,
}

impl<T> PeekableReceiver<T> {
    fn new(rx: Receiver<T>) -> Self {
        Self { rx, peeked: None }
    }

    fn peek(&mut self) -> Option<&T> {
        if self.peeked.is_none() {
            self.peeked = self.rx.try_recv().ok();
        }
        self.peeked.as_ref()
    }

    fn recv(&mut self) -> Option<T> {
        if self.peeked.is_some() {
            self.peeked.take()
        } else {
            match self.rx.try_recv() {
                // The terminal state keeps a sender, so this cannot happen while the loop runs.
                Err(TryRecvError::Disconnected) => panic!("the pty loop channel is closed"),
                res => res.ok(),
            }
        }
    }
}
