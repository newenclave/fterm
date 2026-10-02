//! `stream::run` against a small local HTTP server (no network, no key).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use fterm_ai::stream::run;
use fterm_ai::{AiError, Chat, Event, Kind, Message, Provider, Role};

/// What the server got: the request line, the headers (lower case), and the body.
struct Got {
    line: String,
    headers: Vec<(String, String)>,
    body: String,
}

/// Starts a server that answers one request with `status`, `content_type`, and `chunks` (sent with a pause).
fn server(
    status: u16,
    content_type: &str,
    chunks: Vec<String>,
) -> (String, std::thread::JoinHandle<Got>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let content_type = content_type.to_owned();
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut headers = Vec::new();
        loop {
            let mut h = String::new();
            reader.read_line(&mut h).unwrap();
            let h = h.trim_end().to_owned();
            if h.is_empty() {
                break;
            }
            let (k, v) = h.split_once(':').unwrap();
            headers.push((k.trim().to_lowercase(), v.trim().to_owned()));
        }
        let length: usize = headers
            .iter()
            .find(|(k, _)| k == "content-length")
            .map_or(0, |(_, v)| v.parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let mut out = stream;
        let head = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n"
        );
        out.write_all(head.as_bytes()).unwrap();
        for chunk in chunks {
            if out.write_all(chunk.as_bytes()).is_err() {
                break;
            }
            let _ = out.flush();
            std::thread::sleep(Duration::from_millis(30));
        }
        Got {
            line: line.trim_end().to_owned(),
            headers,
            body: String::from_utf8(body).unwrap(),
        }
    });
    (url, handle)
}

fn chat() -> Chat {
    Chat {
        system: "Be short.".into(),
        messages: vec![Message {
            role: Role::User,
            content: "hi".into(),
        }],
        max_tokens: 100,
    }
}

fn provider(kind: Kind, url: &str) -> Provider {
    Provider {
        name: "test".into(),
        kind,
        url: url.into(),
        model: "test-model".into(),
        key_env: None,
        // Like Ollama: an OpenAI-like local server needs no key.
        needs_key: kind == Kind::Anthropic,
    }
}

fn collect(p: &Provider, key: Option<&str>, stop: &Arc<AtomicBool>) -> Vec<Event> {
    let mut events = Vec::new();
    run(p, key, &chat(), stop, &mut |e| events.push(e));
    events
}

fn header<'a>(got: &'a Got, name: &str) -> Option<&'a str> {
    got.headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

#[test]
fn anthropic_streams_an_answer() {
    let chunks = vec![
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\n".to_owned(),
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n".to_owned(),
        // A chunk can end in the middle of a line.
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":".to_owned(),
        "{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n".to_owned(),
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n".to_owned(),
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_owned(),
    ];
    let (url, server) = server(200, "text/event-stream", chunks);
    let events = collect(
        &provider(Kind::Anthropic, &url),
        Some("sk-test"),
        &Arc::default(),
    );
    assert_eq!(
        events,
        [
            Event::Delta("Hel".into()),
            Event::Delta("lo".into()),
            Event::Done {
                stop_reason: Some("end_turn".into()),
                input_tokens: Some(5),
                output_tokens: Some(2),
            },
        ]
    );
    let got = server.join().unwrap();
    assert_eq!(got.line, "POST /v1/messages HTTP/1.1");
    assert_eq!(header(&got, "x-api-key"), Some("sk-test"));
    assert_eq!(header(&got, "anthropic-version"), Some("2023-06-01"));
    let sent: serde_json::Value = serde_json::from_str(&got.body).unwrap();
    assert_eq!(sent["stream"], serde_json::json!(true));
    assert_eq!(sent["model"], serde_json::json!("test-model"));
}

#[test]
fn openai_streams_an_answer() {
    let chunks = vec![
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n".to_owned(),
        "data: {\"choices\":[{\"delta\":{\"content\":\"!\"},\"finish_reason\":\"stop\"}]}\n\n"
            .to_owned(),
        "data: [DONE]\n\n".to_owned(),
    ];
    let (url, server) = server(200, "text/event-stream", chunks);
    let events = collect(&provider(Kind::OpenAi, &url), Some("sk-o"), &Arc::default());
    assert_eq!(events[0], Event::Delta("Hi".into()));
    assert_eq!(events[1], Event::Delta("!".into()));
    assert!(matches!(&events[2], Event::Done { stop_reason: Some(r), .. } if r == "stop"));
    let got = server.join().unwrap();
    assert_eq!(got.line, "POST /v1/chat/completions HTTP/1.1");
    assert_eq!(header(&got, "authorization"), Some("Bearer sk-o"));
}

#[test]
fn a_bad_key_is_a_clear_error() {
    let body = "{\"type\":\"error\",\"error\":{\"type\":\"authentication_error\",\"message\":\"invalid x-api-key\"}}";
    let (url, _server) = server(401, "application/json", vec![body.to_owned()]);
    let events = collect(
        &provider(Kind::Anthropic, &url),
        Some("bad"),
        &Arc::default(),
    );
    assert_eq!(
        events,
        [Event::Failed(AiError::Http {
            status: 401,
            message: "invalid x-api-key".into()
        })]
    );
}

#[test]
fn a_server_error_and_no_server() {
    let (url, _server) = server(500, "text/plain", vec!["boom".to_owned()]);
    let events = collect(&provider(Kind::OpenAi, &url), None, &Arc::default());
    assert_eq!(
        events,
        [Event::Failed(AiError::Http {
            status: 500,
            message: "boom".into()
        })]
    );
    // Nobody listens on this port.
    let free = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let events = collect(
        &provider(Kind::OpenAi, &format!("http://{free}/v1")),
        None,
        &Arc::default(),
    );
    assert!(
        matches!(events.as_slice(), [Event::Failed(AiError::Network(_))]),
        "{events:?}"
    );
}

#[test]
fn stop_ends_a_running_answer() {
    let mut chunks = Vec::new();
    for i in 0..100 {
        chunks.push(format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{i} \"}}}}]}}\n\n"
        ));
    }
    let (url, _server) = server(200, "text/event-stream", chunks);
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let mut events = Vec::new();
    run(
        &provider(Kind::OpenAi, &url),
        None,
        &chat(),
        &stop,
        &mut |e| {
            if matches!(e, Event::Delta(_)) && events.len() == 2 {
                flag.store(true, Ordering::SeqCst);
            }
            events.push(e);
        },
    );
    assert!(
        events.len() < 10,
        "it stopped early: {} events",
        events.len()
    );
    assert_eq!(events.last(), Some(&Event::Failed(AiError::Stopped)));
}

#[test]
fn no_key_no_request() {
    let p = Provider {
        key_env: Some("FTERM_TEST_NO_SUCH_KEY".into()),
        ..provider(Kind::Anthropic, "http://127.0.0.1:9/v1")
    };
    let events = collect(&p, None, &Arc::default());
    assert_eq!(
        events,
        [Event::Failed(AiError::NoKey {
            provider: "test".into(),
            env: "FTERM_TEST_NO_SUCH_KEY".into()
        })]
    );
}

#[test]
fn an_answer_with_an_end_reason_is_done_without_done() {
    // Some servers close the stream after `finish_reason` and send no `[DONE]`.
    let chunks = vec![
        "data: {\"choices\":[{\"delta\":{\"content\":\"ls\"}}]}\n\n".to_owned(),
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n".to_owned(),
    ];
    let (url, _server) = server(200, "text/event-stream", chunks);
    let events = collect(&provider(Kind::OpenAi, &url), None, &Arc::default());
    assert_eq!(events[0], Event::Delta("ls".into()));
    assert!(
        matches!(&events[1], Event::Done { stop_reason: Some(r), .. } if r == "stop"),
        "{events:?}"
    );
}

#[test]
fn an_error_in_the_stream_says_what_it_is() {
    // Ollama writes an error as a string, not as an object.
    let chunks = vec![
        "data: {\"choices\":[{\"delta\":{\"content\":\"Get\"}}]}\n\n".to_owned(),
        "data: {\"error\":\"model runner has unexpectedly stopped\"}\n\n".to_owned(),
    ];
    let (url, _server) = server(200, "text/event-stream", chunks);
    let events = collect(&provider(Kind::OpenAi, &url), None, &Arc::default());
    let last = events.last().unwrap();
    assert!(
        matches!(last, Event::Failed(AiError::Http { message, .. }) if message.contains("runner has unexpectedly stopped")),
        "{events:?}"
    );
}

#[test]
fn a_broken_stream_says_how_much_came() {
    let chunks = vec!["data: {\"choices\":[{\"delta\":{\"content\":\"Get-Chi\"}}]}\n\n".to_owned()];
    let (url, _server) = server(200, "text/event-stream", chunks);
    let events = collect(&provider(Kind::OpenAi, &url), None, &Arc::default());
    let Some(Event::Failed(AiError::Network(message))) = events.last() else {
        panic!("{events:?}");
    };
    assert!(message.contains("7 characters"), "{message}");
    assert!(message.contains("still running"), "{message}");
}
