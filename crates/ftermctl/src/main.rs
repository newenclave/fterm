//! `ftermctl`: control a running fterm window from a shell, a script, or an agent.
//!
//! It is a console program (fterm.exe is a window program, so it has no console for output).
//! It finds the window by `FTERM_SOCKET` (set in every fterm pane), by `--window <pid>`,
//! or it takes the newest fterm window.

mod cli;
mod mcp;
mod run;
mod show;

use std::process::ExitCode;

use cli::{Cli, Command};
use run::connect;
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

Braille scenes (2x4 dots in each cell):
  scene [--right|--down] [--pane N]      open a scene pane; prints its id and its size in dots
  draw [--pane N] JSON|-                 draw commands (one or a list; `-` = read them from stdin), for example
                                         '[{\"op\":\"line\",\"x0\":0,\"y0\":0,\"x1\":20,\"y1\":10}]' (see docs/SCENE.md)

Text:
  send-text [--pane N] [--enter] TEXT    type TEXT into a pane
  run [--pane N] [--wait] [--timeout S] COMMAND
                                         type COMMAND and press Enter; with --wait, wait until it ends,
                                         print its output, and exit with its exit code
  get-text [--pane N] [--history|--last-output] [--lines N] [--styled]
                                         --styled = with colors and styles, as JSON
  screenshot [--pane N] [FILE.png]       a PNG of a pane, as you see it; prints the file
                                         (no FILE = a file in the temp folder)

Events and messages:
  wait-for [--pane N] EVENT [--pattern TEXT] [--timeout S]
                                         EVENT: command_done, agent_done, agent_waiting, message, text, scene_resized
  notify TITLE [--body TEXT] [--level info|success|warning|error|attention]
  send-message N TEXT                    put a message into the inbox of pane N
  read-messages [--pane N] [--all] [--peek]
  subscribe [EVENT...]                   print events as JSON lines (Ctrl+C to stop)

Other:
  call METHOD [JSON]                     any API call (see docs/API.md)
  mcp                                    an MCP server on stdin/stdout (see docs/MCP.md)
  guide [--skill]                        how agents use fterm (recipes and rules); --skill = as a
                                         Claude Code skill (~/.claude/skills/fterm/SKILL.md)

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
        Command::Mcp => mcp::serve(cli.window).map(|()| ExitCode::SUCCESS),
        Command::Guide { skill } => {
            if *skill {
                print!("{}", fterm_api::guide::skill_md());
            } else {
                print!("{}", fterm_api::guide::GUIDE);
            }
            Ok(ExitCode::SUCCESS)
        }
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
        Command::DrawStdin { pane } => {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .map_err(|err| format!("cannot read stdin: {err}"))?;
            // PowerShell 5.1 puts a BOM before piped text.
            let ops: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
                .map_err(|err| format!("bad JSON on stdin: {err}"))?;
            let mut params = json!({ "ops": ops });
            if let Some(pane) = pane {
                params["pane"] = json!(pane);
            }
            let mut client = connect(cli.window)?;
            let answer = client
                .call("scene_draw", params)
                .map_err(|err| err.to_string())?;
            print_answer(cli.json, "scene_draw", &answer);
            Ok(ExitCode::SUCCESS)
        }
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
    if !wait {
        let mut client = connect(cli.window)?;
        let mut params = json!({ "text": text, "enter": true });
        if let Some(pane) = pane {
            params["pane"] = json!(pane);
        }
        client
            .call("send_text", params)
            .map_err(|err| err.to_string())?;
        return Ok(ExitCode::SUCCESS);
    }
    let result = run::run_and_wait(cli.window, pane, text, timeout_ms)?;
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    } else if let Some(output) = result["output"].as_str().filter(|t| !t.is_empty()) {
        println!("{output}");
    }
    let exit = result["exit"].as_i64().unwrap_or(0);
    // Exit codes of a process are 0..=255.
    Ok(ExitCode::from(exit.clamp(0, 255) as u8))
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
        "get_text" if answer.get("lines").is_some() => println!(
            "{}",
            serde_json::to_string_pretty(answer).unwrap_or_default()
        ),
        "get_text" => println!("{}", answer["text"].as_str().unwrap_or("")),
        "spawn" => println!("{}", answer["pane"]),
        "scene_open" => println!(
            "{} ({}x{} dots)",
            answer["pane"], answer["width"], answer["height"]
        ),
        "scene_draw" => {}
        "screenshot" => println!("{}", answer["path"].as_str().unwrap_or("")),
        "send_message" => println!("message {}", answer["id"]),
        "wait_for" => println!("{}", serde_json::to_string(answer).unwrap_or_default()),
        _ if answer.as_object().is_some_and(|o| o.is_empty()) => {}
        _ => println!(
            "{}",
            serde_json::to_string_pretty(answer).unwrap_or_default()
        ),
    }
}
