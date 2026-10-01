//! Ask a provider one question from the command line (to try a key or a local server):
//!
//! ```text
//! cargo run -p fterm-ai --example ask -- "Why is the sky blue?"
//! cargo run -p fterm-ai --example ask -- --ollama llama3.2 "Hello"
//! ```

use std::io::Write;
use std::sync::Arc;

use fterm_ai::{Chat, Event, Kind, Message, Provider, Role, keys, stream};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let provider = if args.first().map(String::as_str) == Some("--ollama") {
        args.remove(0);
        let model = if args.is_empty() {
            "llama3.2".to_owned()
        } else {
            args.remove(0)
        };
        Provider {
            name: "ollama".into(),
            kind: Kind::OpenAi,
            url: "http://localhost:11434/v1".into(),
            model,
            key_env: None,
            needs_key: false,
        }
    } else {
        Provider::anthropic("claude-haiku-4-5-20251001")
    };
    let question = args.join(" ");
    let chat = Chat {
        system: "Answer in a few short sentences.".into(),
        messages: vec![Message {
            role: Role::User,
            content: if question.is_empty() {
                "Say hello.".into()
            } else {
                question
            },
        }],
        max_tokens: 300,
    };
    let key = keys::get(&provider);
    stream::run(
        &provider,
        key.as_deref(),
        &chat,
        &Arc::default(),
        &mut |event| match event {
            Event::Delta(text) => {
                print!("{text}");
                let _ = std::io::stdout().flush();
            }
            Event::Done { output_tokens, .. } => {
                println!("\n[done, {} tokens]", output_tokens.unwrap_or(0))
            }
            Event::Failed(err) => eprintln!("\n[error] {err}"),
        },
    );
}
