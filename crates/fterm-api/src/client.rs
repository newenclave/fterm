//! A client: `fterm cli`, `fterm mcp`, and tests use it.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};

use interprocess::local_socket::{RecvHalf, SendHalf, prelude::*};
use serde_json::Value;

use crate::protocol::{Notification, Request, RpcError, encode};
use crate::transport::connect;

#[derive(Debug)]
pub enum ClientError {
    Io(io::Error),
    /// The server answered with an error.
    Rpc(RpcError),
    /// The server sent something that is not JSON-RPC.
    Protocol(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Rpc(err) => write!(f, "{err}"),
            Self::Protocol(text) => write!(f, "bad answer from fterm: {text}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<io::Error> for ClientError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

pub struct Client {
    reader: BufReader<RecvHalf>,
    writer: SendHalf,
    next_id: u64,
    /// Events that came while we waited for an answer.
    events: VecDeque<Notification>,
}

impl Client {
    fn read_message(&mut self) -> Result<Value, ClientError> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                return Err(ClientError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "fterm closed the connection",
                )));
            }
            if !line.trim().is_empty() {
                return serde_json::from_str(&line)
                    .map_err(|err| ClientError::Protocol(err.to_string()));
            }
        }
    }

    pub fn connect(name: &str) -> io::Result<Self> {
        let (recv, send) = connect(name)?.split();
        Ok(Self {
            reader: BufReader::new(recv),
            writer: send,
            next_id: 1,
            events: VecDeque::new(),
        })
    }

    /// Sends a request and waits for its answer. Events that come before it are kept for `next_event`.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        self.writer
            .write_all(encode(&Request::new(id, method, params)).as_bytes())?;
        self.writer.flush()?;
        loop {
            let value = self.read_message()?;
            if value.get("method").is_some() && value.get("id").is_none() {
                let event = serde_json::from_value(value)
                    .map_err(|err| ClientError::Protocol(err.to_string()))?;
                self.events.push_back(event);
                continue;
            }
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                // An answer to a bad line, or to an old request: not ours.
                continue;
            }
            if let Some(error) = value.get("error") {
                let error: RpcError = serde_json::from_value(error.clone())
                    .map_err(|err| ClientError::Protocol(err.to_string()))?;
                return Err(ClientError::Rpc(error));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// The next event (after `subscribe`). It blocks until one comes.
    pub fn next_event(&mut self) -> Result<Notification, ClientError> {
        if let Some(event) = self.events.pop_front() {
            return Ok(event);
        }
        loop {
            let value = self.read_message()?;
            if value.get("method").is_some() && value.get("id").is_none() {
                return serde_json::from_value(value)
                    .map_err(|err| ClientError::Protocol(err.to_string()));
            }
        }
    }
}
