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
                    let (recv, send) = stream.split();
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

/// Reads the requests of one client and answers them, until the client goes away.
fn serve(
    inner: &Inner,
    handler: &dyn Handler,
    id: ClientId,
    reader: impl BufRead,
    writer: &Mutex<Box<dyn Write + Send>>,
) {
    for line in reader.lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let answer = match parse_request(&line) {
            Err(err) => Some(Response::err(Value::Null, err)),
            Ok(request) => {
                let result = match request.method.as_str() {
                    "subscribe" => subscribe(inner, id, &request.params, true),
                    "unsubscribe" => subscribe(inner, id, &request.params, false),
                    _ => handler.call(id, &request.method, request.params),
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
    if let Ok(mut clients) = inner.clients.lock() {
        clients.remove(&id);
    }
    handler.disconnected(id);
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
