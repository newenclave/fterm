//! The bridge between the API server (its client threads) and the app (the event loop),
//! and small helpers for the params of the methods.

use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use fterm_api::RpcError;
use fterm_api::server::{ClientId, Handler};
use fterm_config::keys::SpawnWhere;
use serde_json::Value;
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

/// All methods (for `hello`). `subscribe` and `unsubscribe` are in the server.
pub const METHODS: &[&str] = &[
    "hello",
    "list",
    "spawn",
    "send_text",
    "get_text",
    "focus",
    "close",
    "zoom",
    "set_title",
    "notify",
    "panel",
    "wait_for",
    "send_message",
    "read_messages",
    "scene_open",
    "scene_draw",
    "screenshot",
    "set_tab_color",
    "ai_read",
    "ai_ask",
    "ai_input",
    "ai_stop",
    "ai_clear",
    "subscribe",
    "unsubscribe",
];

/// The events that `subscribe` can ask for.
pub const EVENTS: &[&str] = &[
    "pane_opened",
    "pane_closed",
    "command_done",
    "agent_state",
    "notification",
    "cwd",
    "title",
    "message",
    "scene_resized",
    "ai_answer",
];

/// How long `ai_ask` with `wait` waits for the answer when it does not say (5 minutes).
pub const AI_TIMEOUT_MS: f64 = 300_000.0;

/// One request on its way to the app. The app sends the answer on `reply`.
#[derive(Debug)]
pub struct ApiRequest {
    pub client: ClientId,
    pub method: String,
    pub params: Value,
    pub reply: mpsc::Sender<Result<Value, RpcError>>,
}

/// The server calls this on the client threads. It passes each request to the event loop and waits.
pub struct Bridge {
    proxy: Mutex<EventLoopProxy<UserEvent>>,
}

impl Bridge {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            proxy: Mutex::new(proxy),
        }
    }
}

impl Handler for Bridge {
    fn call(&self, client: ClientId, method: &str, params: Value) -> Result<Value, RpcError> {
        let wait = timeout_for(method, &params);
        let (reply, answer) = mpsc::channel();
        let request = ApiRequest {
            client,
            method: method.to_owned(),
            params,
            reply,
        };
        let sent = self
            .proxy
            .lock()
            .map_err(|_| RpcError::new(RpcError::INTERNAL, "the app is gone"))?
            .send_event(UserEvent::Api(request));
        if sent.is_err() {
            return Err(RpcError::new(RpcError::INTERNAL, "the window is closed"));
        }
        answer.recv_timeout(wait).unwrap_or_else(|_| {
            Err(RpcError::new(
                RpcError::TIMEOUT,
                format!("no answer from fterm in {} s", wait.as_secs()),
            ))
        })
    }

    fn disconnected(&self, client: ClientId) {
        if let Ok(proxy) = self.proxy.lock() {
            let _ = proxy.send_event(UserEvent::ApiGone(client));
        }
    }
}

/// How long a client thread waits for the app. `wait_for` and `ai_ask` with `wait` wait as long as
/// they ask (at most one hour).
pub fn timeout_for(method: &str, params: &Value) -> Duration {
    // Long enough for the user to answer the access question.
    const BASE: Duration = Duration::from_secs(120);
    const WAIT_FOR: Duration = Duration::from_secs(30);
    const EXTRA: Duration = Duration::from_secs(5);
    let asked = |default: f64| {
        let ms = params.get("timeout_ms").and_then(Value::as_f64);
        Duration::from_millis(ms.unwrap_or(default).clamp(0.0, 3_600_000.0) as u64)
    };
    if method == "ai_ask" && params.get("wait").and_then(Value::as_bool) == Some(true) {
        // The access question can come first, then the answer.
        return BASE + asked(AI_TIMEOUT_MS);
    }
    if method != "wait_for" {
        return BASE;
    }
    asked(WAIT_FOR.as_millis() as f64) + EXTRA
}

/// An optional pane id: `{"pane": 3}`.
pub fn pane_param(params: &Value, key: &str) -> Result<Option<u64>, RpcError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            RpcError::invalid_params(format!("`{key}` must be a pane id (a number)"))
        }),
    }
}

pub fn str_param(params: &Value, key: &str) -> Result<Option<String>, RpcError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(RpcError::invalid_params(format!(
            "`{key}` must be a string"
        ))),
    }
}

pub fn bool_param(params: &Value, key: &str) -> Result<Option<bool>, RpcError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(on)) => Ok(Some(*on)),
        Some(_) => Err(RpcError::invalid_params(format!(
            "`{key}` must be true or false"
        ))),
    }
}

/// `place`: "tab" (the default), "right", or "down".
pub fn place_param(params: &Value) -> Result<SpawnWhere, RpcError> {
    match str_param(params, "place")?.as_deref() {
        None | Some("tab") => Ok(SpawnWhere::Tab),
        Some("right") => Ok(SpawnWhere::SplitRight),
        Some("down") => Ok(SpawnWhere::SplitDown),
        Some(other) => Err(RpcError::invalid_params(format!(
            "`place` must be \"tab\", \"right\", or \"down\", got `{other}`"
        ))),
    }
}

/// `color` of `set_tab_color`: `#rrggbb` or `#rgb`, or `"none"` / `null` for no color.
pub fn tab_color_param(params: &Value) -> Result<Option<[u8; 3]>, RpcError> {
    match params.get("color") {
        None => Err(RpcError::invalid_params(
            "give `color`: #rrggbb, or \"none\" for no color",
        )),
        Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            fterm_config::colors::tab_color(text).map_err(RpcError::invalid_params)
        }
        Some(_) => Err(RpcError::invalid_params(
            "`color` must be a string like \"#f38ba8\"",
        )),
    }
}

/// Where a screenshot of `pane` goes: `path` (a .png file), else a file in the temp folder.
pub fn shot_path(
    params: &Value,
    pane: u64,
    unix_ms: u64,
    temp: &std::path::Path,
) -> Result<std::path::PathBuf, RpcError> {
    match str_param(params, "path")? {
        None => Ok(temp.join(format!("fterm-shot-{pane}-{unix_ms}.png"))),
        Some(path) if path.to_ascii_lowercase().ends_with(".png") => Ok(path.into()),
        Some(path) => Err(RpcError::invalid_params(format!(
            "a screenshot is a PNG file: `{path}` must end with .png"
        ))),
    }
}

/// Styled text for `get_text`: the default colors once, and runs that have only what is not
/// the default (so the answer stays small).
pub fn styled_json(
    lines: &[Vec<fterm_term::styled::Run>],
    fg: fterm_term::alacritty_terminal::vte::ansi::Rgb,
    bg: fterm_term::alacritty_terminal::vte::ansi::Rgb,
) -> Value {
    let hex = |c: fterm_term::alacritty_terminal::vte::ansi::Rgb| {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    };
    let lines: Vec<Value> = lines
        .iter()
        .map(|line| {
            line.iter()
                .map(|run| {
                    let s = &run.style;
                    let mut v = serde_json::json!({ "text": run.text });
                    if s.fg != fg {
                        v["fg"] = hex(s.fg).into();
                    }
                    if s.bg != bg {
                        v["bg"] = hex(s.bg).into();
                    }
                    for (key, on) in [
                        ("bold", s.bold),
                        ("italic", s.italic),
                        ("underline", s.underline),
                        ("strike", s.strike),
                        ("dim", s.dim),
                    ] {
                        if on {
                            v[key] = true.into();
                        }
                    }
                    v
                })
                .collect()
        })
        .collect();
    serde_json::json!({ "fg": hex(fg), "bg": hex(bg), "lines": lines })
}

/// `place` of a scene: "right" (the default) or "down". A scene is a split.
pub fn scene_place(params: &Value) -> Result<fterm_mux::Direction, RpcError> {
    match str_param(params, "place")?.as_deref() {
        None | Some("right") => Ok(fterm_mux::Direction::Right),
        Some("down") => Ok(fterm_mux::Direction::Down),
        Some(other) => Err(RpcError::invalid_params(format!(
            "a scene is a split: `place` must be \"right\" or \"down\", got `{other}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_tab_color() {
        assert_eq!(
            tab_color_param(&json!({"color": "#f38ba8"})).unwrap(),
            Some([0xf3, 0x8b, 0xa8])
        );
        assert_eq!(
            tab_color_param(&json!({"color": "#fa0"})).unwrap(),
            Some([0xff, 0xaa, 0x00])
        );
        // No color again.
        assert_eq!(tab_color_param(&json!({"color": "none"})).unwrap(), None);
        assert_eq!(tab_color_param(&json!({"color": null})).unwrap(), None);
        assert!(tab_color_param(&json!({})).is_err(), "say the color");
        assert!(tab_color_param(&json!({"color": "red"})).is_err());
        assert!(tab_color_param(&json!({"color": 5})).is_err());
    }

    #[test]
    fn where_a_screenshot_goes() {
        use std::path::{Path, PathBuf};
        let temp = Path::new("T:/temp");
        assert_eq!(
            shot_path(&json!({}), 3, 1_790_000_000_123, temp).unwrap(),
            temp.join("fterm-shot-3-1790000000123.png")
        );
        assert_eq!(
            shot_path(&json!({"path": "D:/shots/map.png"}), 3, 0, temp).unwrap(),
            PathBuf::from("D:/shots/map.png")
        );
        assert!(
            shot_path(&json!({"path": "D:/shots/map.jpg"}), 3, 0, temp).is_err(),
            "PNG only"
        );
        assert!(shot_path(&json!({"path": 5}), 3, 0, temp).is_err());
    }

    #[test]
    fn styled_text_as_json() {
        use fterm_term::alacritty_terminal::vte::ansi::Rgb;
        use fterm_term::styled::{Run, Style};
        let fg = Rgb {
            r: 0xcd,
            g: 0xd6,
            b: 0xf4,
        };
        let bg = Rgb {
            r: 0x1e,
            g: 0x1e,
            b: 0x2e,
        };
        let plain = Style {
            fg,
            bg,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            dim: false,
        };
        let red = Style {
            fg: Rgb {
                r: 0xf3,
                g: 0x8b,
                b: 0xa8,
            },
            bold: true,
            ..plain
        };
        let bar = Style {
            bg: Rgb { r: 0, g: 0, b: 255 },
            underline: true,
            ..plain
        };
        let run = |text: &str, style| Run {
            text: text.to_owned(),
            style,
        };
        let lines = vec![
            vec![run("ok ", plain), run("error", red)],
            vec![],
            vec![run("bar", bar)],
        ];
        // The default colors are said once; a run has only what is not the default.
        assert_eq!(
            styled_json(&lines, fg, bg),
            json!({
                "fg": "#cdd6f4",
                "bg": "#1e1e2e",
                "lines": [
                    [{"text": "ok "}, {"text": "error", "fg": "#f38ba8", "bold": true}],
                    [],
                    [{"text": "bar", "bg": "#0000ff", "underline": true}],
                ]
            })
        );
    }

    #[test]
    fn a_scene_is_a_split() {
        use fterm_mux::Direction;
        assert_eq!(scene_place(&json!({})).unwrap(), Direction::Right);
        assert_eq!(
            scene_place(&json!({"place": "right"})).unwrap(),
            Direction::Right
        );
        assert_eq!(
            scene_place(&json!({"place": "down"})).unwrap(),
            Direction::Down
        );
        let err = scene_place(&json!({"place": "tab"})).unwrap_err();
        assert!(
            err.message.contains("right") && err.message.contains("down"),
            "{}",
            err.message
        );
        assert!(scene_place(&json!({"place": 3})).is_err());
    }

    #[test]
    fn timeouts() {
        // Long enough for the user to answer the access question.
        assert_eq!(timeout_for("list", &Value::Null), Duration::from_secs(120));
        assert_eq!(
            timeout_for("wait_for", &json!({"timeout_ms": 60_000})),
            Duration::from_secs(65),
            "its own time and a bit more"
        );
        assert_eq!(timeout_for("wait_for", &json!({})), Duration::from_secs(35));
        assert_eq!(
            timeout_for("wait_for", &json!({"timeout_ms": 1e12})),
            Duration::from_secs(3605)
        );
        assert_eq!(
            timeout_for("ai_ask", &json!({"text": "hi"})),
            Duration::from_secs(120),
            "no wait: the answer is the id"
        );
        assert_eq!(
            timeout_for("ai_ask", &json!({"wait": true})),
            Duration::from_secs(420),
            "the access question, then 5 minutes for the answer"
        );
        assert_eq!(
            timeout_for("ai_ask", &json!({"wait": true, "timeout_ms": 10_000})),
            Duration::from_secs(130)
        );
    }

    #[test]
    fn params() {
        let p = json!({"pane": 3, "text": "ls", "enter": true, "bad": "x"});
        assert_eq!(pane_param(&p, "pane").unwrap(), Some(3));
        assert_eq!(pane_param(&p, "nope").unwrap(), None);
        assert_eq!(
            pane_param(&p, "bad").unwrap_err().code,
            RpcError::INVALID_PARAMS
        );
        assert_eq!(str_param(&p, "text").unwrap().as_deref(), Some("ls"));
        assert_eq!(
            str_param(&p, "pane").unwrap_err().code,
            RpcError::INVALID_PARAMS
        );
        assert_eq!(bool_param(&p, "enter").unwrap(), Some(true));
        assert_eq!(
            bool_param(&Value::Null, "enter").unwrap(),
            None,
            "no params at all"
        );
    }

    #[test]
    fn places() {
        assert_eq!(place_param(&json!({})).unwrap(), SpawnWhere::Tab);
        assert_eq!(
            place_param(&json!({"place": "right"})).unwrap(),
            SpawnWhere::SplitRight
        );
        assert_eq!(
            place_param(&json!({"place": "down"})).unwrap(),
            SpawnWhere::SplitDown
        );
        assert!(place_param(&json!({"place": "up"})).is_err());
    }
}
