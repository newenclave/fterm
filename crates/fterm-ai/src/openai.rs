//! OpenAI chat completions: `POST {url}/chat/completions` with `stream: true`.
//! OpenAI, OpenRouter, Ollama (`/v1`), and LM Studio speak it.

use serde_json::{Value, json};

use crate::sse::SseEvent;
use crate::{Chat, Event};

/// The JSON body of a request. The system text is the first message.
pub fn body(chat: &Chat, model: &str) -> Value {
    let mut messages = Vec::new();
    if !chat.system.trim().is_empty() {
        messages.push(json!({ "role": "system", "content": chat.system }));
    }
    for m in &chat.messages {
        messages.push(json!(m));
    }
    json!({
        "model": model,
        "max_tokens": chat.max_tokens,
        "stream": true,
        "stream_options": { "include_usage": true },
        "messages": messages,
    })
}

pub fn headers(key: Option<&str>) -> Vec<(&'static str, String)> {
    key.map(|k| vec![("authorization", format!("Bearer {k}"))])
        .unwrap_or_default()
}

#[derive(Default)]
pub struct Reader {
    stop_reason: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl Reader {
    /// The stream ended with no `[DONE]`: after a `finish_reason` the answer is complete anyway.
    pub fn at_end(&mut self) -> Option<Event> {
        self.stop_reason.is_some().then(|| Event::Done {
            stop_reason: self.stop_reason.take(),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        })
    }

    pub fn event(&mut self, event: &SseEvent) -> Option<Event> {
        if event.data.trim() == "[DONE]" {
            return Some(Event::Done {
                stop_reason: self.stop_reason.take(),
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
            });
        }
        let value: Value = serde_json::from_str(&event.data).ok()?;
        // `{"error":{"message":..}}`, or `{"error":".."}` (Ollama).
        let error = value["error"]["message"]
            .as_str()
            .or_else(|| value["error"].as_str());
        if let Some(message) = error {
            return Some(Event::Failed(crate::AiError::Http {
                status: 500,
                message: message.to_owned(),
            }));
        }
        if let Some(usage) = value.get("usage").filter(|u| u.is_object()) {
            self.input_tokens = usage["prompt_tokens"].as_u64();
            self.output_tokens = usage["completion_tokens"].as_u64();
        }
        let choice = &value["choices"][0];
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.stop_reason = Some(reason.to_owned());
        }
        let text = choice["delta"]["content"].as_str()?;
        (!text.is_empty()).then(|| Event::Delta(text.to_owned()))
    }
}

/// The message of an error body (`{"error":{"message":..}}`).
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

    fn data(text: &str) -> SseEvent {
        SseEvent {
            event: None,
            data: text.into(),
        }
    }

    #[test]
    fn the_request_body() {
        let chat = Chat {
            system: "Be short.".into(),
            messages: vec![Message {
                role: Role::User,
                content: "hi".into(),
            }],
            max_tokens: 500,
        };
        assert_eq!(
            body(&chat, "llama3.2"),
            json!({
                "model": "llama3.2",
                "max_tokens": 500,
                "stream": true,
                "stream_options": { "include_usage": true },
                "messages": [
                    { "role": "system", "content": "Be short." },
                    { "role": "user", "content": "hi" },
                ],
            })
        );
    }

    #[test]
    fn a_stream() {
        let mut r = Reader::default();
        assert_eq!(
            r.event(&data(
                r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#
            )),
            None,
            "an empty piece is nothing"
        );
        assert_eq!(
            r.event(&data(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#)),
            Some(Event::Delta("Hi".into()))
        );
        assert_eq!(
            r.event(&data(
                r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#
            )),
            None
        );
        assert_eq!(
            r.event(&data(
                r#"{"choices":[],"usage":{"prompt_tokens":9,"completion_tokens":3}}"#
            )),
            None
        );
        assert_eq!(
            r.event(&data("[DONE]")),
            Some(Event::Done {
                stop_reason: Some("stop".into()),
                input_tokens: Some(9),
                output_tokens: Some(3),
            })
        );
    }

    #[test]
    fn error_bodies() {
        assert_eq!(
            error_message(r#"{"error":{"message":"model not found"}}"#),
            "model not found"
        );
        assert_eq!(error_message("oops"), "oops");
    }
}
