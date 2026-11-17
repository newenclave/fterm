//! Connect to the window, and run a command in a pane and wait for its end (for `run --wait` and MCP).

use fterm_api::client::Client;
use fterm_api::discovery::{SOCKET_ENV, find, instances_dir};
use serde_json::{Value, json};

/// The socket of the window: `--window`, else the env var of fterm, else the newest window.
pub fn find_socket(window: Option<u32>) -> Result<String, String> {
    let env_socket = std::env::var(SOCKET_ENV).ok();
    // `--window` wins over the env var.
    let env_socket = if window.is_some() { None } else { env_socket };
    find(&instances_dir(), env_socket.as_deref(), window)
        .ok_or_else(|| "no fterm window found (is fterm running?)".to_owned())
}

/// Connects to the window and says who we are (`name` is shown in the access question).
pub fn connect_as(window: Option<u32>, name: &str) -> Result<Client, String> {
    connect_socket(&find_socket(window)?, name)
}

/// Connects to the window of `socket` and says who we are.
pub fn connect_socket(socket: &str, name: &str) -> Result<Client, String> {
    let mut client =
        Client::connect(socket).map_err(|err| format!("cannot connect to {socket}: {err}"))?;
    client
        .call("hello", json!({ "name": name, "pane": own_pane() }))
        .map_err(|err| err.to_string())?;
    Ok(client)
}

pub fn connect(window: Option<u32>) -> Result<Client, String> {
    connect_as(window, &client_name(std::env::var(NAME_ENV).ok()))
}

/// The name that ftermctl gives itself (`hello`): `ftermctl`, or this env var (`ftermctl mcp` sets it for the
/// `ftermctl` that it starts for a long call, so the dialog shows the name of the agent).
pub const NAME_ENV: &str = "FTERM_CLIENT_NAME";

/// Set by `ftermctl mcp` for the `ftermctl` of a long call: the params come as one line on stdin, and stdin stays
/// open; when it closes (the parent ended in any way), this ftermctl ends too, so fterm closes the dialog.
pub const EXIT_ON_EOF_ENV: &str = "FTERM_EXIT_ON_EOF";

/// The client name from the env var: `ftermctl` when it is not there or empty.
pub fn client_name(env: Option<String>) -> String {
    env.filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "ftermctl".to_owned())
}

/// The pane where this program runs (inside fterm).
pub fn own_pane() -> Option<u64> {
    std::env::var("FTERM_PANE_ID")
        .ok()
        .and_then(|p| p.parse().ok())
}

/// The pane that a call without `pane` uses: our own pane, else the active pane.
pub fn target_pane(client: &mut Client) -> Result<u64, String> {
    if let Some(pane) = own_pane() {
        return Ok(pane);
    }
    let list = client
        .call("list", Value::Null)
        .map_err(|err| err.to_string())?;
    list["active_pane"]
        .as_u64()
        .ok_or_else(|| "no active pane".to_owned())
}

/// Types `text` into the pane, presses Enter, waits until the command ends, and reads its output.
/// The answer has `pane`, `command`, `exit`, `took_ms`, and `output`.
pub fn run_and_wait(
    window: Option<u32>,
    pane: Option<u64>,
    text: &str,
    timeout_ms: u64,
) -> Result<Value, String> {
    let mut client = connect(window)?;
    run_and_wait_with(&mut client, window, pane, text, timeout_ms)
}

/// Like `run_and_wait`, on a connection that is open already. The text goes through `client`, so the
/// user's answer to the access question for this client counts. Only the event stream is a second
/// connection (events need no access).
pub fn run_and_wait_with(
    client: &mut Client,
    window: Option<u32>,
    pane: Option<u64>,
    text: &str,
    timeout_ms: u64,
) -> Result<Value, String> {
    let target = match pane {
        Some(pane) => pane,
        None => target_pane(client)?,
    };
    // Subscribe first (on a second connection), so a fast command cannot end before we listen.
    let mut events = connect_as(window, "ftermctl events")?;
    events
        .call(
            "subscribe",
            json!({ "events": ["command_done", "pane_closed"] }),
        )
        .map_err(|err| err.to_string())?;
    client
        .call(
            "send_text",
            json!({ "pane": target, "text": text, "enter": true }),
        )
        .map_err(|err| err.to_string())?;
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        loop {
            match events.next_event() {
                Ok(event) if event.params["pane"] == json!(target) => {
                    let _ = done_tx.send(Ok(event));
                    return;
                }
                Ok(_) => {}
                Err(err) => {
                    let _ = done_tx.send(Err(err.to_string()));
                    return;
                }
            }
        }
    });
    let event = match done_rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
        Ok(event) => event?,
        Err(_) => {
            return Err(format!(
                "the command did not end in {} s",
                timeout_ms / 1000
            ));
        }
    };
    if event.method == "pane_closed" {
        return Err("the pane closed".to_owned());
    }
    let output = client
        .call(
            "get_text",
            json!({ "pane": target, "what": "last_output", "lines": 10_000 }),
        )
        .map_err(|err| err.to_string())?;
    Ok(json!({
        "pane": target,
        "command": event.params["command"],
        "exit": event.params["exit"],
        "took_ms": event.params["took_ms"],
        "output": output["text"],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_name() {
        assert_eq!(client_name(None), "ftermctl");
        assert_eq!(client_name(Some(String::new())), "ftermctl", "empty");
        assert_eq!(client_name(Some("claude-code".into())), "claude-code");
    }
}
