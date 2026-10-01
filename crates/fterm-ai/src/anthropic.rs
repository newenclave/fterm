//! The Anthropic Messages API: `POST {url}/messages` with `stream: true`.

use serde_json::{Value, json};

use crate::sse::SseEvent;
use crate::{AiError, Chat, Event};

pub const VERSION: &str = "2023-06-01";

/// The JSON body of a request.
pub fn body(chat: &Chat, model: &str) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": chat.max_tokens,
        "stream": true,
    });
    if !chat.system.trim().is_empty() {
        body["system"] = json!(chat.system);
    }
    body["messages"] = json!(chat.messages);
    body
}

/// The headers (without the content type).
pub fn headers(key: &str) -> Vec<(&'static str, String)> {
    vec![
        ("x-api-key", key.to_owned()),
        ("anthropic-version", VERSION.to_owned()),
    ]
}

/// Keeps the state between the events of one answer (the stop reason and the token counts).
#[derive(Default)]
pub struct Reader {
    stop_reason: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl Reader {
    /// One SSE event of the stream. `None` = nothing for the user (ping, starts, stops of blocks).
    pub fn event(&mut self, event: &SseEvent) -> Option<Event> {
        let value: Value = serde_json::from_str(&event.data).ok()?;
        match value["type"].as_str()? {
            "message_start" => {
                let usage = &value["message"]["usage"];
                self.input_tokens = usage["input_tokens"].as_u64();
                self.output_tokens = usage["output_tokens"].as_u64();
                None
            }
            "content_block_delta" => {
                let text = value["delta"]["text"].as_str()?;
                (!text.is_empty()).then(|| Event::Delta(text.to_owned()))
            }
            "message_delta" => {
                if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_owned());
                }
                if let Some(n) = value["usage"]["output_tokens"].as_u64() {
                    self.output_tokens = Some(n);
                }
                None
            }
            "message_stop" => Some(Event::Done {
                stop_reason: self.stop_reason.take(),
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
            }),
            "error" => {
                let message = value["error"]["message"]
                    .as_str()
                    .unwrap_or("unknown error");
                // Overloaded and API errors in the stream have no HTTP status; 529 is what Anthropic uses.
                let status = match value["error"]["type"].as_str() {
                    Some("overloaded_error") => 529,
                    Some("rate_limit_error") => 429,
                    _ => 500,
                };
                Some(Event::Failed(AiError::Http {
                    status,
                    message: message.to_owned(),
                }))
            }
            _ => None,
        }
    }
}

/// The message of an error body (`{"type":"error","error":{"type":..,"message":..}}`).
pub fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, Role};

    fn sse(event: &str, data: &str) -> SseEvent {
        SseEvent {
            event: Some(event.into()),
            data: data.into(),
        }
    }

    #[test]
    fn the_request_body() {
        let chat = Chat {
            system: "Be short.".into(),
            messages: vec![
                Message {
                    role: Role::User,
                    content: "hi".into(),
                },
                Message {
                    role: Role::Assistant,
                    content: "hello".into(),
                },
                Message {
                    role: Role::User,
                    content: "why?".into(),
                },
            ],
            max_tokens: 1000,
        };
        assert_eq!(
            body(&chat, "claude-haiku-4-5-20251001"),
            json!({
                "model": "claude-haiku-4-5-20251001",
                "max_tokens": 1000,
                "stream": true,
                "system": "Be short.",
                "messages": [
                    { "role": "user", "content": "hi" },
                    { "role": "assistant", "content": "hello" },
                    { "role": "user", "content": "why?" },
                ],
            })
        );
        let no_system = Chat {
            system: String::new(),
            ..chat
        };
        assert!(
            body(&no_system, "m").get("system").is_none(),
            "no empty system"
        );
    }

    #[test]
    fn a_stream() {
        let mut r = Reader::default();
        let start = sse(
            "message_start",
            r#"{"type":"message_start","message":{"usage":{"input_tokens":25,"output_tokens":1}}}"#,
        );
        assert_eq!(r.event(&start), None);
        assert_eq!(r.event(&sse("ping", r#"{"type":"ping"}"#)), None);
        assert_eq!(
            r.event(&sse("content_block_delta", r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#)),
            Some(Event::Delta("Hel".into()))
        );
        assert_eq!(
            r.event(&sse("message_delta", r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":12}}"#)),
            None
        );
        assert_eq!(
            r.event(&sse("message_stop", r#"{"type":"message_stop"}"#)),
            Some(Event::Done {
                stop_reason: Some("end_turn".into()),
                input_tokens: Some(25),
                output_tokens: Some(12),
            })
        );
    }

    #[test]
    fn an_error_in_the_stream() {
        let mut r = Reader::default();
        let e = sse(
            "error",
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        );
        assert_eq!(
            r.event(&e),
            Some(Event::Failed(AiError::Http {
                status: 529,
                message: "Overloaded".into()
            }))
        );
    }

    #[test]
    fn error_bodies() {
        assert_eq!(
            error_message(
                r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#
            ),
            "invalid x-api-key"
        );
        assert_eq!(
            error_message("Bad Gateway"),
            "Bad Gateway",
            "not JSON: the text"
        );
    }
}
