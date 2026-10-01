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
];

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

/// How long a client thread waits for the app. `wait_for` waits as long as it asks (at most one hour).
pub fn timeout_for(method: &str, params: &Value) -> Duration {
    const BASE: Duration = Duration::from_secs(30);
    const EXTRA: Duration = Duration::from_secs(5);
    if method != "wait_for" {
        return BASE;
    }
    let asked = params
        .get("timeout_ms")
        .and_then(Value::as_f64)
        .map_or(BASE, |ms| {
            Duration::from_millis(ms.clamp(0.0, 3_600_000.0) as u64)
        });
    asked + EXTRA
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn timeouts() {
        assert_eq!(timeout_for("list", &Value::Null), Duration::from_secs(30));
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
