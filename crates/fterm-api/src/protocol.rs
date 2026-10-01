//! JSON-RPC 2.0 messages. One message is one line of JSON.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The version of the fterm API. New methods and new optional params do not change it.
pub const API_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    #[serde(default = "two")]
    pub jsonrpc: String,
    /// `None` = a notification (no answer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// A message from the server without a request (an event).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}

impl std::error::Error for RpcError {}

fn two() -> String {
    "2.0".to_owned()
}

impl RpcError {
    pub const PARSE: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const NO_METHOD: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL: i64 = -32603;
    /// fterm: the user (or the config) did not allow it.
    pub const DENIED: i64 = -32001;
    /// fterm: there is no such pane or tab.
    pub const NOT_FOUND: i64 = -32002;
    /// fterm: `wait_for` waited too long.
    pub const TIMEOUT: i64 = -32003;

    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn no_method(method: &str) -> Self {
        Self::new(Self::NO_METHOD, format!("no method `{method}`"))
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(Self::INVALID_PARAMS, message)
    }
}

impl Request {
    pub fn new(id: u64, method: &str, params: Value) -> Self {
        Self {
            jsonrpc: two(),
            id: Some(Value::from(id)),
            method: method.to_owned(),
            params,
        }
    }
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: two(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, error: RpcError) -> Self {
        Self {
            jsonrpc: two(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

impl Notification {
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            jsonrpc: two(),
            method: method.to_owned(),
            params,
        }
    }
}

/// Reads one request line. A bad line gives the JSON-RPC error to send back.
pub fn parse_request(line: &str) -> Result<Request, RpcError> {
    let value: serde_json::Value = serde_json::from_str(line)
        .map_err(|err| RpcError::new(RpcError::PARSE, err.to_string()))?;
    if !value.is_object() || value.get("method").is_none_or(|m| !m.is_string()) {
        return Err(RpcError::new(
            RpcError::INVALID_REQUEST,
            "a request is an object with a `method` string",
        ));
    }
    serde_json::from_value(value)
        .map_err(|err| RpcError::new(RpcError::INVALID_REQUEST, err.to_string()))
}

/// One message as a line (with `\n` at the end).
pub fn encode(message: &impl Serialize) -> String {
    let mut line = serde_json::to_string(message).unwrap_or_else(|_| "null".to_owned());
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_request_line() {
        let r =
            parse_request(r#"{"jsonrpc":"2.0","id":7,"method":"list","params":{"a":1}}"#).unwrap();
        assert_eq!(r.id, Some(json!(7)));
        assert_eq!(r.method, "list");
        assert_eq!(r.params, json!({"a": 1}));
        // No params, no jsonrpc field: still fine.
        let r = parse_request(r#"{"id":"x","method":"hello"}"#).unwrap();
        assert_eq!(r.params, Value::Null);
        // No id: a notification.
        assert_eq!(parse_request(r#"{"method":"ping"}"#).unwrap().id, None);
    }

    #[test]
    fn bad_lines() {
        assert_eq!(parse_request("not json").unwrap_err().code, RpcError::PARSE);
        assert_eq!(
            parse_request(r#"{"id":1}"#).unwrap_err().code,
            RpcError::INVALID_REQUEST
        );
        assert_eq!(
            parse_request("[1,2]").unwrap_err().code,
            RpcError::INVALID_REQUEST
        );
    }

    #[test]
    fn encode_is_one_line() {
        let line = encode(&Response::ok(json!(1), json!({"text": "a\nb"})));
        assert!(line.ends_with('\n'));
        assert_eq!(
            line.matches('\n').count(),
            1,
            "new lines in strings are escaped"
        );
        let back: Response = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(back.result, Some(json!({"text": "a\nb"})));
        assert!(!line.contains("error"), "no error field in a good answer");
    }
}
