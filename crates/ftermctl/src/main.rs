//! `ftermctl`: control a running fterm window from a shell, a script, or an agent.
//!
//! It is a console program (fterm.exe is a window program, so it has no console for output).
//! It finds the window by `FTERM_SOCKET` (set in every fterm pane), by `--window <pid>`,
//! or it takes the newest fterm window.

use std::process::ExitCode;

use fterm_api::client::Client;
use fterm_api::discovery::{SOCKET_ENV, find, instances_dir};
use serde_json::{Value, json};

const USAGE: &str = "\
ftermctl - control fterm from a shell, a script, or an agent

Usage: ftermctl [--window PID] <command> [args]

Commands:
  call <method> [json]   send one API call and print the JSON answer
                         for example: ftermctl call list
                                      ftermctl call send_text '{\"pane\": 2, \"text\": \"ls\", \"enter\": true}'
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ftermctl: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let mut args = args.to_vec();
    let window = match args.iter().position(|a| a == "--window") {
        Some(i) => {
            let pid = args
                .get(i + 1)
                .and_then(|p| p.parse::<u32>().ok())
                .ok_or("--window needs a process id")?;
            args.drain(i..=i + 1);
            Some(pid)
        }
        None => None,
    };
    let Some(command) = args.first() else {
        print!("{USAGE}");
        return Ok(());
    };
    match command.as_str() {
        "call" => {
            let method = args.get(1).ok_or("call needs a method name")?;
            let params: Value = match args.get(2) {
                Some(text) => {
                    serde_json::from_str(text).map_err(|err| format!("bad JSON: {err}"))?
                }
                None => Value::Null,
            };
            let mut client = connect(window)?;
            let answer = client.call(method, params).map_err(|err| err.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&answer).unwrap_or_default()
            );
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}` (try `ftermctl help`)")),
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
