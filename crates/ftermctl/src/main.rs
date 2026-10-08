//! `ftermctl`: control a running fterm window from a shell, a script, or an agent.
//!
//! It is a console program (fterm.exe is a window program, so it has no console for output).
//! It finds the window by `FTERM_SOCKET` (set in every fterm pane), by `--window <pid>`,
//! or it takes the newest fterm window.

mod cli;
mod mcp;
mod review;
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
  review FILE|- [--title T] [--timeout S]
                                         show a plan (markdown) in a Review tab and wait: prints the
                                         answer; exit 0 approved, 1 changes, 2 cancelled
  review --hook                          the same as a Claude Code hook for ExitPlanMode (docs/REVIEW.md)
  theme [NAME | --file x.json]           no NAME: list the themes (* = in use); NAME: use a theme
                                         until fterm closes (see docs/THEMES.md)
  tab-color [--pane N] \"#rrggbb\"|none    a color line at the top of the tab of a pane
                                         (quote the color: # starts a comment in shells)
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

The AI panel:
  ai read [--last N]                     the chat: questions and answers
  ai ask [--pane N] [--wait] [--timeout S] TEXT
                                         ask a question (--pane N: with the last command and output of
                                         pane N); with --wait, print the answer. Needs
                                         ai = { api_access = true } in fterm.lua
  ai input TEXT | ai stop | ai clear     type into the input (not sent), stop the answer, a new chat

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
            if method == "ai_ask"
                && let Some(error) = answer.get("error").and_then(Value::as_str)
            {
                if !cli.json {
                    eprintln!("ftermctl: the AI answer failed: {error}");
                }
                return Ok(ExitCode::FAILURE);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Run {
            pane,
            text,
            wait,
            timeout_ms,
        } => run_command(cli, *pane, text, *wait, *timeout_ms),
        Command::Review {
            source,
            title,
            timeout_ms,
            hook,
        } => run_review(cli, source.as_deref(), title.as_deref(), *timeout_ms, *hook),
        Command::ThemeFile { path } => {
            let text = std::fs::read_to_string(path)
                .map_err(|err| format!("cannot read {path}: {err}"))?;
            let theme: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
                .map_err(|err| format!("{path}: bad JSON: {err}"))?;
            let mut client = connect(cli.window)?;
            let answer = client
                .call("set_theme", json!({ "theme": theme }))
                .map_err(|err| err.to_string())?;
            print_answer(cli.json, "set_theme", &answer);
            Ok(ExitCode::SUCCESS)
        }
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

/// `review`: shows a plan in a Review tab and waits for the user. With `--hook` it is a Claude Code
/// `PreToolUse` hook for `ExitPlanMode`: the hook input comes on stdin, and the answer for Claude goes to
/// stdout. Out of fterm, or with no answer, the hook says nothing, so Claude shows its own dialog.
fn run_review(
    cli: &Cli,
    source: Option<&str>,
    title: Option<&str>,
    timeout_ms: Option<u64>,
    hook: bool,
) -> Result<ExitCode, String> {
    let read_stdin = || {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .map_err(|err| format!("cannot read stdin: {err}"))?;
        Ok::<_, String>(text.trim_start_matches('\u{feff}').to_owned())
    };
    let hook_input: Option<Value> = if hook {
        let text = read_stdin()?;
        match serde_json::from_str(&text) {
            Ok(input) => Some(input),
            // Not a hook input: no answer, Claude goes on as usual.
            Err(_) => return Ok(ExitCode::SUCCESS),
        }
    } else {
        None
    };
    let plan = match (&hook_input, source) {
        (Some(input), _) => match review::hook_plan(input) {
            Some(plan) => plan,
            None => return Ok(ExitCode::SUCCESS),
        },
        (None, Some("-")) => read_stdin()?,
        (None, Some(path)) => {
            std::fs::read_to_string(path).map_err(|err| format!("cannot read {path}: {err}"))?
        }
        (None, None) => return Err("review needs a file".to_owned()),
    };
    let params = review::params(&plan, title, timeout_ms, hook);
    if hook {
        // A hook outside fterm (or with no window) gives no answer.
        if std::env::var_os("FTERM_SOCKET").is_none() && cli.window.is_none() {
            return Ok(ExitCode::SUCCESS);
        }
        let Ok(mut client) = connect(cli.window) else {
            return Ok(ExitCode::SUCCESS);
        };
        let Ok(result) = client.call("review", params) else {
            return Ok(ExitCode::SUCCESS);
        };
        if let Some(answer) = hook_input
            .as_ref()
            .and_then(|input| review::hook_answer(input, &result))
        {
            println!("{answer}");
        }
        return Ok(ExitCode::SUCCESS);
    }
    let mut client = connect(cli.window)?;
    let result = client
        .call("review", params)
        .map_err(|err| err.to_string())?;
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    } else {
        println!("{}", result["feedback"].as_str().unwrap_or_default());
    }
    Ok(ExitCode::from(review::exit_code(&result)))
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
        "ai_read" => print!("{}", show::chat(answer)),
        // With `wait`: the answer (a failed one: only the error, on stderr); without: the request id.
        "ai_ask" if answer.get("error").is_some() => {}
        "ai_ask" if answer.get("text").is_some() => {
            println!("{}", answer["text"].as_str().unwrap_or(""))
        }
        "ai_ask" => println!("{}", answer["id"]),
        "ai_stop" => {}
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
        "themes" => print!("{}", show::themes(answer)),
        "set_theme" => println!("{}", answer["name"].as_str().unwrap_or("")),
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
