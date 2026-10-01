//! `ftermctl`: control a running fterm window from a shell, a script, or an agent.
//!
//! It is a console program (fterm.exe is a window program, so it has no console for output).
//! It finds the window by `FTERM_SOCKET` (set in every fterm pane), by `--window <pid>`,
//! or it takes the newest fterm window.

mod cli;
mod show;

use std::process::ExitCode;

use cli::{Cli, Command};
use fterm_api::client::Client;
use fterm_api::discovery::{SOCKET_ENV, find, instances_dir};
use serde_json::{Value, json};

const USAGE: &str = "\
ftermctl - control fterm from a shell, a script, or an agent

Usage: ftermctl [--window PID] [--json] <command> [args]

Panes and tabs:
  list                                   all tabs and panes
  spawn [--right|--down] [--profile P] [--cwd DIR] [--pane N]
                                         open a tab (or a split next to pane N); prints the new pane id
  focus N | close N [--force] | zoom [N] | title [--pane N] TEXT
  panel [events|agents]                  show a panel of the dock (no name = close the dock)

Text:
  send-text [--pane N] [--enter] TEXT    type TEXT into a pane
  run [--pane N] [--wait] [--timeout S] COMMAND
                                         type COMMAND and press Enter; with --wait, wait until it ends,
                                         print its output, and exit with its exit code
  get-text [--pane N] [--history|--last-output] [--lines N]

Events and messages:
  wait-for [--pane N] EVENT [--pattern TEXT] [--timeout S]
                                         EVENT: command_done, agent_done, agent_waiting, message, text
  notify TITLE [--body TEXT] [--level info|success|warning|error|attention]
  send-message N TEXT                    put a message into the inbox of pane N
  read-messages [--pane N] [--all] [--peek]
  subscribe [EVENT...]                   print events as JSON lines (Ctrl+C to stop)

Other:
  call METHOD [JSON]                     any API call (see docs/API.md)
  mcp                                    an MCP server on stdin/stdout (see docs/MCP.md)

No --pane: your own pane (inside fterm), else the active pane.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match cli::parse(&args) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("ftermctl: {err}");
            return ExitCode::from(2);
        }
    };
    match run(&cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("ftermctl: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode, String> {
    match &cli.command {
        Command::Help => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        Command::Mcp => Err("the MCP server comes in the next step".to_owned()),
        Command::Call { method, params } => {
            let mut client = connect(cli.window)?;
            let answer = client
                .call(method, params.clone())
                .map_err(|err| err.to_string())?;
            print_answer(cli.json, method, &answer);
            Ok(ExitCode::SUCCESS)
        }
        Command::Run {
            pane,
            text,
            wait,
            timeout_ms,
        } => run_command(cli, *pane, text, *wait, *timeout_ms),
        Command::Subscribe { events } => {
            let mut client = connect(cli.window)?;
            client
                .call("subscribe", json!({ "events": events }))
                .map_err(|err| err.to_string())?;
            loop {
                let event = client.next_event().map_err(|err| err.to_string())?;
                let line = json!({ "event": event.method, "params": event.params });
                println!("{}", serde_json::to_string(&line).unwrap_or_default());
            }
        }
    }
}

/// `run`: type the command; with `--wait`, wait for its end, print the output, and give its exit code.
fn run_command(
    cli: &Cli,
    pane: Option<u64>,
    text: &str,
    wait: bool,
    timeout_ms: u64,
) -> Result<ExitCode, String> {
    let mut client = connect(cli.window)?;
    let mut params = json!({ "text": text, "enter": true });
    if let Some(pane) = pane {
        params["pane"] = json!(pane);
    }
    if !wait {
        client
            .call("send_text", params)
            .map_err(|err| err.to_string())?;
        return Ok(ExitCode::SUCCESS);
    }
    // Which pane: ask the window, so the event below can be matched.
    let target = match pane {
        Some(pane) => pane,
        None => target_pane(&mut client)?,
    };
    params["pane"] = json!(target);
    // Subscribe first (on a second connection), so a fast command cannot end before we listen.
    let mut events = connect(cli.window)?;
    events
        .call(
            "subscribe",
            json!({ "events": ["command_done", "pane_closed"] }),
        )
        .map_err(|err| err.to_string())?;
    client
        .call("send_text", params)
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
    let exit = event.params["exit"].as_i64().unwrap_or(0);
    if cli.json {
        let answer = json!({
            "pane": target,
            "command": event.params["command"],
            "exit": exit,
            "took_ms": event.params["took_ms"],
            "output": output["text"],
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&answer).unwrap_or_default()
        );
    } else if let Some(text) = output["text"].as_str().filter(|t| !t.is_empty()) {
        println!("{text}");
    }
    // Exit codes of a process are 0..=255.
    Ok(ExitCode::from(exit.clamp(0, 255) as u8))
}

/// The pane that a call without `pane` uses: our own pane, else the active pane.
fn target_pane(client: &mut Client) -> Result<u64, String> {
    if let Some(pane) = std::env::var("FTERM_PANE_ID")
        .ok()
        .and_then(|p| p.parse().ok())
    {
        return Ok(pane);
    }
    let list = client
        .call("list", Value::Null)
        .map_err(|err| err.to_string())?;
    list["active_pane"]
        .as_u64()
        .ok_or_else(|| "no active pane".to_owned())
}

fn print_answer(as_json: bool, method: &str, answer: &Value) {
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(answer).unwrap_or_default()
        );
        return;
    }
    match method {
        "list" => print!("{}", show::list(answer)),
        "read_messages" => print!("{}", show::messages(answer)),
        "get_text" => println!("{}", answer["text"].as_str().unwrap_or("")),
        "spawn" => println!("{}", answer["pane"]),
        "send_message" => println!("message {}", answer["id"]),
        "wait_for" => println!("{}", serde_json::to_string(answer).unwrap_or_default()),
        _ if answer.as_object().is_some_and(|o| o.is_empty()) => {}
        _ => println!(
            "{}",
            serde_json::to_string_pretty(answer).unwrap_or_default()
        ),
    }
}

/// Connects to the window and says who we are.
fn connect(window: Option<u32>) -> Result<Client, String> {
    let env_socket = std::env::var(SOCKET_ENV).ok();
    // `--window` wins over the env var.
    let env_socket = if window.is_some() { None } else { env_socket };
    let socket = find(&instances_dir(), env_socket.as_deref(), window)
        .ok_or("no fterm window found (is fterm running?)")?;
    let mut client =
        Client::connect(&socket).map_err(|err| format!("cannot connect to {socket}: {err}"))?;
    let pane: Option<u64> = std::env::var("FTERM_PANE_ID")
        .ok()
        .and_then(|p| p.parse().ok());
    client
        .call("hello", json!({ "name": "ftermctl", "pane": pane }))
        .map_err(|err| err.to_string())?;
    Ok(client)
}
