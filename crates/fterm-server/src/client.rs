//! The window side: one connection to the mux server for all its mux panes.

use std::collections::HashMap;
use std::io::{self, BufReader, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fterm_term::session::RemoteLink;
use fterm_term::size::GridSize;
use interprocess::local_socket::prelude::*;

use crate::frame::{self, CONTROL, DATA, SNAPSHOT};
use crate::proto::{Meta, PaneInfo, Reply, Request, ServerMsg, Spawn};

/// How long a request waits for its answer.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Gets the output bytes of a pane.
pub type OutputFn = Box<dyn Fn(&[u8]) + Send + Sync>;
/// An answer and the snapshot that came before it.
type Answer = (Reply, Option<Vec<u8>>);

/// What a pane does with its output and its end (set with `MuxClient::set_handlers`).
pub struct Handlers {
    pub output: OutputFn,
    pub exit: Box<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
struct Panes {
    handlers: HashMap<u64, Handlers>,
    /// Output of a pane that has no handlers yet (right after spawn or attach).
    early: HashMap<u64, Vec<u8>>,
    /// The snapshot of an attach, until its answer comes.
    snapshots: HashMap<u64, Vec<u8>>,
}

pub struct MuxClient {
    writer: Mutex<Box<dyn Write + Send>>,
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, mpsc::Sender<Answer>>>,
    panes: Mutex<Panes>,
}

impl MuxClient {
    /// Connects to the server at `name`. `gone` runs when the connection ends (the server stopped).
    pub fn connect(name: &str, gone: impl Fn() + Send + 'static) -> io::Result<Arc<Self>> {
        let (recv, send) = fterm_api::transport::connect(name)?.split();
        let client = Arc::new(Self {
            writer: Mutex::new(Box::new(send)),
            next: AtomicU64::new(1),
            waiting: Mutex::new(HashMap::new()),
            panes: Mutex::new(Panes::default()),
        });
        let reader = Arc::downgrade(&client);
        std::thread::Builder::new()
            .name("mux reader".into())
            .spawn(move || {
                let mut r = BufReader::new(recv);
                while let Ok(Some((kind, body))) = frame::read_frame(&mut r) {
                    let Some(client) = reader.upgrade() else {
                        return;
                    };
                    client.dispatch(kind, &body);
                }
                if let Some(client) = reader.upgrade() {
                    // All panes of the server end for this window.
                    let panes = std::mem::take(&mut client.panes.lock().unwrap().handlers);
                    for h in panes.values() {
                        (h.exit)();
                    }
                }
                gone();
            })?;
        Ok(client)
    }

    fn dispatch(&self, kind: u8, body: &[u8]) {
        match kind {
            DATA => {
                if let Some((pane, bytes)) = frame::split_pane(body) {
                    let mut panes = self.panes.lock().unwrap();
                    match panes.handlers.get(&pane) {
                        Some(h) => (h.output)(bytes),
                        None => panes
                            .early
                            .entry(pane)
                            .or_default()
                            .extend_from_slice(bytes),
                    }
                }
            }
            SNAPSHOT => {
                if let Some((pane, bytes)) = frame::split_pane(body) {
                    self.panes
                        .lock()
                        .unwrap()
                        .snapshots
                        .insert(pane, bytes.to_vec());
                }
            }
            CONTROL => match serde_json::from_slice::<ServerMsg>(body) {
                Ok(ServerMsg::Reply(reply)) => {
                    let snapshot = reply
                        .pane
                        .and_then(|p| self.panes.lock().unwrap().snapshots.remove(&p));
                    if let Some(tx) = self.waiting.lock().unwrap().remove(&reply.id) {
                        let _ = tx.send((*reply, snapshot));
                    }
                }
                Ok(ServerMsg::Exit { pane }) => {
                    let h = self.panes.lock().unwrap().handlers.remove(&pane);
                    if let Some(h) = h {
                        (h.exit)();
                    }
                }
                Err(err) => tracing::warn!("mux: bad message: {err}"),
            },
            other => tracing::warn!("mux: unknown frame {other}"),
        }
    }

    fn send(&self, request: &Request) -> io::Result<()> {
        let text = serde_json::to_vec(request).map_err(io::Error::other)?;
        frame::write_frame(&mut *self.writer.lock().unwrap(), CONTROL, &text)
    }

    /// Sends a request and waits for its answer.
    fn call(&self, make: impl FnOnce(u64) -> Request) -> Result<Answer, String> {
        self.call_within(TIMEOUT, make)
    }

    fn call_within(
        &self,
        timeout: Duration,
        make: impl FnOnce(u64) -> Request,
    ) -> Result<Answer, String> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.waiting.lock().unwrap().insert(id, tx);
        if let Err(err) = self.send(&make(id)) {
            self.waiting.lock().unwrap().remove(&id);
            return Err(format!("the mux server is not there: {err}"));
        }
        let (reply, snapshot) = rx
            .recv_timeout(timeout)
            .map_err(|_| "no answer from the mux server".to_owned())?;
        match reply.error {
            Some(err) => Err(err),
            None => Ok((reply, snapshot)),
        }
    }

    /// Starts a program in a new pane: its id and its program name.
    pub fn spawn(&self, spawn: Spawn, size: GridSize) -> Result<(u64, String), String> {
        let (reply, _) = self.call(|id| Request::Spawn { id, spawn, size })?;
        let pane = reply.pane.ok_or("no pane in the answer")?;
        Ok((pane, reply.program.unwrap_or_default()))
    }

    /// Takes a pane: its snapshot, what it knows, its program, and the size of its Term on the server.
    pub fn attach(&self, pane: u64) -> Result<(Vec<u8>, Meta, String, GridSize), String> {
        let (reply, snapshot) = self.call(|id| Request::Attach { id, pane })?;
        Ok((
            snapshot.unwrap_or_default(),
            reply.meta.unwrap_or_default(),
            reply.program.unwrap_or_default(),
            reply.size.ok_or("no size in the answer")?,
        ))
    }

    /// From now on the output of `pane` goes to `handlers` (first the output that came before).
    pub fn set_handlers(&self, pane: u64, handlers: Handlers) {
        let mut panes = self.panes.lock().unwrap();
        if let Some(early) = panes.early.remove(&pane) {
            (handlers.output)(&early);
        }
        panes.handlers.insert(pane, handlers);
    }

    /// The window lets the pane go; its program goes on.
    pub fn detach(&self, pane: u64) {
        self.panes.lock().unwrap().handlers.remove(&pane);
        let _ = self.send(&Request::Detach { id: 0, pane });
    }

    /// Ends the program of the pane.
    pub fn close(&self, pane: u64) {
        self.panes.lock().unwrap().handlers.remove(&pane);
        let _ = self.send(&Request::Close { id: 0, pane });
    }

    pub fn list(&self) -> Result<Vec<PaneInfo>, String> {
        let (reply, _) = self.call(|id| Request::List { id })?;
        Ok(reply.panes.unwrap_or_default())
    }

    /// Ends all panes of the server and the server. Gives how many panes ended. A server from before `Stop` gets
    /// a `Close` for each pane instead (it ends by itself a minute later).
    pub fn stop(&self) -> Result<usize, String> {
        match self.call_within(Duration::from_secs(3), |id| Request::Stop { id }) {
            Ok((reply, _)) => Ok(reply.panes.map_or(0, |p| p.len())),
            Err(_) => {
                let panes = self.list()?;
                for p in &panes {
                    self.close(p.pane);
                }
                // The answer comes after the closes.
                self.list()?;
                Ok(panes.len())
            }
        }
    }

    pub fn set_layout(&self, layout: serde_json::Value) {
        let _ = self.send(&Request::SetLayout { id: 0, layout });
    }

    pub fn layout(&self) -> Result<Option<serde_json::Value>, String> {
        let (reply, _) = self.call(|id| Request::GetLayout { id })?;
        Ok(reply.layout)
    }

    /// Keys for the program of `pane`.
    pub fn write(&self, pane: u64, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        let _ = frame::write_pane(&mut *w, DATA, pane, bytes);
    }

    pub fn resize(&self, pane: u64, size: GridSize) {
        let _ = self.send(&Request::Resize { id: 0, pane, size });
    }

    /// The link of a remote session of `pane` (see `fterm_term::session::Session::remote`).
    pub fn link(self: &Arc<Self>, pane: u64) -> Arc<dyn RemoteLink> {
        Arc::new(PaneLink {
            client: self.clone(),
            pane,
        })
    }
}

struct PaneLink {
    client: Arc<MuxClient>,
    pane: u64,
}

impl RemoteLink for PaneLink {
    fn write(&self, bytes: Vec<u8>) {
        self.client.write(self.pane, &bytes);
    }

    fn resize(&self, size: GridSize) {
        self.client.resize(self.pane, size);
    }
}
