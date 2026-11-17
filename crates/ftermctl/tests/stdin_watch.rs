//! The `ftermctl` of a long call of `ftermctl mcp` (a form, a Review) ends when its stdin closes: its parent ended,
//! also when it was killed. Then its connection goes, and fterm closes the dialog. Here the window is a test server.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use fterm_api::RpcError;
use fterm_api::server::{ClientId, Handler, Server};
use serde_json::{Value, json};

/// A window whose `wait` call waits until its client is gone (or 20 seconds), like a form.
#[derive(Default)]
struct Window {
    started: Mutex<bool>,
    gone: Mutex<Vec<ClientId>>,
    changed: Condvar,
}

impl Handler for Window {
    fn call(&self, client: ClientId, method: &str, _params: Value) -> Result<Value, RpcError> {
        match method {
            "hello" => Ok(json!({})),
            "wait" => {
                *self.started.lock().unwrap() = true;
                let gone = self.gone.lock().unwrap();
                let _ = self
                    .changed
                    .wait_timeout_while(gone, Duration::from_secs(20), |g| !g.contains(&client))
                    .unwrap();
                Ok(json!("done"))
            }
            other => Err(RpcError::no_method(other)),
        }
    }

    fn disconnected(&self, client: ClientId) {
        self.gone.lock().unwrap().push(client);
        self.changed.notify_all();
    }
}

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < deadline, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_long_call_ends_when_its_parent_goes() {
    let name =
        fterm_api::transport::named_socket(&format!("stdin-watch-test-{}", std::process::id()));
    let window = Arc::new(Window::default());
    let server = Server::start(&name, window.clone()).expect("the server starts");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ftermctl"))
        .args(["--json", "call", "wait", "-"])
        .env(fterm_api::discovery::SOCKET_ENV, server.name())
        .env("FTERM_EXIT_ON_EOF", "1")
        .env_remove("FTERM_PANE_ID")
        .env_remove("FTERM_CLIENT_NAME")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("ftermctl starts");
    let mut stdin = child.stdin.take().unwrap();
    // The params are one line; stdin stays open.
    stdin.write_all(b"{}\n").unwrap();
    stdin.flush().unwrap();
    wait("the call to start", || *window.started.lock().unwrap());
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        child.try_wait().unwrap().is_none(),
        "it waits while its parent is there"
    );
    // The parent goes: its end of the pipe closes.
    drop(stdin);
    wait("ftermctl to end", || child.try_wait().unwrap().is_some());
    wait("the window to see the client go", || {
        !window.gone.lock().unwrap().is_empty()
    });
}
