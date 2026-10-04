//! The command line of `ftermctl`: the words become one API call (or a few).

use serde_json::{Value, json};

/// What to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Help,
    /// One API call; `wait_and_report` = `run --wait` (wait for the command, print its output, exit with its code).
    Call {
        method: String,
        params: Value,
    },
    Run {
        pane: Option<u64>,
        text: String,
        wait: bool,
        timeout_ms: u64,
    },
    Subscribe {
        events: Vec<String>,
    },
    /// The guide for agents (assets/agents/GUIDE.md); `skill` = as a Claude Code skill file.
    Guide {
        skill: bool,
    },
    /// `draw -`: the drawing commands come on stdin (a big batch does not fit on a command line).
    DrawStdin {
        pane: Option<u64>,
    },
    Mcp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cli {
    pub window: Option<u32>,
    /// Print the JSON answer, not text for people.
    pub json: bool,
    pub command: Command,
}

/// Reads the arguments (without the program name).
pub fn parse(args: &[String]) -> Result<Cli, String> {
    let mut window = None;
    let mut json_out = false;
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        match arg.as_str() {
            "--window" => {
                let pid = args
                    .get(i + 1)
                    .and_then(|p| p.parse::<u32>().ok())
                    .ok_or("--window needs a process id")?;
                window = Some(pid);
                i += 2;
            }
            "--json" => {
                json_out = true;
                i += 1;
            }
            _ => break,
        }
    }
    let command = match args.get(i) {
        None => Command::Help,
        Some(word) => command(word, &args[i + 1..])?,
    };
    Ok(Cli {
        window,
        json: json_out,
        command,
    })
}

/// The flags and the other words of a command. `--` ends the flags.
struct Words {
    values: Vec<(String, String)>,
    flags: Vec<String>,
    words: Vec<String>,
}

impl Words {
    fn read(args: &[String], with_value: &[&str], without: &[&str]) -> Result<Self, String> {
        let mut out = Words {
            values: Vec::new(),
            flags: Vec::new(),
            words: Vec::new(),
        };
        let mut i = 0;
        let mut flags_end = false;
        while let Some(arg) = args.get(i) {
            i += 1;
            if flags_end || !arg.starts_with("--") {
                out.words.push(arg.clone());
                continue;
            }
            let name = &arg[2..];
            if name.is_empty() {
                flags_end = true;
            } else if with_value.contains(&name) {
                let value = args.get(i).ok_or(format!("--{name} needs a value"))?;
                out.values.push((name.to_owned(), value.clone()));
                i += 1;
            } else if without.contains(&name) {
                out.flags.push(name.to_owned());
            } else {
                return Err(format!("unknown flag --{name}"));
            }
        }
        Ok(out)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    fn number(&self, name: &str) -> Result<Option<u64>, String> {
        self.value(name)
            .map(|v| {
                v.parse::<u64>()
                    .map_err(|_| format!("--{name} needs a number, got `{v}`"))
            })
            .transpose()
    }

    fn text(&self) -> String {
        self.words.join(" ")
    }

    /// Adds `pane` from `--pane` to the params.
    fn pane_into(&self, params: &mut Value) -> Result<(), String> {
        if let Some(pane) = self.number("pane")? {
            params["pane"] = json!(pane);
        }
        Ok(())
    }
}

fn pane_word(word: Option<&String>, what: &str) -> Result<u64, String> {
    let word = word.ok_or(format!("{what} needs a pane id"))?;
    word.parse()
        .map_err(|_| format!("{what} needs a pane id (a number), got `{word}`"))
}

fn call(method: &str, params: Value) -> Command {
    Command::Call {
        method: method.to_owned(),
        params,
    }
}

/// `ai read|ask|input|stop|clear`: the AI panel.
fn ai_command(args: &[String]) -> Result<Command, String> {
    let rest = args.get(1..).unwrap_or_default();
    let read = |with_value: &[&str], without: &[&str]| Words::read(rest, with_value, without);
    Ok(match args.first().map(String::as_str) {
        Some("read") => {
            let w = read(&["last"], &[])?;
            let mut params = json!({});
            if let Some(last) = w.number("last")? {
                params["last"] = json!(last);
            }
            call("ai_read", params)
        }
        Some("ask") => {
            let w = read(&["pane", "timeout"], &["wait"])?;
            let text = w.text();
            if text.trim().is_empty() {
                return Err("ai ask needs a question".to_owned());
            }
            let mut params = json!({ "text": text });
            if w.flag("wait") {
                params["wait"] = json!(true);
            }
            if let Some(secs) = w.number("timeout")? {
                params["timeout_ms"] = json!(secs * 1000);
            }
            w.pane_into(&mut params)?;
            call("ai_ask", params)
        }
        Some("input") => call("ai_input", json!({ "text": read(&[], &[])?.text() })),
        Some("stop") => call("ai_stop", json!({})),
        Some("clear") => call("ai_clear", json!({})),
        Some(other) => {
            return Err(format!(
                "unknown ai command `{other}` (read, ask, input, stop, clear)"
            ));
        }
        None => return Err("ai needs a command: read, ask, input, stop, clear".to_owned()),
    })
}

fn command(word: &str, args: &[String]) -> Result<Command, String> {
    let read = |with_value: &[&str], without: &[&str]| Words::read(args, with_value, without);
    Ok(match word {
        "help" | "--help" | "-h" => Command::Help,
        "mcp" => Command::Mcp,
        "guide" => Command::Guide {
            skill: read(&[], &["skill"])?.flag("skill"),
        },
        "list" => call("list", json!({})),
        "focus" => call(
            "focus",
            json!({ "pane": pane_word(args.first(), "focus")? }),
        ),
        "close" => {
            let w = read(&[], &["force"])?;
            let mut params = json!({ "pane": pane_word(w.words.first(), "close")? });
            if w.flag("force") {
                params["force"] = json!(true);
            }
            call("close", params)
        }
        "zoom" => match args.first() {
            Some(_) => call("zoom", json!({ "pane": pane_word(args.first(), "zoom")? })),
            None => call("zoom", json!({})),
        },
        "ai" => ai_command(args)?,
        "panel" => match args.first() {
            Some(name) => call("panel", json!({ "name": name })),
            None => call("panel", json!({})),
        },
        "spawn" => {
            let w = read(&["profile", "cwd", "pane"], &["right", "down", "tab"])?;
            let place = if w.flag("right") {
                "right"
            } else if w.flag("down") {
                "down"
            } else {
                "tab"
            };
            let mut params = json!({ "place": place });
            for key in ["profile", "cwd"] {
                if let Some(v) = w.value(key) {
                    params[key] = json!(v);
                }
            }
            w.pane_into(&mut params)?;
            call("spawn", params)
        }
        "scene" => {
            let w = read(&["pane"], &["right", "down"])?;
            let place = if w.flag("down") { "down" } else { "right" };
            let mut params = json!({ "place": place });
            w.pane_into(&mut params)?;
            call("scene_open", params)
        }
        "draw" => {
            let w = read(&["pane"], &[])?;
            let text = w.text();
            if text.trim() == "-" {
                return Ok(Command::DrawStdin {
                    pane: w.number("pane")?,
                });
            }
            if text.trim().is_empty() {
                return Err("draw needs the commands as JSON (or `-` for stdin)".to_owned());
            }
            let ops: Value =
                serde_json::from_str(&text).map_err(|err| format!("bad JSON: {err}"))?;
            let mut params = json!({ "ops": ops });
            w.pane_into(&mut params)?;
            call("scene_draw", params)
        }
        "send-text" => {
            let w = read(&["pane"], &["enter"])?;
            let mut params = json!({ "text": w.text(), "enter": w.flag("enter") });
            w.pane_into(&mut params)?;
            call("send_text", params)
        }
        "get-text" => {
            let w = read(&["pane", "lines"], &["history", "last-output", "styled"])?;
            let what = if w.flag("last-output") {
                "last_output"
            } else if w.flag("history") {
                "history"
            } else {
                "screen"
            };
            let mut params = json!({ "what": what });
            if let Some(lines) = w.number("lines")? {
                params["lines"] = json!(lines);
            }
            if w.flag("styled") {
                params["styled"] = json!(true);
            }
            w.pane_into(&mut params)?;
            call("get_text", params)
        }
        "screenshot" => {
            let w = read(&["pane"], &[])?;
            let mut params = json!({});
            let file = w.text();
            if !file.trim().is_empty() {
                // fterm has another current folder: give it the full path.
                let full = std::path::absolute(file.trim())
                    .map_err(|err| format!("bad file `{file}`: {err}"))?;
                params["path"] = json!(full.to_string_lossy().replace('\\', "/"));
            }
            w.pane_into(&mut params)?;
            call("screenshot", params)
        }
        "title" => {
            let w = read(&["pane"], &[])?;
            let mut params = json!({ "title": w.text() });
            w.pane_into(&mut params)?;
            call("set_title", params)
        }
        "notify" => {
            let w = read(&["level", "body"], &[])?;
            let mut params = json!({ "title": w.text() });
            for key in ["level", "body"] {
                if let Some(v) = w.value(key) {
                    params[key] = json!(v);
                }
            }
            call("notify", params)
        }
        "run" => {
            let w = read(&["pane", "timeout"], &["wait"])?;
            let text = w.text();
            if text.trim().is_empty() {
                return Err("run needs a command".to_owned());
            }
            Command::Run {
                pane: w.number("pane")?,
                text,
                wait: w.flag("wait"),
                timeout_ms: w.number("timeout")?.map_or(600_000, |s| s * 1000),
            }
        }
        "wait-for" => {
            let w = read(&["pane", "pattern", "timeout"], &[])?;
            let event = w.words.first().ok_or("wait-for needs an event name")?;
            let mut params = json!({ "event": event });
            if let Some(pattern) = w.value("pattern") {
                params["pattern"] = json!(pattern);
            }
            if let Some(secs) = w.number("timeout")? {
                params["timeout_ms"] = json!(secs * 1000);
            }
            w.pane_into(&mut params)?;
            call("wait_for", params)
        }
        "send-message" => {
            let to = pane_word(args.first(), "send-message")?;
            let text = args[1..].join(" ");
            if text.trim().is_empty() {
                return Err("send-message needs a text".to_owned());
            }
            call("send_message", json!({ "to": to, "text": text }))
        }
        "read-messages" => {
            let w = read(&["pane"], &["all", "peek"])?;
            let mut params = json!({});
            if w.flag("all") {
                params["unread_only"] = json!(false);
            }
            if w.flag("peek") {
                params["mark_read"] = json!(false);
            }
            w.pane_into(&mut params)?;
            call("read_messages", params)
        }
        "subscribe" => Command::Subscribe {
            events: if args.is_empty() {
                vec!["*".to_owned()]
            } else {
                args.to_vec()
            },
        },
        "call" => {
            let method = args.first().ok_or("call needs a method name")?;
            let rest = args[1..].join(" ");
            let params = if rest.trim().is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&rest).map_err(|err| format!("bad JSON: {err}"))?
            };
            call(method, params)
        }
        other => return Err(format!("unknown command `{other}` (try `ftermctl help`)")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(line: &str) -> Result<Cli, String> {
        let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        parse(&args)
    }

    fn call(line: &str) -> (String, Value) {
        match cli(line).unwrap().command {
            Command::Call { method, params } => (method, params),
            other => panic!("{line}: {other:?}"),
        }
    }

    #[test]
    fn global_flags() {
        let c = cli("--window 42 --json list").unwrap();
        assert_eq!((c.window, c.json), (Some(42), true));
        assert_eq!(cli("").unwrap().command, Command::Help);
        assert_eq!(cli("help").unwrap().command, Command::Help);
        assert!(cli("--window x list").is_err());
        assert!(cli("dance").is_err());
    }

    #[test]
    fn simple_calls() {
        assert_eq!(call("list"), ("list".into(), json!({})));
        assert_eq!(call("focus 3"), ("focus".into(), json!({"pane": 3})));
        assert_eq!(
            call("close 3 --force"),
            ("close".into(), json!({"pane": 3, "force": true}))
        );
        assert_eq!(call("zoom"), ("zoom".into(), json!({})));
        assert_eq!(
            call("panel events"),
            ("panel".into(), json!({"name": "events"}))
        );
        assert_eq!(call("panel"), ("panel".into(), json!({})));
    }

    #[test]
    fn spawn() {
        assert_eq!(call("spawn"), ("spawn".into(), json!({"place": "tab"})));
        assert_eq!(
            call("spawn --right --profile Claude --cwd C:/work --pane 2"),
            (
                "spawn".into(),
                json!({"place": "right", "profile": "Claude", "cwd": "C:/work", "pane": 2})
            )
        );
        assert_eq!(call("spawn --down").1["place"], json!("down"));
    }

    #[test]
    fn the_guide_for_agents() {
        assert_eq!(
            cli("guide").unwrap().command,
            Command::Guide { skill: false }
        );
        assert_eq!(
            cli("guide --skill").unwrap().command,
            Command::Guide { skill: true },
            "the Claude Code skill file"
        );
    }

    #[test]
    fn scenes() {
        assert_eq!(
            call("scene"),
            ("scene_open".into(), json!({"place": "right"}))
        );
        assert_eq!(
            call("scene --down --pane 2"),
            ("scene_open".into(), json!({"place": "down", "pane": 2}))
        );
        assert_eq!(
            call(r#"draw [{"op":"clear"},{"op":"dot","x":1,"y":2}]"#),
            (
                "scene_draw".into(),
                json!({"ops": [{"op": "clear"}, {"op": "dot", "x": 1, "y": 2}]})
            )
        );
        // The JSON can have spaces (the shell splits it into words).
        assert_eq!(
            call(r#"draw --pane 3 {"op": "text", "col": 0, "row": 0, "text": "a b"}"#),
            (
                "scene_draw".into(),
                json!({"ops": {"op": "text", "col": 0, "row": 0, "text": "a b"}, "pane": 3})
            )
        );
        assert!(cli("draw").is_err(), "draw needs the commands");
        assert!(cli("draw {nope").is_err(), "bad JSON");
        assert_eq!(
            cli("draw --pane 4 -").unwrap().command,
            Command::DrawStdin { pane: Some(4) },
            "`-` = the commands come on stdin"
        );
    }

    #[test]
    fn screenshots() {
        assert_eq!(call("screenshot"), ("screenshot".into(), json!({})));
        let file = if cfg!(windows) {
            "C:/shots/a.png"
        } else {
            "/shots/a.png"
        };
        assert_eq!(
            call(&format!("screenshot --pane 3 {file}")),
            ("screenshot".into(), json!({"pane": 3, "path": file}))
        );
        // fterm has another folder: a short path is made full here.
        let (_, params) = call("screenshot map.png");
        let path = std::path::PathBuf::from(params["path"].as_str().unwrap());
        assert!(path.is_absolute() && path.ends_with("map.png"), "{path:?}");
    }

    #[test]
    fn ai_commands() {
        assert_eq!(call("ai read"), ("ai_read".into(), json!({})));
        assert_eq!(call("ai read --last 4").1, json!({"last": 4}));
        assert_eq!(
            call("ai ask --pane 2 --wait --timeout 60 why did it fail?"),
            (
                "ai_ask".into(),
                json!({"text": "why did it fail?", "pane": 2, "wait": true, "timeout_ms": 60_000})
            )
        );
        assert_eq!(call("ai ask hi").1, json!({"text": "hi"}));
        assert_eq!(
            call("ai input explain this"),
            ("ai_input".into(), json!({"text": "explain this"}))
        );
        assert_eq!(call("ai stop").0, "ai_stop");
        assert_eq!(call("ai clear").0, "ai_clear");
        assert!(cli("ai").is_err());
        assert!(cli("ai ask").is_err(), "no question");
        assert!(cli("ai dance").is_err());
    }

    #[test]
    fn text_commands() {
        assert_eq!(
            call("send-text --pane 2 git status"),
            (
                "send_text".into(),
                json!({"pane": 2, "text": "git status", "enter": false})
            )
        );
        assert_eq!(
            call("get-text --pane 2 --last-output --lines 20"),
            (
                "get_text".into(),
                json!({"pane": 2, "what": "last_output", "lines": 20})
            )
        );
        assert_eq!(call("get-text --history").1, json!({"what": "history"}));
        assert_eq!(call("get-text").1, json!({"what": "screen"}));
        assert_eq!(
            call("get-text --styled --lines 5").1,
            json!({"what": "screen", "lines": 5, "styled": true})
        );
        assert_eq!(
            call("title --pane 4 build server"),
            (
                "set_title".into(),
                json!({"pane": 4, "title": "build server"})
            )
        );
        assert_eq!(
            call("notify Deploy is done --level success"),
            (
                "notify".into(),
                json!({"title": "Deploy is done", "level": "success"})
            )
        );
    }

    #[test]
    fn run() {
        assert_eq!(
            cli("run --pane 3 --wait --timeout 60 cargo test")
                .unwrap()
                .command,
            Command::Run {
                pane: Some(3),
                text: "cargo test".into(),
                wait: true,
                timeout_ms: 60_000,
            }
        );
        let Command::Run {
            wait, timeout_ms, ..
        } = cli("run ls").unwrap().command
        else {
            panic!("run");
        };
        assert!(!wait);
        assert_eq!(timeout_ms, 600_000, "ten minutes by default");
        assert!(cli("run").is_err(), "run needs a command");
    }

    #[test]
    fn waits_and_messages() {
        assert_eq!(
            call("wait-for --pane 3 text --pattern ok --timeout 5"),
            (
                "wait_for".into(),
                json!({"pane": 3, "event": "text", "pattern": "ok", "timeout_ms": 5000})
            )
        );
        assert_eq!(
            call("send-message 2 please review"),
            (
                "send_message".into(),
                json!({"to": 2, "text": "please review"})
            )
        );
        assert_eq!(
            call("read-messages --all --peek"),
            (
                "read_messages".into(),
                json!({"unread_only": false, "mark_read": false})
            )
        );
        assert!(cli("send-message please").is_err(), "the pane id first");
    }

    #[test]
    fn raw_call_subscribe_and_mcp() {
        assert_eq!(call(r#"call list"#), ("list".into(), Value::Null));
        let args = vec![
            "call".to_owned(),
            "notify".to_owned(),
            r#"{"title": "hi"}"#.to_owned(),
        ];
        assert_eq!(
            parse(&args).unwrap().command,
            Command::Call {
                method: "notify".into(),
                params: json!({"title": "hi"})
            }
        );
        assert_eq!(
            cli("subscribe command_done agent_state").unwrap().command,
            Command::Subscribe {
                events: vec!["command_done".into(), "agent_state".into()]
            }
        );
        assert_eq!(
            cli("subscribe").unwrap().command,
            Command::Subscribe {
                events: vec!["*".into()]
            }
        );
        assert_eq!(cli("mcp").unwrap().command, Command::Mcp);
    }
}
