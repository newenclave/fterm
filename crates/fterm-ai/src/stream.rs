//! Run one request and read the streamed answer (on the calling thread; start it on a new one).

use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::sse::Parser;
use crate::{AiError, Chat, Event, Kind, Provider, anthropic, openai};

/// Sends `chat` to the provider and gives the events to `on_event` until the answer is complete,
/// fails, or `stop` becomes true (then the last event is `Failed(Stopped)`).
/// `key` is `None` for providers that need no key.
pub fn run(
    provider: &Provider,
    key: Option<&str>,
    chat: &Chat,
    stop: &Arc<AtomicBool>,
    on_event: &mut dyn FnMut(Event),
) {
    if provider.needs_key && key.is_none() {
        return on_event(Event::Failed(AiError::NoKey {
            provider: provider.name.clone(),
            env: provider.key_env(),
        }));
    }
    let base = provider.url.trim_end_matches('/');
    let (url, body, headers) = match provider.kind {
        Kind::Anthropic => (
            format!("{base}/messages"),
            anthropic::body(chat, &provider.model),
            anthropic::headers(key.unwrap_or_default()),
        ),
        Kind::OpenAi => (
            format!("{base}/chat/completions"),
            openai::body(chat, &provider.model),
            openai::headers(key),
        ),
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut request = agent.post(&url);
    for (name, value) in &headers {
        request = request.header(*name, value);
    }
    let response = match request.send_json(&body) {
        Ok(response) => response,
        Err(err) => return on_event(Event::Failed(AiError::Network(err.to_string()))),
    };
    let status = response.status().as_u16();
    let mut reader = BufReader::new(response.into_body().into_reader());
    if !(200..300).contains(&status) {
        let mut text = String::new();
        let _ = reader.take(64 * 1024).read_to_string(&mut text);
        let message = match provider.kind {
            Kind::Anthropic => anthropic::error_message(&text),
            Kind::OpenAi => openai::error_message(&text),
        };
        return on_event(Event::Failed(AiError::Http { status, message }));
    }
    let mut parser = Parser::default();
    let mut anthropic = anthropic::Reader::default();
    let mut openai = openai::Reader::default();
    let mut line = String::new();
    // How much of the answer came (for the message when the stream breaks).
    let mut received = 0usize;
    loop {
        if stop.load(Ordering::SeqCst) {
            return on_event(Event::Failed(AiError::Stopped));
        }
        line.clear();
        let event = match reader.read_line(&mut line) {
            Ok(0) => parser.finish(),
            Ok(_) => parser.line(line.trim_end_matches('\n')),
            Err(err) => return on_event(Event::Failed(AiError::Network(err.to_string()))),
        };
        let at_end = line.is_empty();
        if let Some(event) = event {
            let out = match provider.kind {
                Kind::Anthropic => anthropic.event(&event),
                Kind::OpenAi => openai.event(&event),
            };
            if let Some(out) = out {
                if let Event::Delta(text) = &out {
                    received += text.chars().count();
                }
                let last = matches!(out, Event::Done { .. } | Event::Failed(_));
                on_event(out);
                if last {
                    return;
                }
            }
        }
        if at_end {
            if let Kind::OpenAi = provider.kind
                && let Some(done) = openai.at_end()
            {
                return on_event(done);
            }
            return on_event(Event::Failed(AiError::Network(format!(
                "the stream ended before the answer was complete ({received} characters came, \
                 with no end mark). Is the server still running?"
            ))));
        }
    }
}
