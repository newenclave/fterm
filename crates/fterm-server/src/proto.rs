//! The control messages (JSON in `frame::CONTROL` frames).

use fterm_term::osc::OscEvent;
use fterm_term::size::GridSize;
use serde::{Deserialize, Serialize};

/// The version of the protocol. `Hello` tells it, so a window knows that the other side speaks it.
pub const VERSION: u32 = 1;

/// What to start in a new pane (like `fterm_term::session::SessionOptions`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spawn {
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    pub scrollback: usize,
}

/// A request from a window. `id` comes back in the answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Spawn {
        id: u64,
        spawn: Spawn,
        size: GridSize,
    },
    /// The snapshot comes first (a `SNAPSHOT` frame), then the answer with the `meta` and the `size` of the Term
    /// of the server (feed the snapshot into a Term of that size, then resize it), then the output.
    Attach {
        id: u64,
        pane: u64,
    },
    Detach {
        id: u64,
        pane: u64,
    },
    Resize {
        id: u64,
        pane: u64,
        size: GridSize,
    },
    /// Ends the program of the pane.
    Close {
        id: u64,
        pane: u64,
    },
    List {
        id: u64,
    },
    /// Keeps the layout of the panes of a window (tabs, splits, workspaces), for the next window.
    SetLayout {
        id: u64,
        layout: serde_json::Value,
    },
    GetLayout {
        id: u64,
    },
    /// Ends all panes and then the server. The answer says how many panes there were (`panes` has them).
    Stop {
        id: u64,
    },
    /// The first request over a connect command: the answer has the `version` of the server.
    Hello {
        id: u64,
        version: u32,
    },
}

impl Request {
    pub fn id(&self) -> u64 {
        match self {
            Self::Spawn { id, .. }
            | Self::Attach { id, .. }
            | Self::Detach { id, .. }
            | Self::Resize { id, .. }
            | Self::Close { id, .. }
            | Self::List { id }
            | Self::SetLayout { id, .. }
            | Self::GetLayout { id }
            | Self::Stop { id }
            | Self::Hello { id, .. } => *id,
        }
    }
}

/// What a pane knows that its output does not show again: the window gets it on attach.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub title: String,
    /// The last OSC event of each kind that matters later (the folder, the prompt marks, the agent state, the tab
    /// color, the command line), in the order they came.
    pub osc: Vec<OscEvent>,
}

impl Meta {
    /// Keeps `event` if it matters later; an older event of the same kind goes.
    pub fn note(&mut self, event: &OscEvent) {
        let kind = |e: &OscEvent| -> Option<u8> {
            Some(match e {
                OscEvent::Cwd(_) => 0,
                OscEvent::Prompt(_) => 1,
                OscEvent::Agent { .. } => 2,
                OscEvent::TabColor(_) => 3,
                OscEvent::CommandLine(_) => 4,
                OscEvent::InputStart { .. } => 5,
                OscEvent::OutputMark { .. } => 6,
                OscEvent::Notify { .. } => return None,
            })
        };
        let Some(k) = kind(event) else {
            return;
        };
        self.osc.retain(|e| kind(e) != Some(k));
        self.osc.push(event.clone());
    }
}

/// A pane of the server, for `List`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub pane: u64,
    pub program: String,
    /// A window shows it now.
    pub attached: bool,
    /// The programs that run in it (for the close question).
    pub running: Vec<String>,
}

/// An answer to a request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panes: Option<Vec<PaneInfo>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<GridSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
}

impl Reply {
    pub fn ok(id: u64) -> Self {
        Self {
            id,
            error: None,
            pane: None,
            program: None,
            meta: None,
            panes: None,
            layout: None,
            size: None,
            version: None,
        }
    }

    pub fn error(id: u64, error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Self::ok(id)
        }
    }
}

/// A message from the server: an answer, or an event of a pane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerMsg {
    Reply(Box<Reply>),
    /// The program of the pane ended (after its last output).
    Exit {
        pane: u64,
    },
}

impl ServerMsg {
    pub fn reply(reply: Reply) -> Self {
        Self::Reply(Box::new(reply))
    }
}

#[cfg(test)]
mod tests {
    use fterm_term::osc::PromptMark;

    use super::*;

    #[test]
    fn requests_and_answers_are_json() {
        let r = Request::Resize {
            id: 3,
            pane: 9,
            size: GridSize::new(80, 24),
        };
        let text = serde_json::to_string(&r).unwrap();
        assert!(text.contains(r#""op":"resize""#), "{text}");
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), r);
        assert_eq!(r.id(), 3);
        let m = ServerMsg::reply(Reply {
            pane: Some(9),
            ..Reply::ok(3)
        });
        let text = serde_json::to_string(&m).unwrap();
        assert!(!text.contains("error"), "no empty fields: {text}");
        assert_eq!(serde_json::from_str::<ServerMsg>(&text).unwrap(), m);
    }

    #[test]
    fn the_meta_keeps_the_last_event_of_each_kind() {
        let mut meta = Meta::default();
        meta.note(&OscEvent::Cwd("/a".into()));
        meta.note(&OscEvent::Prompt(PromptMark::PromptStart));
        meta.note(&OscEvent::Notify {
            title: None,
            body: "x".into(),
        });
        meta.note(&OscEvent::Cwd("/b".into()));
        meta.note(&OscEvent::Prompt(PromptMark::CommandStart));
        assert_eq!(
            meta.osc,
            [
                OscEvent::Cwd("/b".into()),
                OscEvent::Prompt(PromptMark::CommandStart)
            ]
        );
    }
}
