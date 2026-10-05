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
    /// A file that fterm wrote (a screenshot).
    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, String>;
}

/// Base64 (standard, with `=`) of `bytes`: MCP sends images like this.
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub struct Server<B: Backend> {
    pub backend: B,
}

/// The tools with their JSON schemas.
use fterm_api::guide::INSTRUCTIONS;

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
                "lines": { "type": "integer", "description": "For history and last_output: only the last lines (default 200)." },
                "styled": { "type": "boolean", "description": "Also give the colors and styles: the default fg and bg, and each line as runs of text with fg, bg (#rrggbb), bold, italic, underline, strike, dim (only what is not the default). For example to see which lines are red errors." }
            }}
        },
        {
            "name": "wait_for",
            "description": "Wait until something happens in a pane: a command ends, an agent is done or waits, a message comes, or a text shows on the screen.",
            "inputSchema": { "type": "object", "properties": {
                "event": { "type": "string", "enum": ["command_done", "agent_done", "agent_waiting", "message", "text", "scene_resized"] },
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
            "name": "set_tab_color",
            "description": "Give the tab of a pane a color: a line at the top of the tab, so the user sees it from other tabs. For example red for failed tests, green when done. \"none\" takes the color away.",
            "inputSchema": { "type": "object", "properties": {
                "pane": pane,
                "color": { "type": "string", "description": "#rrggbb (or #rgb), or \"none\"." }
            }, "required": ["color"] }
        },
        {
            "name": "screenshot_pane",
            "description": "Take a picture (PNG) of a pane, as the user sees it: colors, scenes, the layout. Use it to check what you drew or how a program looks. The pane must be on the screen (in the active tab).",
            "inputSchema": { "type": "object", "properties": {
                "pane": pane,
                "path": { "type": "string", "description": "Save the PNG here (a full path that ends with .png). No path = a file in the temp folder." }
            }}
        },
        {
            "name": "ai_read",
            "description": "Read the chat of the fterm AI panel (the user's own AI assistant): the questions, the answers, and the text in its input box.",
            "inputSchema": { "type": "object", "properties": {
                "last": { "type": "integer", "description": "Only the last turns (a question and its answer are 2 turns)." }
            }}
        },
        {
            "name": "ai_ask",
            "description": "Ask a question in the fterm AI panel and wait for the answer. It uses the user's AI key, so it works only when the user set ai = { api_access = true } in fterm.lua. The user sees the question and the answer in the panel.",
            "inputSchema": { "type": "object", "properties": {
                "text": { "type": "string", "description": "The question." },
                "pane": { "type": "integer", "description": "Send the last command of this pane and its output with the question." },
                "timeout": timeout
            }, "required": ["text"] }
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
                    "instructions": INSTRUCTIONS,
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
                    Some(Ok(content)) => Ok(json!({ "content": content, "isError": false })),
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
    /// One tool: the MCP content (text, or an image too). `None` = no such tool;
    /// `Some(Err)` = a tool error (the agent sees the text).
    fn tool(&mut self, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
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
        if name == "screenshot_pane" {
            let params = pick(&[("pane", "pane"), ("path", "path")]);
            return Some(self.screenshot(params));
        }
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
                    pick(&[
                        ("pane", "pane"),
                        ("what", "what"),
                        ("lines", "lines"),
                        ("styled", "styled"),
                    ]),
                )
                .map(|mut v| {
                    if args.get("styled") == Some(&json!(true)) {
                        // The lines have the text already: do not send it twice.
                        if let Some(o) = v.as_object_mut() {
                            o.remove("text");
                        }
                        serde_json::to_string(&v).unwrap_or_default()
                    } else {
                        v["text"].as_str().unwrap_or("").to_owned()
                    }
                }),
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
            "ai_read" => self
                .backend
                .call("ai_read", pick(&[("last", "last")]))
                .map(|v| crate::show::chat(&v)),
            "ai_ask" => need("text").and_then(|_| {
                let mut params = pick(&[("text", "text"), ("pane", "pane")]);
                params["wait"] = json!(true);
                if let Some(ms) = seconds("timeout") {
                    params["timeout_ms"] = json!(ms);
                }
                let v = self.backend.call("ai_ask", params)?;
                match v["error"].as_str() {
                    Some(error) => Err(format!("The AI answer failed: {error}")),
                    None => Ok(v["text"].as_str().unwrap_or("").to_owned()),
                }
            }),
            "set_tab_color" => need("color").and_then(|_| {
                self.backend
                    .call(
                        "set_tab_color",
                        pick(&[("pane", "pane"), ("color", "color")]),
                    )
                    .map(|_| "Done.".to_owned())
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
        Some(result.map(|text| vec![json!({ "type": "text", "text": text })]))
    }

    /// `screenshot_pane`: fterm writes a PNG; the agent gets the picture and where the file is.
    fn screenshot(&mut self, params: Value) -> Result<Vec<Value>, String> {
        let shot = self.backend.call("screenshot", params)?;
        let path = shot["path"].as_str().ok_or("fterm gave no file")?;
        let png = self.backend.read_file(path)?;
        Ok(vec![
            json!({ "type": "image", "mimeType": "image/png", "data": base64(&png) }),
            json!({ "type": "text", "text": format!(
                "Pane {}: {}x{} pixels, saved as {path}.",
                shot["pane"], shot["width"], shot["height"]
            ) }),
        ])
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

    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|err| format!("cannot read {path}: {err}"))
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
            self.calls.push((method.to_owned(), params.clone()));
            if self.fail {
                return Err("no fterm window found".into());
            }
            Ok(match method {
                "list" => json!({ "tabs": [], "active_pane": 1 }),
                "spawn" => json!({ "pane": 7 }),
                "get_text" if params["styled"] == json!(true) => json!({
                    "pane": 1, "text": "ok error", "fg": "#cdd6f4", "bg": "#1e1e2e",
                    "lines": [[{ "text": "ok " }, { "text": "error", "fg": "#f38ba8" }]]
                }),
                "get_text" => json!({ "pane": 1, "text": "hello\nworld" }),
                "send_message" => json!({ "id": 3 }),
                "ai_read" => json!({
                    "turns": [
                        { "role": "user", "text": "what is ls?", "streaming": false },
                        { "role": "ai", "text": "It lists files.", "streaming": false }
                    ],
                    "input": "", "context": [], "running": false
                }),
                "ai_ask" if params["text"] == "fail" => {
                    json!({ "id": 2, "text": "", "error": "no API key" })
                }
                "ai_ask" => json!({ "id": 1, "text": "Use dir." }),
                "screenshot" => {
                    json!({ "pane": 4, "path": "T:/fterm-shot-4.png", "width": 640, "height": 300 })
                }
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

        fn read_file(&mut self, path: &str) -> Result<Vec<u8>, String> {
            assert_eq!(path, "T:/fterm-shot-4.png");
            Ok(b"PNG!".to_vec())
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
    fn the_instructions_tell_how_to_work() {
        let answer = server()
            .handle(&request(
                1,
                "initialize",
                json!({ "protocolVersion": "2025-06-18" }),
            ))
            .unwrap();
        let text = answer["result"]["instructions"].as_str().unwrap();
        for word in [
            "run_command",
            "wait_for",
            "send_message",
            "open_scene",
            "plot",
            "ftermctl guide",
        ] {
            assert!(text.contains(word), "{word}");
        }
    }

    #[test]
    fn the_guide_names_every_tool() {
        // A new tool must come into the guide too.
        for t in tools().as_array().unwrap() {
            let name = t["name"].as_str().unwrap();
            assert!(
                fterm_api::guide::GUIDE.contains(&format!("`{name}`")),
                "{name} is not in GUIDE.md"
            );
        }
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
            "screenshot_pane",
            "ai_read",
            "ai_ask",
            "set_tab_color",
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
    fn a_tab_color() {
        let mut s = server();
        let result = tool(
            &mut s,
            "set_tab_color",
            json!({ "pane": 2, "color": "#a6e3a1" }),
        );
        assert_eq!(result["isError"], json!(false), "{result}");
        assert_eq!(
            s.backend.calls[0],
            (
                "set_tab_color".into(),
                json!({ "pane": 2, "color": "#a6e3a1" })
            )
        );
        let result = tool(&mut s, "set_tab_color", json!({ "pane": 2 }));
        assert_eq!(result["isError"], json!(true), "the color is needed");
    }

    #[test]
    fn base64_of_bytes() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn a_screenshot_is_an_image() {
        let mut s = server();
        let result = tool(&mut s, "screenshot_pane", json!({ "pane": 4 }));
        assert_eq!(result["isError"], json!(false), "{result}");
        assert_eq!(
            s.backend.calls[0],
            ("screenshot".into(), json!({ "pane": 4 }))
        );
        let content = result["content"].as_array().unwrap();
        let image = content.iter().find(|c| c["type"] == "image").unwrap();
        assert_eq!(image["mimeType"], json!("image/png"));
        assert_eq!(image["data"], json!(base64(b"PNG!")));
        let text = content.iter().find(|c| c["type"] == "text").unwrap();
        assert!(
            text["text"].as_str().unwrap().contains("fterm-shot-4.png"),
            "{text}"
        );
    }

    #[test]
    fn the_ai_panel() {
        let mut s = server();
        let result = tool(&mut s, "ai_read", json!({ "last": 2 }));
        assert_eq!(s.backend.calls[0], ("ai_read".into(), json!({ "last": 2 })));
        assert_eq!(
            result["content"][0]["text"],
            json!("> what is ls?\nIt lists files.\n\n")
        );

        let result = tool(
            &mut s,
            "ai_ask",
            json!({ "text": "and on Windows?", "pane": 3, "timeout": 60 }),
        );
        assert_eq!(result["isError"], json!(false));
        assert_eq!(result["content"][0]["text"], json!("Use dir."));
        assert_eq!(
            s.backend.calls[1],
            (
                "ai_ask".into(),
                json!({ "text": "and on Windows?", "pane": 3, "wait": true, "timeout_ms": 60_000 })
            ),
            "the tool always waits for the answer"
        );

        let result = tool(&mut s, "ai_ask", json!({ "text": "fail" }));
        assert_eq!(result["isError"], json!(true));
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("no API key")
        );
        assert_eq!(tool(&mut s, "ai_ask", json!({}))["isError"], json!(true));
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

        // With colors: the styled lines as JSON, not only the text. Its own server, so the calls of
        // `s` keep their places.
        let mut colored = server();
        let result = tool(
            &mut colored,
            "read_pane",
            json!({ "pane": 1, "styled": true }),
        );
        assert_eq!(
            colored.backend.calls.last().unwrap().1,
            json!({ "pane": 1, "styled": true })
        );
        let text = result["content"][0]["text"].as_str().unwrap();
        let styled: Value = serde_json::from_str(text).unwrap();
        assert_eq!(styled["fg"], json!("#cdd6f4"));
        assert_eq!(styled["lines"][0][1]["fg"], json!("#f38ba8"));
        assert!(
            styled.get("text").is_none(),
            "the text is in the lines already"
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
