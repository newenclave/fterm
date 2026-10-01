//! These tests use a real local socket (a named pipe on Windows).

use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fterm_api::client::{Client, ClientError};
use fterm_api::discovery::{Instance, find, instances, register};
use fterm_api::server::{ClientId, Handler, Server};
use fterm_api::transport::{connect, socket_name};
use fterm_api::{RpcError, protocol::Request, protocol::encode};
use serde_json::{Value, json};

/// A socket name that no other test uses.
fn unique_name() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    // A fake pid: the real one plus a big number, so names do not meet.
    socket_name(
        std::process::id()
            .wrapping_mul(100)
            .wrapping_add(1_000_000 + n),
    )
}

#[derive(Default)]
struct Echo {
    gone: Mutex<Vec<ClientId>>,
}

impl Handler for Echo {
    fn call(&self, client: ClientId, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            "echo" => Ok(params),
            "who" => Ok(json!(client)),
            "slow" => {
                std::thread::sleep(Duration::from_millis(300));
                Ok(json!("late"))
            }
            other => Err(RpcError::no_method(other)),
        }
    }

    fn disconnected(&self, client: ClientId) {
        self.gone.lock().unwrap().push(client);
    }
}

fn start() -> (Server, Arc<Echo>) {
    let handler = Arc::new(Echo::default());
    let server = Server::start(&unique_name(), handler.clone()).expect("the server starts");
    (server, handler)
}

#[test]
fn a_call_and_its_answer() {
    let (server, _) = start();
    let mut client = Client::connect(server.name()).unwrap();
    assert_eq!(
        client.call("echo", json!({"a": [1, 2]})).unwrap(),
        json!({"a": [1, 2]})
    );
    assert_eq!(
        client.call("echo", json!("two")).unwrap(),
        json!("two"),
        "many calls on one connection"
    );
}

#[test]
fn an_unknown_method_is_an_error() {
    let (server, _) = start();
    let mut client = Client::connect(server.name()).unwrap();
    match client.call("dance", Value::Null) {
        Err(ClientError::Rpc(err)) => assert_eq!(err.code, RpcError::NO_METHOD),
        other => panic!("{other:?}"),
    }
}

#[test]
fn two_clients_at_the_same_time() {
    let (server, _) = start();
    let mut slow = Client::connect(server.name()).unwrap();
    let mut fast = Client::connect(server.name()).unwrap();
    let name = server.name().to_owned();
    let waiting = std::thread::spawn(move || slow.call("slow", Value::Null).unwrap());
    std::thread::sleep(Duration::from_millis(50));
    // The slow call of the first client does not stop the second one.
    let started = std::time::Instant::now();
    assert_eq!(fast.call("echo", json!(1)).unwrap(), json!(1));
    assert!(started.elapsed() < Duration::from_millis(250));
    assert_eq!(waiting.join().unwrap(), json!("late"));
    let a = Client::connect(&name)
        .unwrap()
        .call("who", Value::Null)
        .unwrap();
    let b = Client::connect(&name)
        .unwrap()
        .call("who", Value::Null)
        .unwrap();
    assert_ne!(a, b, "each client has its own id");
}

#[test]
fn subscribed_clients_get_events() {
    let (server, _) = start();
    let mut all = Client::connect(server.name()).unwrap();
    let mut some = Client::connect(server.name()).unwrap();
    let mut none = Client::connect(server.name()).unwrap();
    all.call("subscribe", json!({"events": ["*"]})).unwrap();
    some.call("subscribe", json!({"events": ["command_done"]}))
        .unwrap();
    server.broadcast("title", json!({"pane": 1}));
    server.broadcast("command_done", json!({"pane": 2, "exit": 0}));
    let first = all.next_event().unwrap();
    assert_eq!(
        (first.method.as_str(), first.params.clone()),
        ("title", json!({"pane": 1}))
    );
    assert_eq!(all.next_event().unwrap().method, "command_done");
    let only = some.next_event().unwrap();
    assert_eq!(only.method, "command_done");
    assert_eq!(only.params, json!({"pane": 2, "exit": 0}));
    // A client without a subscription gets no events: its next answer is the answer.
    assert_eq!(none.call("echo", json!(5)).unwrap(), json!(5));
    // Events that come before an answer are kept for later.
    server.broadcast("title", json!({"pane": 3}));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(all.call("echo", json!(6)).unwrap(), json!(6));
    assert_eq!(all.next_event().unwrap().params, json!({"pane": 3}));
}

#[test]
fn a_bad_line_gets_an_error_and_the_connection_stays() {
    let (server, _) = start();
    let stream = connect(server.name()).unwrap();
    let mut reader = BufReader::new(stream);
    reader.get_mut().write_all(b"this is not json\n").unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let answer: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(answer["error"]["code"], json!(RpcError::PARSE));
    assert_eq!(answer["id"], Value::Null);
    reader
        .get_mut()
        .write_all(encode(&Request::new(9, "echo", json!("still here"))).as_bytes())
        .unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    let answer: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(answer["result"], json!("still here"));
    assert_eq!(answer["id"], json!(9));
}

#[test]
fn a_closed_client_is_noticed() {
    let (server, handler) = start();
    let mut client = Client::connect(server.name()).unwrap();
    let id = client.call("who", Value::Null).unwrap();
    drop(client);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        handler.gone.lock().unwrap().as_slice(),
        [id.as_u64().unwrap()]
    );
    assert_eq!(server.client_count(), 0);
    // The server still works.
    assert_eq!(
        Client::connect(server.name())
            .unwrap()
            .call("echo", json!(1))
            .unwrap(),
        json!(1)
    );
}

#[test]
fn windows_are_found() {
    let dir = std::env::temp_dir().join(format!("fterm-instances-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (old, _) = start();
    let (new, _) = start();
    let reg_old = register(
        &dir,
        &Instance {
            pid: 11,
            socket: old.name().into(),
            started: 100,
        },
    )
    .unwrap();
    let _reg_new = register(
        &dir,
        &Instance {
            pid: 12,
            socket: new.name().into(),
            started: 200,
        },
    )
    .unwrap();
    // A window that is gone (no server on its socket).
    let _dead = register(
        &dir,
        &Instance {
            pid: 13,
            socket: unique_name(),
            started: 300,
        },
    )
    .unwrap();
    let list = instances(&dir);
    assert_eq!(
        list.iter().map(|i| i.pid).collect::<Vec<_>>(),
        [12, 11],
        "newest first, dead ones gone"
    );
    assert!(
        !dir.join("13.json").exists(),
        "the file of a dead window is deleted"
    );
    assert_eq!(find(&dir, None, None).as_deref(), Some(new.name()));
    assert_eq!(find(&dir, None, Some(11)).as_deref(), Some(old.name()));
    assert_eq!(
        find(&dir, Some("from-env"), Some(11)).as_deref(),
        Some("from-env"),
        "FTERM_SOCKET wins"
    );
    drop(reg_old);
    assert!(
        !dir.join("11.json").exists(),
        "the file goes away with the window"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
