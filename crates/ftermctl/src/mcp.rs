//! `ftermctl mcp`: an MCP server on stdin/stdout. MCP clients (Claude Code, OpenCode, ...) start it;
//! it talks to the fterm window over the API.
//!
//! MCP on stdio is JSON-RPC 2.0 with one message per line: `initialize`, `tools/list`, `tools/call`, `ping`.

use serde_json::{Value, json};

/// The MCP versions that this server knows. It answers with the client's version when it knows it.
pub const PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// What the MCP server needs from fterm (the real one talks to the window; tests use a fake).
pub trait Backend {
    /// The MCP client said its name (in `initialize`).
    fn set_client_name(&mut self, name: &str);
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    fn run_and_wait(
        &mut self,
        pane: Option<u64>,
        command: &str,
        timeout_ms: u64,
    ) -> Result<Value, String>;
}

pub struct Server<B: Backend> {
    pub backend: B,
}

/// The tools with their JSON schemas.
pub fn tools() -> Value {
    let pane = json!({ "type": "integer", "description": "The pane id (from list_panes). No pane = your own pane." });
    let timeout = json!({ "type": "integer", "description": "How long to wait, in seconds." });
    json!([
        {
            "name": "list_panes",
            "description": "List all tabs and panes of the fterm window: pane ids, programs, folders, agent states, the last command and its exit code.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "open_pane",
            "description": "Open a new pane: a new tab, or a split on the right or below. Returns the new pane id.",
            "inputSchema": { "type": "object", "properties": {
                "place": { "type": "string", "enum": ["tab", "right", "down"], "description": "Where. The default is a new tab." },
                "profile": { "type": "string", "description": "An fterm profile name (for example PowerShell). No profile = the default one." },
                "cwd": { "type": "string", "description": "The folder to start in." },
                "next_to": { "type": "integer", "description": "Split next to this pane." }
            }}
        },
        {
            "name": "run_command",
            "description": "Type a command into a pane, press Enter, wait until it ends, and return its exit code and output. Use it to run tests or builds in another pane.",
            "inputSchema": { "type": "object", "properties": {
                "command": { "type": "string", "description": "The command line, for the shell of that pane." },
                "pane": pane,
                "timeout_seconds": { "type": "integer", "description": "How long to wait. The default is 600." }
            }, "required": ["command"] }
        },
        {
            "name": "send_text",
            "description": "Type text into a pane (for example an answer to a program). It does not wait.",
            "inputSchema": { "type": "object", "properties": {
                "text": { "type": "string" },
                "pane": pane,
                "enter": { "type": "boolean", "description": "Press Enter after the text." }
            }, "required": ["text"] }
        },
        {
            "name": "read_pane",
            "description": "Read the text of a pane: the screen, the history, or the output of the last command.",
            "inputSchema": { "type": "object", "properties": {
                "pane": pane,
                "what": { "type": "string", "enum": ["screen", "history", "last_output"], "description": "The default is screen." },
                "lines": { "type": "integer", "description": "For history and last_output: only the last lines (default 200)." }
            }}
        },
        {
            "name": "wait_for",
            "description": "Wait until something happens in a pane: a command ends, an agent is done or waits, a message comes, or a text shows on the screen.",
            "inputSchema": { "type": "object", "properties": {
                "event": { "type": "string", "enum": ["command_done", "agent_done", "agent_waiting", "message", "text"] },
                "pattern": { "type": "string", "description": "For text: the text to wait for." },
                "pane": pane,
                "timeout_seconds": timeout
            }, "required": ["event"] }
        },
        {
            "name": "notify",
            "description": "Show a notification to the user in fterm.",
            "inputSchema": { "type": "object", "properties": {
                "title": { "type": "string" },
                "body": { "type": "string" },
                "level": { "type": "string", "enum": ["info", "success", "warning", "error", "attention"] }
            }, "required": ["title"] }
        },
        {
            "name": "send_message",
            "description": "Send a message to the agent in another pane. It goes to the inbox of that pane; the agent reads it with read_messages.",
            "inputSchema": { "type": "object", "properties": {
                "to": { "type": "integer", "description": "The pane id of the other agent." },
                "text": { "type": "string" }
            }, "required": ["to", "text"] }
        },
        {
            "name": "read_messages",
            "description": "Read the messages that other agents sent to your pane (or to another pane).",
            "inputSchema": { "type": "object", "properties": {
                "pane": pane,
                "unread_only": { "type": "boolean", "description": "Only new messages (the default)." }
            }}
        },
        {
            "name": "focus_pane",
            "description": "Show a pane to the user (go to its tab and give it the keyboard).",
            "inputSchema": { "type": "object", "properties": { "pane": pane }, "required": ["pane"] }
        },
        {
            "name": "close_pane",
            "description": "Close a pane. A pane where a program runs needs force = true.",
            "inputSchema": { "type": "object", "properties": {
                "pane": pane,
                "force": { "type": "boolean" }
            }, "required": ["pane"] }
        },
        {
            "name": "set_title",
            "description": "Set the title of the tab of a pane. An empty title = the automatic title.",
            "inputSchema": { "type": "object", "properties": {
                "title": { "type": "string" },
                "pane": pane
            }, "required": ["title"] }
        },
        {
            "name": "open_scene",
            "description": "Open a Braille scene: a pane that you draw into (charts, diagrams, simple pictures). Each cell is 2x4 dots. Returns the pane id, the size in dots, and the aspect (dot height / width). For a chart of numbers use plot.",
            "inputSchema": { "type": "object", "properties": {
                "place": { "type": "string", "enum": ["right", "down"], "description": "A split on the right (default) or below." },
                "next_to": { "type": "integer", "description": "Split next to this pane." }
            }}
        },
        {
            "name": "draw_scene",
            "description": "Draw into a scene. Commands (x, y in dots from the top left; text in cells): {op:clear}, {op:color, color:'#rrggbb'} (no color = normal), {op:dot|undot, x, y}, {op:line, x0, y0, x1, y1}, {op:rect, x, y, w, h, fill}, {op:circle, x, y, r, fill}, {op:text, col, row, text}, {op:plot, values, min, max, bars, x, y, w, h}. All commands of one call are drawn at once.",
            "inputSchema": { "type": "object", "properties": {
                "pane": { "type": "integer", "description": "The scene pane (from open_scene)." },
                "commands": { "type": "array", "items": { "type": "object" }, "description": "The drawing commands." }
            }, "required": ["pane", "commands"] }
        },
        {
            "name": "plot",
            "description": "Draw a chart of numbers in a scene: a line from left to right (or bars), scaled to the scene. Call it again with new values for a live chart.",
            "inputSchema": { "type": "object", "properties": {
                "pane": { "type": "integer", "description": "The scene pane (from open_scene)." },
                "values": { "type": "array", "items": { "type": "number" }, "description": "The values, from left to right." },
                "bars": { "type": "boolean", "description": "Bars instead of a line." },
                "min": { "type": "number", "description": "The bottom of the scale. The default is the smallest value." },
                "max": { "type": "number", "description": "The top of the scale. The default is the biggest value." },
                "color": { "type": "string", "description": "#rrggbb" },
                "title": { "type": "string", "description": "Text in the top left corner." },
                "clear": { "type": "boolean", "description": "Clear the scene first (default true)." }
            }, "required": ["pane", "values"] }
        }
    ])
}

impl<B: Backend> Server<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// One message from the client. `None` = no answer (a notification).
    pub fn handle(&mut self, message: &Value) -> Option<Value> {
        let method = message["method"].as_str().unwrap_or_default();
        // A message without an id is a notification (for example notifications/initialized).
        let id = message.get("id").cloned()?;
        let params = &message["params"];
        let answer = match method {
            "initialize" => {
                if let Some(name) = params["clientInfo"]["name"].as_str() {
                    self.backend.set_client_name(name);
                }
                let asked = params["protocolVersion"].as_str().unwrap_or_default();
                let version = PROTOCOLS
                    .iter()
                    .find(|v| **v == asked)
                    .copied()
                    .unwrap_or(PROTOCOLS[0]);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "fterm", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "These tools control the fterm terminal window that you run in. \
                        Panes have ids (see list_panes); without a pane id a tool uses your own pane. \
                        run_command runs a command in a pane and gives its output and exit code. \
                        Agents in other panes get messages with send_message.",
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match self.tool(name, &args) {
                    None => Err((-32602, format!("no tool `{name}`"))),
                    Some(Ok(text)) => Ok(
                        json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
                    ),
                    Some(Err(text)) => Ok(
                        json!({ "content": [{ "type": "text", "text": text }], "isError": true }),
                    ),
                }
            }
            other => Err((-32601, format!("no method `{other}`"))),
        };
        Some(match answer {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        })
    }
}

impl<B: Backend> Server<B> {
    /// One tool. `None` = no such tool; `Some(Err)` = a tool error (the agent sees the text).
    fn tool(&mut self, name: &str, args: &Value) -> Option<Result<String, String>> {
        // An argument that the tool does not have: say which ones it has, so the agent can fix it.
        let all = tools();
        let schema = all.as_array()?.iter().find(|t| t["name"] == name)?;
        let known: Vec<&str> = schema["inputSchema"]["properties"]
            .as_object()
            .map(|p| p.keys().map(String::as_str).collect())
            .unwrap_or_default();
        if let Some(bad) = args
            .as_object()
            .and_then(|a| a.keys().find(|k| !known.contains(&k.as_str())))
        {
            return Some(Err(if known.is_empty() {
                format!("unknown argument `{bad}`; this tool takes no arguments")
            } else {
                format!(
                    "unknown argument `{bad}`; this tool takes: {}",
                    known.join(", ")
                )
            }));
        }
        let pick = |keys: &[(&str, &str)]| {
            let mut params = json!({});
            for (from, to) in keys {
                if let Some(value) = args.get(*from).filter(|v| !v.is_null()) {
                    params[*to] = value.clone();
                }
            }
            params
        };
        let seconds = |key: &str| args.get(key).and_then(Value::as_u64).map(|s| s * 1000);
        let need = |key: &str| {
            args.get(key)
                .filter(|v| !v.is_null())
                .ok_or_else(|| format!("`{key}` is missing"))
        };
        let pretty = |v: Value| serde_json::to_string_pretty(&v).unwrap_or_default();
        let result = match name {
            "list_panes" => self.backend.call("list", json!({})).map(pretty),
            "open_pane" => self
                .backend
                .call(
                    "spawn",
                    pick(&[
                        ("place", "place"),
                        ("profile", "profile"),
                        ("cwd", "cwd"),
                        ("next_to", "pane"),
                    ]),
                )
                .map(|v| format!("Opened pane {}.", v["pane"])),
            "run_command" => need("command").and_then(|command| {
                let command = command
                    .as_str()
                    .ok_or("`command` must be a string")?
                    .to_owned();
                let pane = args.get("pane").and_then(Value::as_u64);
                let timeout = seconds("timeout_seconds").unwrap_or(600_000);
                let r = self.backend.run_and_wait(pane, &command, timeout)?;
                Ok(format!(
                    "Exit code: {} (pane {}, {} ms)\n\n{}",
                    r["exit"],
                    r["pane"],
                    r["took_ms"],
                    r["output"].as_str().unwrap_or("")
                ))
            }),
            "send_text" => need("text").and_then(|_| {
                self.backend
                    .call(
                        "send_text",
                        pick(&[("pane", "pane"), ("text", "text"), ("enter", "enter")]),
                    )
                    .map(|_| "Done.".to_owned())
            }),
            "read_pane" => self
                .backend
                .call(
                    "get_text",
                    pick(&[("pane", "pane"), ("what", "what"), ("lines", "lines")]),
                )
                .map(|v| v["text"].as_str().unwrap_or("").to_owned()),
            "wait_for" => need("event").and_then(|_| {
                let mut params =
                    pick(&[("pane", "pane"), ("event", "event"), ("pattern", "pattern")]);
                if let Some(ms) = seconds("timeout_seconds") {
                    params["timeout_ms"] = json!(ms);
                }
                self.backend.call("wait_for", params).map(pretty)
            }),
            "notify" => need("title").and_then(|_| {
                self.backend
                    .call(
                        "notify",
                        pick(&[("title", "title"), ("body", "body"), ("level", "level")]),
                    )
                    .map(|_| "Done.".to_owned())
            }),
            "send_message" => need("to").and(need("text")).and_then(|_| {
                self.backend
                    .call("send_message", pick(&[("to", "to"), ("text", "text")]))
                    .map(|v| {
                        format!(
                            "Message {} is in the inbox of pane {}.",
                            v["id"], args["to"]
                        )
                    })
            }),
            "read_messages" => self
                .backend
                .call(
                    "read_messages",
                    pick(&[("pane", "pane"), ("unread_only", "unread_only")]),
                )
                .map(pretty),
            "focus_pane" => need("pane").and_then(|_| {
                self.backend
                    .call("focus", pick(&[("pane", "pane")]))
                    .map(|_| "Done.".to_owned())
            }),
            "close_pane" => need("pane").and_then(|_| {
                self.backend
                    .call("close", pick(&[("pane", "pane"), ("force", "force")]))
                    .map(|_| "Closed.".to_owned())
            }),
            "set_title" => need("title").and_then(|_| {
                self.backend
                    .call("set_title", pick(&[("pane", "pane"), ("title", "title")]))
                    .map(|_| "Done.".to_owned())
            }),
            "open_scene" => self
                .backend
                .call(
                    "scene_open",
                    pick(&[("place", "place"), ("next_to", "pane")]),
                )
                .map(|v| {
                    format!(
                        "Opened scene pane {}: {}x{} dots (aspect {}). Draw with draw_scene or plot.",
                        v["pane"], v["width"], v["height"], v["aspect"]
                    )
                }),
            "draw_scene" => need("pane")
                .and_then(|_| need("commands"))
                .and_then(|commands| {
                    let params = json!({ "pane": args["pane"], "ops": commands });
                    self.backend.call("scene_draw", params).map(|v| {
                        format!("Drawn. The scene is {}x{} dots.", v["width"], v["height"])
                    })
                }),
            "plot" => need("pane").and_then(|_| need("values")).and_then(|values| {
                let mut ops = Vec::new();
                if args.get("clear").and_then(Value::as_bool).unwrap_or(true) {
                    ops.push(json!({ "op": "clear" }));
                }
                let color = args.get("color").filter(|c| !c.is_null());
                if let Some(color) = color {
                    ops.push(json!({ "op": "color", "color": color }));
                }
                let mut plot = json!({ "op": "plot", "values": values });
                for key in ["min", "max", "bars"] {
                    if let Some(v) = args.get(key).filter(|v| !v.is_null()) {
                        plot[key] = v.clone();
                    }
                }
                ops.push(plot);
                if let Some(title) = args.get("title").and_then(Value::as_str) {
                    if color.is_some() {
                        ops.push(json!({ "op": "color" }));
                    }
                    ops.push(json!({ "op": "text", "col": 0, "row": 0, "text": title }));
                }
                let params = json!({ "pane": args["pane"], "ops": ops });
                self.backend
                    .call("scene_draw", params)
                    .map(|_| "Plotted.".to_owned())
            }),
            _ => return None,
        };
        Some(result)
    }
}

/// The real backend: one connection to the window, made at the first tool call.
struct Window {
    window: Option<u32>,
    name: String,
    client: Option<fterm_api::client::Client>,
}

impl Window {
    fn client(&mut self) -> Result<&mut fterm_api::client::Client, String> {
        if self.client.is_none() {
            self.client = Some(crate::run::connect_as(self.window, &self.name)?);
        }
        Ok(self.client.as_mut().expect("just made"))
    }
}

impl Backend for Window {
    fn set_client_name(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let result = self.client()?.call(method, params);
        // A broken connection (fterm closed): connect again next time.
        if let Err(fterm_api::client::ClientError::Io(_)) = &result {
            self.client = None;
        }
        result.map_err(|err| err.to_string())
    }

    fn run_and_wait(
        &mut self,
        pane: Option<u64>,
        command: &str,
        timeout_ms: u64,
    ) -> Result<Value, String> {
        // On our own connection, so the user's answer to the access question counts for it.
        let window = self.window;
        let client = self.client()?;
        crate::run::run_and_wait_with(client, window, pane, command, timeout_ms)
    }
}

/// One line from the client as JSON. A UTF-8 BOM in front (PowerShell 5.1 sends one) is skipped.
pub fn parse_line(line: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(line.trim_start_matches('\u{feff}'))
}

/// Reads stdin and writes stdout until the client closes stdin.
pub fn serve(window: Option<u32>) -> Result<(), String> {
    use std::io::{BufRead, Write};

    let mut server = Server::new(Window {
        window,
        name: "mcp".to_owned(),
        client: None,
    });
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|err| err.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let answer = match parse_line(&line) {
            Ok(message) => server.handle(&message),
            Err(err) => Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": err.to_string() },
            })),
        };
        if let Some(answer) = answer {
            let text = serde_json::to_string(&answer).unwrap_or_default();
            writeln!(stdout, "{text}").map_err(|err| err.to_string())?;
            stdout.flush().map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        name: String,
        calls: Vec<(String, Value)>,
        runs: Vec<(Option<u64>, String, u64)>,
        fail: bool,
    }

    impl Backend for Fake {
        fn set_client_name(&mut self, name: &str) {
            self.name = name.to_owned();
        }

        fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
            self.calls.push((method.to_owned(), params));
            if self.fail {
                return Err("no fterm window found".into());
            }
            Ok(match method {
                "list" => json!({ "tabs": [], "active_pane": 1 }),
                "spawn" => json!({ "pane": 7 }),
                "get_text" => json!({ "pane": 1, "text": "hello\nworld" }),
                "send_message" => json!({ "id": 3 }),
                "scene_open" | "scene_draw" => {
                    json!({ "pane": 7, "cols": 66, "rows": 21, "width": 132, "height": 84, "aspect": 1.1875 })
                }
                _ => json!({}),
            })
        }

        fn run_and_wait(
            &mut self,
            pane: Option<u64>,
            command: &str,
            timeout_ms: u64,
        ) -> Result<Value, String> {
            self.runs.push((pane, command.to_owned(), timeout_ms));
            Ok(
                json!({ "pane": 2, "command": command, "exit": 1, "took_ms": 5, "output": "1 test failed" }),
            )
        }
    }

    fn server() -> Server<Fake> {
        Server::new(Fake::default())
    }

    fn request(id: u64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    fn tool(s: &mut Server<Fake>, name: &str, args: Value) -> Value {
        s.handle(&request(
            9,
            "tools/call",
            json!({ "name": name, "arguments": args }),
        ))
        .unwrap()["result"]
            .clone()
    }

    #[test]
    fn a_line_with_a_byte_order_mark() {
        let line = "\u{feff}{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}";
        assert_eq!(parse_line(line).unwrap()["method"], json!("ping"));
        assert!(parse_line("not json").is_err());
    }

    #[test]
    fn the_handshake() {
        let mut s = server();
        let answer = s
            .handle(&request(
                1,
                "initialize",
                json!({ "protocolVersion": "2025-03-26", "capabilities": {},
                        "clientInfo": { "name": "claude-code", "version": "2" } }),
            ))
            .unwrap();
        assert_eq!(answer["id"], json!(1));
        assert_eq!(
            answer["result"]["protocolVersion"],
            json!("2025-03-26"),
            "the client's version"
        );
        assert!(answer["result"]["capabilities"]["tools"].is_object());
        assert_eq!(answer["result"]["serverInfo"]["name"], json!("fterm"));
        assert_eq!(s.backend.name, "claude-code");
        // An unknown version: the newest one that we know.
        let answer = s
            .handle(&request(
                2,
                "initialize",
                json!({ "protocolVersion": "1999-01-01" }),
            ))
            .unwrap();
        assert_eq!(answer["result"]["protocolVersion"], json!(PROTOCOLS[0]));
        // Notifications get no answer.
        assert_eq!(
            s.handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
            None
        );
        assert_eq!(
            s.handle(&request(3, "ping", json!({}))).unwrap()["result"],
            json!({})
        );
    }

    #[test]
    fn the_tools_have_schemas() {
        let list = server()
            .handle(&request(1, "tools/list", json!({})))
            .unwrap();
        let tools = list["result"]["tools"].as_array().unwrap().clone();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for name in [
            "list_panes",
            "open_pane",
            "run_command",
            "send_text",
            "read_pane",
            "wait_for",
            "notify",
            "send_message",
            "read_messages",
            "focus_pane",
            "close_pane",
            "set_title",
            "open_scene",
            "draw_scene",
            "plot",
        ] {
            assert!(names.contains(&name), "{name}");
        }
        for t in &tools {
            assert_eq!(t["inputSchema"]["type"], json!("object"), "{}", t["name"]);
            assert!(t["description"].as_str().is_some_and(|d| d.len() > 10));
        }
        let run = tools.iter().find(|t| t["name"] == "run_command").unwrap();
        assert_eq!(run["inputSchema"]["required"], json!(["command"]));
    }

    #[test]
    fn tools_call_the_api() {
        let mut s = server();
        let result = tool(
            &mut s,
            "open_pane",
            json!({ "place": "right", "next_to": 2 }),
        );
        assert_eq!(result["isError"], json!(false));
        assert_eq!(result["content"][0]["text"], json!("Opened pane 7."));
        assert_eq!(
            s.backend.calls[0],
            ("spawn".into(), json!({ "place": "right", "pane": 2 }))
        );

        let result = tool(
            &mut s,
            "read_pane",
            json!({ "pane": 1, "what": "last_output" }),
        );
        assert_eq!(result["content"][0]["text"], json!("hello\nworld"));
        assert_eq!(
            s.backend.calls[1].1,
            json!({ "pane": 1, "what": "last_output" })
        );

        tool(
            &mut s,
            "send_text",
            json!({ "pane": 1, "text": "ls", "enter": true }),
        );
        assert_eq!(
            s.backend.calls[2],
            (
                "send_text".into(),
                json!({ "pane": 1, "text": "ls", "enter": true })
            )
        );

        tool(
            &mut s,
            "wait_for",
            json!({ "event": "text", "pattern": "ok", "timeout_seconds": 5 }),
        );
        assert_eq!(
            s.backend.calls[3],
            (
                "wait_for".into(),
                json!({ "event": "text", "pattern": "ok", "timeout_ms": 5000 })
            )
        );

        let result = tool(&mut s, "send_message", json!({ "to": 2, "text": "hi" }));
        assert_eq!(
            result["content"][0]["text"],
            json!("Message 3 is in the inbox of pane 2.")
        );
    }

    #[test]
    fn scenes_for_agents() {
        let mut s = server();
        let result = tool(
            &mut s,
            "open_scene",
            json!({ "place": "down", "next_to": 2 }),
        );
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("pane 7") && text.contains("132x84"), "{text}");
        assert_eq!(
            s.backend.calls[0],
            ("scene_open".into(), json!({ "place": "down", "pane": 2 }))
        );

        let ops = json!([{ "op": "clear" }, { "op": "dot", "x": 1, "y": 1 }]);
        tool(&mut s, "draw_scene", json!({ "pane": 7, "commands": ops }));
        assert_eq!(
            s.backend.calls[1],
            ("scene_draw".into(), json!({ "pane": 7, "ops": ops }))
        );

        let result = tool(
            &mut s,
            "plot",
            json!({ "pane": 7, "values": [1, 3, 2], "color": "#40c0ff", "title": "CPU %", "max": 100 }),
        );
        assert_eq!(result["isError"], json!(false));
        assert_eq!(
            s.backend.calls[2],
            (
                "scene_draw".into(),
                json!({ "pane": 7, "ops": [
                    { "op": "clear" },
                    { "op": "color", "color": "#40c0ff" },
                    { "op": "plot", "values": [1, 3, 2], "max": 100 },
                    { "op": "color" },
                    { "op": "text", "col": 0, "row": 0, "text": "CPU %" },
                ] })
            )
        );
        // Bars, and on top of the last picture (no clear).
        tool(
            &mut s,
            "plot",
            json!({ "pane": 7, "values": [5], "bars": true, "clear": false }),
        );
        assert_eq!(
            s.backend.calls[3].1["ops"],
            json!([{ "op": "plot", "values": [5], "bars": true }])
        );
        let result = tool(&mut s, "plot", json!({ "pane": 7 }));
        assert_eq!(result["isError"], json!(true), "values are needed");
        let result = tool(&mut s, "draw_scene", json!({ "commands": [] }));
        assert_eq!(result["isError"], json!(true), "the scene pane is needed");
    }

    #[test]
    fn run_command_gives_the_exit_code_and_the_output() {
        let mut s = server();
        let result = tool(
            &mut s,
            "run_command",
            json!({ "command": "cargo test", "pane": 2, "timeout_seconds": 60 }),
        );
        assert_eq!(s.backend.runs[0], (Some(2), "cargo test".into(), 60_000));
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("Exit code: 1"), "{text}");
        assert!(text.contains("1 test failed"));
        assert_eq!(
            result["isError"],
            json!(false),
            "a failed command is a result, not a tool error"
        );
    }

    #[test]
    fn an_unknown_argument_is_a_tool_error_that_helps() {
        let mut s = server();
        let result = tool(
            &mut s,
            "read_pane",
            json!({ "pane": 3, "mode": "last_command" }),
        );
        assert_eq!(result["isError"], json!(true));
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("`mode`"), "{text}");
        assert!(
            text.contains("pane, what, lines"),
            "the right names: {text}"
        );
        assert!(s.backend.calls.is_empty(), "nothing went to fterm");
    }

    #[test]
    fn errors() {
        let mut s = server();
        // A missing argument: a tool error (the agent can fix it), not a protocol error.
        let result = tool(&mut s, "run_command", json!({}));
        assert_eq!(result["isError"], json!(true));
        // fterm is not there.
        s.backend.fail = true;
        let result = tool(&mut s, "list_panes", json!({}));
        assert_eq!(result["isError"], json!(true));
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("no fterm window")
        );
        // An unknown tool, and an unknown method.
        let answer = s
            .handle(&request(5, "tools/call", json!({ "name": "dance" })))
            .unwrap();
        assert_eq!(answer["error"]["code"], json!(-32602));
        let answer = s.handle(&request(6, "resources/list", json!({}))).unwrap();
        assert_eq!(answer["error"]["code"], json!(-32601));
    }
}
