//! The server: a listener thread and one thread per client.
//!
//! The server itself knows `subscribe` and `unsubscribe`. All other methods go to the [`Handler`]
//! (on the thread of the client, so a slow method like `wait_for` does not stop other clients).

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use interprocess::local_socket::prelude::*;
use serde_json::{Value, json};

use crate::protocol::{Notification, Response, RpcError, encode, parse_request};

pub type ClientId = u64;

pub trait Handler: Send + Sync + 'static {
    /// One request of a client. It can block (the client waits for its answer).
    fn call(&self, client: ClientId, method: &str, params: Value) -> Result<Value, RpcError>;

    /// The client closed its connection.
    fn disconnected(&self, client: ClientId) {
        let _ = client;
    }
}

/// A running server. Clone it to send events from any thread.
#[derive(Clone)]
pub struct Server {
    inner: Arc<Inner>,
}

struct Inner {
    name: String,
    clients: Mutex<HashMap<ClientId, Client>>,
    next_id: AtomicU64,
}

struct Client {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// The events this client wants. `*` = all.
    events: HashSet<String>,
}

impl Server {
    /// Listens on `name` (see [`crate::transport::socket_name`]) and answers clients with `handler`.
    pub fn start(name: &str, handler: Arc<dyn Handler>) -> io::Result<Self> {
        let listener = crate::transport::listen(name)?;
        let inner = Arc::new(Inner {
            name: name.to_owned(),
            clients: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        });
        let server = Self {
            inner: inner.clone(),
        };
        std::thread::Builder::new()
            .name("fterm-api".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else {
                        continue;
                    };
                    let id = inner.next_id.fetch_add(1, Ordering::SeqCst);
                    let (recv, send) = crate::transport::halves(stream);
                    let writer: Arc<Mutex<Box<dyn Write + Send>>> =
                        Arc::new(Mutex::new(Box::new(send)));
                    if let Ok(mut clients) = inner.clients.lock() {
                        clients.insert(
                            id,
                            Client {
                                writer: writer.clone(),
                                events: HashSet::new(),
                            },
                        );
                    }
                    let inner = inner.clone();
                    let handler = handler.clone();
                    let _ = std::thread::Builder::new()
                        .name(format!("fterm-api-{id}"))
                        .spawn(move || serve(&inner, &*handler, id, BufReader::new(recv), &writer));
                }
            })?;
        Ok(server)
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Sends an event to the clients that subscribed to it.
    pub fn broadcast(&self, event: &str, params: Value) {
        let line = encode(&Notification::new(event, params));
        let writers: Vec<_> = match self.inner.clients.lock() {
            Ok(clients) => clients
                .values()
                .filter(|c| c.events.contains("*") || c.events.contains(event))
                .map(|c| c.writer.clone())
                .collect(),
            Err(_) => return,
        };
        // Write without the lock of the client list: a slow client does not stop the others.
        for writer in writers {
            let _ = write_line(&writer, &line);
        }
    }

    /// How many clients are connected now.
    pub fn client_count(&self) -> usize {
        self.inner.clients.lock().map_or(0, |c| c.len())
    }
}

/// Where the connection of a client is: a call runs now, the handler heard that the client went away.
#[derive(Default)]
struct Line {
    calling: bool,
    told: bool,
}

/// How many lines of a client may wait while one of its calls runs (then its writes wait, as before).
const QUEUED_LINES: usize = 16;

/// Reads the requests of one client and answers them, until the client goes away.
///
/// A call may wait long (a Review, a form). The lines are read on their own thread, so when the client goes away
/// while its call waits, the handler hears it at once (the waiting call can then end). A call that a client sent
/// just before it went away (before the call started here) still runs, and the handler hears "gone" after it: a
/// client that sends a long call and goes away at once without waiting for the answer is not seen early. The end
/// of the reading (also a client that only closes its writing side, or a line that is not UTF-8) counts as "gone".
fn serve(
    inner: &Inner,
    handler: &dyn Handler,
    id: ClientId,
    reader: impl BufRead + Send,
    writer: &Mutex<Box<dyn Write + Send>>,
) {
    let line_state = Mutex::new(Line::default());
    let tell = || {
        let first = line_state
            .lock()
            .map(|mut s| !std::mem::replace(&mut s.told, true))
            .unwrap_or(false);
        if first {
            handler.disconnected(id);
        }
    };
    std::thread::scope(|scope| {
        let (lines, incoming) = std::sync::mpsc::sync_channel::<String>(QUEUED_LINES);
        let (state, tell) = (&line_state, &tell);
        scope.spawn(move || {
            for line in reader.lines() {
                let Ok(line) = line else {
                    break;
                };
                if lines.send(line).is_err() {
                    return;
                }
            }
            if state.lock().is_ok_and(|s| s.calling) {
                tell();
            }
        });
        for line in incoming {
            if line.trim().is_empty() {
                continue;
            }
            let answer = match parse_request(&line) {
                Err(err) => Some(Response::err(Value::Null, err)),
                Ok(request) => {
                    let result = match request.method.as_str() {
                        "subscribe" => subscribe(inner, id, &request.params, true),
                        "unsubscribe" => subscribe(inner, id, &request.params, false),
                        _ => {
                            if let Ok(mut s) = state.lock() {
                                s.calling = true;
                            }
                            let result = handler.call(id, &request.method, request.params);
                            if let Ok(mut s) = state.lock() {
                                s.calling = false;
                            }
                            result
                        }
                    };
                    request.id.map(|rid| match result {
                        Ok(value) => Response::ok(rid, value),
                        Err(err) => Response::err(rid, err),
                    })
                }
            };
            if let Some(answer) = answer
                && write_line(writer, &encode(&answer)).is_err()
            {
                break;
            }
        }
    });
    if let Ok(mut clients) = inner.clients.lock() {
        clients.remove(&id);
    }
    tell();
}

/// `subscribe { events: ["command_done", ...] }` (`"*"` = all events) and `unsubscribe` (the same params).
fn subscribe(inner: &Inner, id: ClientId, params: &Value, on: bool) -> Result<Value, RpcError> {
    let names: Vec<String> = params
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| RpcError::invalid_params("`events` must be a list of event names"))?
        .iter()
        .filter_map(|e| e.as_str().map(str::to_owned))
        .collect();
    let mut clients = inner
        .clients
        .lock()
        .map_err(|_| RpcError::new(RpcError::INTERNAL, "the client list is broken"))?;
    let client = clients
        .get_mut(&id)
        .ok_or_else(|| RpcError::new(RpcError::INTERNAL, "no such client"))?;
    for name in names {
        if on {
            client.events.insert(name);
        } else {
            client.events.remove(&name);
        }
    }
    let mut now: Vec<&String> = client.events.iter().collect();
    now.sort();
    Ok(json!({ "events": now }))
}

fn write_line(writer: &Mutex<Box<dyn Write + Send>>, line: &str) -> io::Result<()> {
    let mut writer = writer
        .lock()
        .map_err(|_| io::Error::other("the writer is broken"))?;
    writer.write_all(line.as_bytes())?;
    writer.flush()
}
