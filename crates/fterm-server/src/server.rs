//! The server: it starts the programs of its panes, keeps their Terms, and sends their output to the window that
//! shows them. One window per pane: a new attach takes the pane over.

use std::collections::HashMap;
use std::io::{self, BufReader, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use fterm_term::session::{Session, SessionOptions, Tap, TermEvent};
use interprocess::local_socket::prelude::*;

use crate::frame::{self, CONTROL, DATA, SNAPSHOT};
use crate::proto::{Meta, PaneInfo, Reply, Request, ServerMsg, Spawn};

/// The cell size that the server gives to the programs (some ask for it in pixels).
const CELL: (u16, u16) = (8, 16);

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

/// A window connection.
#[derive(Clone)]
struct Client {
    id: u64,
    writer: Writer,
}

impl Client {
    fn send(&self, msg: &ServerMsg) {
        let text = serde_json::to_vec(msg).expect("JSON");
        let mut w = self.writer.lock().unwrap();
        if let Err(err) = frame::write_frame(&mut *w, CONTROL, &text) {
            tracing::debug!(client = self.id, "cannot send: {err}");
        }
    }
}

struct Pane {
    session: Session,
    program: String,
    meta: Arc<Mutex<Meta>>,
    /// The window that shows the pane now.
    client: Arc<Mutex<Option<Client>>>,
}

#[derive(Default)]
struct Inner {
    panes: Mutex<HashMap<u64, Arc<Pane>>>,
    next_pane: AtomicU64,
    next_client: AtomicU64,
    clients: AtomicUsize,
    layout: Mutex<Option<serde_json::Value>>,
    /// When the last client or pane went (for the idle end).
    idle_since: Mutex<Option<Instant>>,
    /// `Stop` came: the server ends now.
    stopped: std::sync::atomic::AtomicBool,
}

/// A running server (the listener runs on its own thread).
#[derive(Clone)]
pub struct Server {
    inner: Arc<Inner>,
}

impl Server {
    /// Listens on the socket `name` (see `fterm_api::transport`). Only the current user can connect.
    pub fn start(name: &str) -> io::Result<Self> {
        let listener = fterm_api::transport::listen(name)?;
        let inner = Arc::new(Inner::default());
        *inner.idle_since.lock().unwrap() = Some(Instant::now());
        let server = Self { inner };
        let accept = server.clone();
        std::thread::Builder::new()
            .name("mux accept".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => accept.serve(stream),
                        Err(err) => tracing::warn!("mux accept: {err}"),
                    }
                }
            })?;
        Ok(server)
    }

    /// Runs until the server has no panes and no windows for `idle`.
    pub fn run_until_idle(&self, idle: Duration) {
        loop {
            if self.inner.stopped.load(Ordering::SeqCst) {
                tracing::info!("mux server: stopped");
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
            let since = *self.inner.idle_since.lock().unwrap();
            if self.pane_count() == 0
                && self.inner.clients.load(Ordering::SeqCst) == 0
                && since.is_some_and(|t| t.elapsed() >= idle)
            {
                tracing::info!("mux server: nothing to do, it ends");
                return;
            }
        }
    }

    pub fn pane_count(&self) -> usize {
        self.inner.panes.lock().unwrap().len()
    }

    fn serve(&self, stream: interprocess::local_socket::Stream) {
        let (recv, send) = stream.split();
        let client = Client {
            id: self.inner.next_client.fetch_add(1, Ordering::SeqCst) + 1,
            writer: Arc::new(Mutex::new(Box::new(send))),
        };
        self.inner.clients.fetch_add(1, Ordering::SeqCst);
        let server = self.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("mux client {}", client.id))
            .spawn(move || {
                let mut reader = BufReader::new(recv);
                loop {
                    match frame::read_frame(&mut reader) {
                        Ok(Some((CONTROL, body))) => match serde_json::from_slice::<Request>(&body)
                        {
                            Ok(request) => server.handle(&client, request),
                            Err(err) => tracing::warn!("mux: bad request: {err}"),
                        },
                        Ok(Some((DATA, body))) => {
                            if let Some((pane, bytes)) = frame::split_pane(&body)
                                && let Some(p) = server.pane(pane)
                            {
                                p.session.write(bytes.to_vec());
                            }
                        }
                        Ok(Some((kind, _))) => tracing::warn!("mux: unknown frame {kind}"),
                        Ok(None) => break,
                        Err(err) => {
                            tracing::debug!("mux client {}: {err}", client.id);
                            break;
                        }
                    }
                }
                server.client_gone(&client);
            });
        if let Err(err) = spawned {
            tracing::warn!("mux: cannot serve a client: {err}");
            self.inner.clients.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn pane(&self, id: u64) -> Option<Arc<Pane>> {
        self.inner.panes.lock().unwrap().get(&id).cloned()
    }

    /// The window went: its panes have no window now (their programs go on).
    fn client_gone(&self, client: &Client) {
        for pane in self.inner.panes.lock().unwrap().values() {
            let mut slot = pane.client.lock().unwrap();
            if slot.as_ref().is_some_and(|c| c.id == client.id) {
                *slot = None;
            }
        }
        if self.inner.clients.fetch_sub(1, Ordering::SeqCst) == 1 {
            *self.inner.idle_since.lock().unwrap() = Some(Instant::now());
        }
    }

    fn handle(&self, client: &Client, request: Request) {
        let id = request.id();
        let reply = match request {
            Request::Spawn { spawn, size, .. } => match self.spawn(spawn, size) {
                Ok((pane, program)) => {
                    // The window that made it shows it.
                    if let Some(p) = self.pane(pane) {
                        *p.client.lock().unwrap() = Some(client.clone());
                    }
                    Reply {
                        pane: Some(pane),
                        program: Some(program),
                        ..Reply::ok(id)
                    }
                }
                Err(err) => Reply::error(id, err.to_string()),
            },
            Request::Attach { pane, .. } => {
                let Some(p) = self.pane(pane) else {
                    return client.send(&ServerMsg::reply(Reply::error(id, "no such pane")));
                };
                // With the Term locked: the snapshot, the answer, and the new window, so every byte of output is
                // in the snapshot or comes after it.
                p.session.with_term_mut(|term, _| {
                    let snapshot = fterm_term::snapshot::snapshot(term);
                    let size = p.session.grid_size();
                    let mut slot = p.client.lock().unwrap();
                    {
                        let mut w = client.writer.lock().unwrap();
                        let _ = frame::write_pane(&mut *w, SNAPSHOT, pane, &snapshot);
                    }
                    client.send(&ServerMsg::reply(Reply {
                        pane: Some(pane),
                        program: Some(p.program.clone()),
                        meta: Some(p.meta.lock().unwrap().clone()),
                        size: Some(size),
                        ..Reply::ok(id)
                    }));
                    *slot = Some(client.clone());
                });
                return;
            }
            Request::Detach { pane, .. } => {
                if let Some(p) = self.pane(pane) {
                    let mut slot = p.client.lock().unwrap();
                    if slot.as_ref().is_some_and(|c| c.id == client.id) {
                        *slot = None;
                    }
                }
                Reply::ok(id)
            }
            Request::Resize { pane, size, .. } => {
                if let Some(p) = self.pane(pane) {
                    p.session.resize(size, CELL);
                }
                Reply::ok(id)
            }
            Request::Close { pane, .. } => {
                // The session goes, so its program ends.
                self.inner.panes.lock().unwrap().remove(&pane);
                self.maybe_idle();
                Reply::ok(id)
            }
            Request::List { .. } => Reply {
                panes: Some(self.list()),
                ..Reply::ok(id)
            },
            Request::SetLayout { layout, .. } => {
                *self.inner.layout.lock().unwrap() = Some(layout);
                Reply::ok(id)
            }
            Request::GetLayout { .. } => Reply {
                layout: self.inner.layout.lock().unwrap().clone(),
                ..Reply::ok(id)
            },
            Request::Stop { .. } => {
                let panes = self.list();
                // The sessions go, so their programs end.
                self.inner.panes.lock().unwrap().clear();
                *self.inner.layout.lock().unwrap() = None;
                self.inner.stopped.store(true, Ordering::SeqCst);
                Reply {
                    panes: Some(panes),
                    ..Reply::ok(id)
                }
            }
        };
        if id != 0 {
            client.send(&ServerMsg::reply(reply));
        }
    }

    fn list(&self) -> Vec<PaneInfo> {
        let mut panes: Vec<PaneInfo> = self
            .inner
            .panes
            .lock()
            .unwrap()
            .iter()
            .map(|(id, p)| PaneInfo {
                pane: *id,
                program: p.program.clone(),
                attached: p.client.lock().unwrap().is_some(),
                running: programs(p),
            })
            .collect();
        panes.sort_by_key(|p| p.pane);
        panes
    }

    fn maybe_idle(&self) {
        if self.pane_count() == 0 {
            *self.inner.idle_since.lock().unwrap() = Some(Instant::now());
        }
    }

    fn spawn(&self, spawn: Spawn, size: fterm_term::size::GridSize) -> io::Result<(u64, String)> {
        let id = self.inner.next_pane.fetch_add(1, Ordering::SeqCst) + 1;
        let meta = Arc::new(Mutex::new(Meta::default()));
        let client: Arc<Mutex<Option<Client>>> = Arc::new(Mutex::new(None));
        let tap_client = client.clone();
        let tap = Tap(Arc::new(move |bytes: &[u8]| {
            if let Some(c) = &*tap_client.lock().unwrap() {
                let mut w = c.writer.lock().unwrap();
                let _ = frame::write_pane(&mut *w, DATA, id, bytes);
            }
        }));
        let options = SessionOptions {
            program: spawn.program,
            args: spawn.args,
            cwd: spawn.cwd.map(std::path::PathBuf::from),
            env: spawn.env,
            scrollback: spawn.scrollback,
            tap: Some(tap),
            ..SessionOptions::default()
        };
        let event_meta = meta.clone();
        let event_client = client.clone();
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        let session = Session::spawn(options, size, CELL, move |event| match event {
            TermEvent::Osc(osc) => event_meta.lock().unwrap().note(&osc),
            TermEvent::Title(title) => event_meta.lock().unwrap().title = title,
            TermEvent::Exit => {
                if let Some(c) = &*event_client.lock().unwrap() {
                    c.send(&ServerMsg::Exit { pane: id });
                }
                // Not on this thread: the pane (and its pty loop) goes away.
                let weak = weak.clone();
                let _ = std::thread::Builder::new()
                    .name("mux pane end".into())
                    .spawn(move || {
                        if let Some(inner) = weak.upgrade() {
                            inner.panes.lock().unwrap().remove(&id);
                            if inner.panes.lock().unwrap().is_empty() {
                                *inner.idle_since.lock().unwrap() = Some(Instant::now());
                            }
                        }
                    });
            }
            TermEvent::Redraw | TermEvent::Bell => {}
        })?;
        let program = session.program().to_owned();
        tracing::info!(pane = id, %program, "mux: new pane");
        self.inner.panes.lock().unwrap().insert(
            id,
            Arc::new(Pane {
                session,
                program: program.clone(),
                meta,
                client,
            }),
        );
        Ok((id, program))
    }
}

/// The programs that run in a pane: its program when it is not a shell, else what runs in the shell.
fn programs(pane: &Pane) -> Vec<String> {
    use fterm_term::process::{display_name, is_shell, running_children};
    if !is_shell(&pane.program) {
        return vec![pane.program.clone()];
    }
    pane.session
        .pid()
        .map(running_children)
        .unwrap_or_default()
        .iter()
        .map(|n| display_name(n).to_owned())
        .collect()
}
