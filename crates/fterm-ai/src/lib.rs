//! AI providers for fterm (Phase 8).
//!
//! Two protocols cover all providers:
//! - the Anthropic Messages API (Claude);
//! - OpenAI chat completions (OpenAI, OpenRouter, Ollama `/v1`, LM Studio, ...).
//!
//! A request runs on its own thread ([`stream::run`]) and gives [`Event`]s: pieces of text while
//! the answer streams, then `Done` or `Failed`. It can be stopped at any time.

pub mod anthropic;
pub mod keys;
pub mod openai;
pub mod sse;
pub mod stream;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// One conversation to send.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chat {
    pub system: String,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Anthropic,
    /// OpenAI chat completions.
    OpenAi,
}

/// One provider from the config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    /// The name in the config (`anthropic`, `ollama`, ...). Also the name of its key in the key store.
    pub name: String,
    pub kind: Kind,
    /// The base URL, for example `https://api.anthropic.com/v1` or `http://localhost:11434/v1`.
    pub url: String,
    pub model: String,
    /// The env var with the key (for example `ANTHROPIC_API_KEY`). `None` = the default for the kind.
    pub key_env: Option<String>,
    /// A local server (Ollama) needs no key.
    pub needs_key: bool,
}

impl Provider {
    /// Anthropic with the default model.
    pub fn anthropic(model: &str) -> Self {
        Self {
            name: "anthropic".into(),
            kind: Kind::Anthropic,
            url: "https://api.anthropic.com/v1".into(),
            model: model.into(),
            key_env: None,
            needs_key: true,
        }
    }

    /// The env var for the key.
    pub fn key_env(&self) -> String {
        match (&self.key_env, self.kind) {
            (Some(env), _) => env.clone(),
            (None, Kind::Anthropic) => "ANTHROPIC_API_KEY".into(),
            (None, Kind::OpenAi) => "OPENAI_API_KEY".into(),
        }
    }
}

/// What a running request gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A piece of the answer.
    Delta(String),
    /// The answer is complete.
    Done {
        stop_reason: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    Failed(AiError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiError {
    /// No key in the env var or the key store.
    NoKey { provider: String, env: String },
    /// The server said no (401 bad key, 429 too many requests, 500, ...).
    Http { status: u16, message: String },
    /// No connection, a broken stream, or a bad answer.
    Network(String),
    /// The user stopped it.
    Stopped,
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoKey { provider, env } => write!(
                f,
                "no API key for {provider}: set {env}, or run \"Set the AI key\" in the command palette"
            ),
            Self::Http {
                status: 401,
                message,
            } => write!(f, "the API key is not good (401): {message}"),
            Self::Http {
                status: 429,
                message,
            } => write!(f, "too many requests, try again soon (429): {message}"),
            Self::Http { status, message } => write!(f, "the server said {status}: {message}"),
            Self::Network(text) => write!(f, "network: {text}"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}
