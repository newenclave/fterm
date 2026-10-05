//! A terminal session: a shell in a pty, the parser, and the grid.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use crate::io_loop::{IoLoop, LoopSender, Msg, Notifier};
use crate::osc::OscEvent;
use alacritty_terminal::event::{Event, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
#[cfg(not(windows))]
use alacritty_terminal::tty;

use crate::select::StickySelection;
use crate::size::GridSize;

/// Events from the session. They come from the pty thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermEvent {
    /// New output: draw the window again.
    Redraw,
    /// The app changed the window title.
    Title(String),
    /// The shell process ended.
    Exit,
    /// Shell integration, a notification, or an agent state (from our OSC scanner).
    Osc(OscEvent),
    /// The bell (BEL).
    Bell,
}

/// What to run in the session.
#[derive(Clone, Debug)]
pub struct SessionOptions {
    /// Program to run. `None` means the default shell.
    pub program: Option<String>,
    pub args: Vec<String>,
    /// The start folder. `None` = the folder of fterm.
    pub cwd: Option<std::path::PathBuf>,
    /// More environment variables for the program.
    pub env: Vec<(String, String)>,
    /// Lines of history.
    pub scrollback: usize,
    /// Bytes for the terminal (not the program) before the program starts: the old text of a restored pane.
    pub intro: Vec<u8>,
    /// Record what the program writes, and the sizes, to this file (asciinema v2; `FTERM_RECORD`).
    pub record: Option<std::path::PathBuf>,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            program: None,
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            scrollback: 10_000,
            intro: Vec::new(),
            record: None,
        }
    }
}

impl SessionOptions {
    pub fn command<I, S>(program: &str, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: Some(program.to_owned()),
            args: args.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }
}

/// Gets events from alacritty on the pty thread and passes them on.
#[derive(Clone)]
pub struct Listener {
    on_event: Arc<dyn Fn(TermEvent) + Send + Sync>,
    /// Set after the event loop starts. Used to answer the app (`PtyWrite`).
    sender: Arc<OnceLock<LoopSender>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => (self.on_event)(TermEvent::Redraw),
            Event::Title(title) => (self.on_event)(TermEvent::Title(title)),
            Event::ResetTitle => (self.on_event)(TermEvent::Title(String::new())),
            // `Exit` comes after the last output is read. `ChildExit` comes before it.
            Event::Exit => (self.on_event)(TermEvent::Exit),
            Event::PtyWrite(text) => {
                if let Some(sender) = self.sender.get() {
                    sender.send(Msg::Input(text.into_bytes().into()));
                }
            }
            Event::ChildExit(status) => tracing::debug!(?status, "child process ended"),
            Event::Bell => (self.on_event)(TermEvent::Bell),
            other => tracing::trace!(?other, "terminal event not used yet"),
        }
    }
}

pub struct Session {
    term: Arc<FairMutex<Term<Listener>>>,
    /// The pty loop. `None` for a scene (no pty).
    notifier: Mutex<Option<Notifier>>,
    size: Mutex<GridSize>,
    /// The user's selection. It is kept here, so the terminal cannot remove it.
    selection: Mutex<StickySelection>,
    /// The shell process.
    pid: Option<u32>,
    /// The program name without folder and `.exe` (for the default tab title).
    program: String,
    /// `FTERM_RECORD`: the recording of this session (the sizes are written here).
    recorder: crate::record::Shared,
}

impl Session {
    /// Starts the program. `cell` is the cell size in pixels (some apps ask for it).
    pub fn spawn(
        options: SessionOptions,
        size: GridSize,
        cell: (u16, u16),
        on_event: impl Fn(TermEvent) + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let listener = Listener {
            on_event: Arc::new(on_event),
            sender: Arc::new(OnceLock::new()),
        };
        let term = Arc::new(FairMutex::new(Term::new(
            Config {
                scrolling_history: options.scrollback,
                ..term_config()
            },
            &size,
            listener.clone(),
        )));

        if !options.intro.is_empty() {
            // Before the pty loop starts, so the program output comes after it.
            alacritty_terminal::vte::ansi::Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new()
                .advance(&mut *term.lock(), &options.intro);
        }
        let (program, args) = match options.program {
            Some(program) => (program, options.args),
            None => (default_shell(), options.args),
        };
        tracing::info!(%program, ?args, "starting session");
        let program_name = program_name(&program);
        let env: HashMap<String, String> = HashMap::from([
            ("TERM".to_owned(), "xterm-256color".to_owned()),
            ("COLORTERM".to_owned(), "truecolor".to_owned()),
            ("TERM_PROGRAM".to_owned(), "fterm".to_owned()),
        ])
        .into_iter()
        .chain(options.env)
        .collect();
        // Windows: our own ConPTY, so it takes our flags (see `conpty`).
        #[cfg(windows)]
        let pty = crate::conpty::new(
            &crate::conpty::Options {
                program,
                args,
                working_directory: options.cwd,
                env,
                escape_args: true,
                flags: crate::conpty::PSEUDOCONSOLE_RESIZE_QUIRK,
            },
            window_size(size, cell),
        )?;
        #[cfg(not(windows))]
        let pty = tty::new(
            &tty::Options {
                shell: Some(tty::Shell::new(program, args)),
                working_directory: options.cwd,
                drain_on_exit: true,
                env,
            },
            window_size(size, cell),
            0,
        )?;
        #[cfg(windows)]
        let pid = pty.child_watcher().pid().map(|pid| pid.get());
        #[cfg(unix)]
        let pid = Some(pty.child().id());
        let osc_listener = listener.clone();
        let osc_sink: crate::io_loop::OscSink = Arc::new(move |events: Vec<OscEvent>| {
            for event in events {
                (osc_listener.on_event)(TermEvent::Osc(event));
            }
        });
        let recorder: crate::record::Shared = Default::default();
        if let Some(path) = &options.record {
            match crate::record::open(path, size.columns, size.rows) {
                Ok(r) => {
                    tracing::info!(path = %path.display(), "recording the session");
                    *recorder.lock().unwrap() = Some(r);
                }
                Err(err) => tracing::warn!(path = %path.display(), "cannot record: {err}"),
            }
        }
        let event_loop = IoLoop::new(
            term.clone(),
            listener.clone(),
            pty,
            osc_sink,
            recorder.clone(),
        )?;
        let sender = event_loop.channel();
        let _ = listener.sender.set(sender.clone());
        event_loop.spawn();

        Ok(Self {
            term,
            notifier: Mutex::new(Some(Notifier(sender))),
            size: Mutex::new(size),
            selection: Mutex::new(StickySelection::default()),
            pid,
            program: program_name,
            recorder,
        })
    }

    /// Sends bytes to the program (keys, paste).
    pub fn write(&self, bytes: Vec<u8>) {
        if let Some(notifier) = &*self.notifier.lock().unwrap() {
            notifier.notify(bytes);
        }
    }

    pub fn resize(&self, size: GridSize, cell: (u16, u16)) {
        crate::record::with(&self.recorder, |r| {
            r.resize(size.columns, size.rows, std::time::Instant::now())
        });
        *self.size.lock().unwrap() = size;
        if cfg!(windows) {
            resize_like_conpty(&mut self.term.lock(), size);
        } else {
            self.term.lock().resize(size);
        }
        if let Some(notifier) = &mut *self.notifier.lock().unwrap() {
            notifier.on_resize(window_size(size, cell));
        }
    }

    /// The shell process id.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// A session with no pty and no program, for a Braille scene (Phase 11): fterm draws into it
    /// with `feed`, and keys go nowhere.
    pub fn scene(size: GridSize) -> Self {
        let listener = Listener {
            on_event: Arc::new(|_| {}),
            sender: Arc::new(OnceLock::new()),
        };
        let term = Term::new(
            Config {
                scrolling_history: 0,
                ..term_config()
            },
            &size,
            listener,
        );
        Self {
            term: Arc::new(FairMutex::new(term)),
            notifier: Mutex::new(None),
            size: Mutex::new(size),
            selection: Mutex::new(StickySelection::default()),
            pid: None,
            program: "scene".to_owned(),
            recorder: Default::default(),
        }
    }

    /// Bytes for the terminal (not for a program): they are drawn at once.
    pub fn feed(&self, bytes: &[u8]) {
        alacritty_terminal::vte::ansi::Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new()
            .advance(&mut *self.term.lock(), bytes);
    }

    /// The program name, for example `pwsh` or `cmd`.
    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn grid_size(&self) -> GridSize {
        *self.size.lock().unwrap()
    }

    /// Runs `f` with the terminal state locked, for example to draw it.
    /// Keep `f` short: the pty thread waits for the lock.
    pub fn with_term<R>(&self, f: impl FnOnce(&Term<Listener>) -> R) -> R {
        let mut term = self.term.lock();
        self.selection.lock().unwrap().sync(&mut term);
        f(&term)
    }

    /// Like `with_term`, but `f` can change the terminal (scroll, vi mode) and the selection.
    /// Change the selection only through `StickySelection::set`.
    pub fn with_term_mut<R>(
        &self,
        f: impl FnOnce(&mut Term<Listener>, &mut StickySelection) -> R,
    ) -> R {
        let mut term = self.term.lock();
        let mut selection = self.selection.lock().unwrap();
        selection.sync(&mut term);
        f(&mut term, &mut selection)
    }

    /// Text on the screen, one line per row, without spaces at the end of lines.
    pub fn screen_text(&self) -> String {
        let term = self.term.lock();
        let grid = term.grid();
        let mut text = String::new();
        for row in 0..grid.screen_lines() {
            let line = &grid[Line(row as i32)];
            let mut row_text = String::new();
            for col in 0..grid.columns() {
                let cell = &line[Column(col)];
                if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    row_text.push(cell.c);
                }
            }
            text.push_str(row_text.trim_end());
            text.push('\n');
        }
        text
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Stop the pty thread. The shell ends when its pty closes.
        if let Some(notifier) = &*self.notifier.lock().unwrap() {
            notifier.0.send(Msg::Shutdown);
        }
    }
}

/// Settings for the terminal state.
pub fn term_config() -> Config {
    Config {
        semantic_escape_chars: WORD_SEPARATORS.to_owned(),
        ..Config::default()
    }
}

/// Chars that end a word for double click. `:` `/` `.` `-` `_` `?` `=` `&` `#` `~` are not here,
/// so URLs, paths, `file:line`, and git hashes are one word.
const WORD_SEPARATORS: &str = ",│`|\"' ()[]{}<>\t";

fn window_size(size: GridSize, (cell_width, cell_height): (u16, u16)) -> WindowSize {
    WindowSize {
        num_lines: size.rows as u16,
        num_cols: size.columns as u16,
        cell_width,
        cell_height,
    }
}

/// `C:\Windows\cmd.exe` -> `cmd`, `/bin/zsh` -> `zsh`.
fn program_name(program: &str) -> String {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let lower = file.to_ascii_lowercase();
    match lower.strip_suffix(".exe") {
        Some(_) => file[..file.len() - 4].to_owned(),
        None => file.to_owned(),
    }
}

/// The default shell: pwsh or powershell on Windows, `$SHELL` on unix.
fn default_shell() -> String {
    if cfg!(windows) {
        if find_in_path("pwsh.exe") {
            "pwsh.exe".to_owned()
        } else {
            "powershell.exe".to_owned()
        }
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned())
    }
}

fn find_in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// Resizes `term` the way ConPTY resizes its own buffer. A taller alacritty grid pulls lines down
/// from the history; ConPTY has no history and adds rows at the bottom. With
/// `PSEUDOCONSOLE_RESIZE_QUIRK` ConPTY does not draw its screen again, so programs that move to an
/// absolute row (PSReadLine) would write over the wrong row. So the pulled lines go back up.
pub fn resize_like_conpty<T: EventListener>(term: &mut Term<T>, size: GridSize) {
    use alacritty_terminal::term::TermMode;
    use alacritty_terminal::vte::ansi::Handler;
    let added = size.rows.saturating_sub(term.screen_lines());
    // The alternate screen has no history, so nothing is pulled there.
    let pulled = if term.mode().contains(TermMode::ALT_SCREEN) {
        0
    } else {
        added.min(term.grid().history_size())
    };
    term.resize(size);
    if pulled > 0 {
        term.scroll_up(pulled);
        term.move_up(pulled);
        // The cursor that DECSC saved went down too.
        let saved = &mut term.grid_mut().saved_cursor.point.line;
        *saved = Line((saved.0 - pulled as i32).max(0));
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::{Column, Point, Side};
    use alacritty_terminal::selection::{Selection, SelectionType};
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;

    fn feed(term: &mut Term<VoidListener>, text: &str) {
        Processor::<StdSyncHandler>::new().advance(term, text.as_bytes());
    }

    #[test]
    fn a_taller_window_keeps_the_rows_where_conpty_has_them() {
        // ConPTY clears and draws the prompt at row 1. alacritty puts one empty line into the history
        // for that clear, and a taller grid pulls history lines down; ConPTY has no history and adds
        // rows at the bottom. Then PSReadLine draws the input with `CSI 1;26 H` over the wrong row.
        let mut term = Term::new(term_config(), &GridSize::new(80, 24), VoidListener);
        feed(&mut term, "\x1b[2J\x1b[m\x1b[HPS C:\\work\\github\\fterm> ");
        resize_like_conpty(&mut term, GridSize::new(90, 30));
        assert_eq!(term.grid().cursor.point, Point::new(Line(0), Column(25)));
        feed(&mut term, "\x1b[1;26Hcla");
        assert_eq!(term.grid()[Line(0)][Column(0)].c, 'P');
        assert_eq!(term.grid()[Line(0)][Column(25)].c, 'c');
        assert!(
            term.grid()[Line(1)][Column(25)].c == ' ',
            "no input on the next row"
        );
    }

    #[test]
    fn a_taller_window_keeps_the_saved_cursor_too() {
        let mut term = Term::new(term_config(), &GridSize::new(80, 24), VoidListener);
        feed(&mut term, "\x1b[2J\x1b[HPS> \x1b7");
        resize_like_conpty(&mut term, GridSize::new(80, 30));
        feed(&mut term, "\x1b[5;1H\x1b8");
        assert_eq!(term.grid().cursor.point, Point::new(Line(0), Column(4)));
    }

    #[test]
    fn a_taller_window_after_long_output_keeps_the_history() {
        // 40 lines in a grid of 10: 30 go to the history. A taller grid must not pull them down,
        // and they must stay in the history.
        let mut term = Term::new(term_config(), &GridSize::new(40, 10), VoidListener);
        let lines: String = (1..=40).map(|n| format!("line-{n}\r\n")).collect();
        feed(&mut term, &lines);
        let before = term.grid().cursor.point.line;
        let history = term.grid().history_size();
        resize_like_conpty(&mut term, GridSize::new(40, 16));
        assert_eq!(term.grid().cursor.point.line, before);
        assert_eq!(term.grid().history_size(), history);
        assert_eq!(
            term.grid()[Line(0)][Column(5)].c,
            '3',
            "line-32 is still the top row"
        );
        // Smaller and wider work as before.
        resize_like_conpty(&mut term, GridSize::new(60, 8));
        assert_eq!(term.screen_lines(), 8);
    }

    /// Double click on column `col` of line 0, and return the selected word.
    fn double_click(text: &str, col: usize) -> String {
        let mut term = Term::new(term_config(), &GridSize::new(60, 2), VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut term, text.as_bytes());
        let point = Point::new(Line(0), Column(col));
        term.selection = Some(Selection::new(SelectionType::Semantic, point, Side::Left));
        term.selection_to_string().unwrap()
    }

    #[test]
    fn double_click_selects_a_whole_url() {
        let text = "see https://example.com/a-b_c.d?x=1&y=2#top now";
        assert_eq!(
            double_click(text, 12),
            "https://example.com/a-b_c.d?x=1&y=2#top"
        );
    }

    #[test]
    fn double_click_selects_a_whole_path_with_line_number() {
        assert_eq!(
            double_click("error in C:\\work\\fterm\\src\\app.rs:42 here", 15),
            "C:\\work\\fterm\\src\\app.rs:42"
        );
        assert_eq!(
            double_click("at ~/code/fterm/main.rs:7", 8),
            "~/code/fterm/main.rs:7"
        );
    }

    #[test]
    fn double_click_stops_at_quotes_and_brackets() {
        assert_eq!(double_click("open(\"a/b.txt\")", 8), "a/b.txt");
        assert_eq!(double_click("commit 1a2b3c4d done", 9), "1a2b3c4d");
    }
}
