//! What an agent in a pane did and does: its steps, its question, its todos, the files it changed, and what it
//! costs. Pure: the app gives it the events (from the hooks and the status line of Claude Code) and draws it.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

/// The most steps an agent keeps.
pub const MAX_STEPS: usize = 50;
/// The longest text of a step, a question, or a message (chars).
pub const MAX_TEXT: usize = 200;

/// A todo of the agent (Claude Code's TodoWrite).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Todo {
    pub content: String,
    pub status: TodoStatus,
    /// What the agent says while it does it ("Writing tests").
    pub active: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

impl TodoStatus {
    pub fn from_name(name: &str) -> Self {
        match name {
            "completed" => Self::Completed,
            "in_progress" => Self::InProgress,
            _ => Self::Pending,
        }
    }
}

/// What the status line of Claude Code tells: money, lines, context, model, limits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub cost_usd: Option<f64>,
    pub lines_added: Option<u64>,
    pub lines_removed: Option<u64>,
    /// How much of the context window is used, in percent.
    pub context_pct: Option<f64>,
    pub model: Option<String>,
    /// The 5-hour and 7-day limits of a subscription, used in percent.
    pub five_hour_pct: Option<f64>,
    pub seven_day_pct: Option<f64>,
}

/// One thing that the agent did or said.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    /// The user sent a prompt.
    Prompt(String),
    /// The agent used a tool: its name, the file it changed or read, the command it ran, the todos it wrote.
    Tool {
        name: String,
        file: Option<String>,
        command: Option<String>,
        todos: Option<Vec<Todo>>,
    },
    /// The agent waits for the user (a permission question, a form).
    Question(String),
    /// The agent finished its answer (its last message).
    Done(Option<String>),
    /// New numbers from the status line.
    Usage(Usage),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Prompt,
    Tool,
    Question,
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub at: Instant,
    pub kind: StepKind,
    pub text: String,
}

/// Everything fterm knows about the agent of a pane.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentInfo {
    /// The newest last.
    pub timeline: VecDeque<Step>,
    /// What the agent asks now (it waits for the user).
    pub question: Option<String>,
    pub todos: Vec<Todo>,
    /// The files it changed, and how many times.
    pub files: BTreeMap<String, u32>,
    pub usage: Option<Usage>,
    /// The first prompt of the conversation (its title in the history).
    pub first_prompt: Option<String>,
    pub last_message: Option<String>,
}

/// One line with no control chars, at most `max` chars (an ellipsis when it was longer).
pub fn clean(text: &str, max: usize) -> String {
    let one: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    let one = one.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut cut: String = one.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// An event from the params of the API method `agent_event` (made by `ftermctl agent-event --hook`).
pub fn event_from_json(params: &serde_json::Value) -> Option<AgentEvent> {
    let text = |key: &str| params.get(key).and_then(|v| v.as_str()).map(str::to_owned);
    Some(match params.get("kind")?.as_str()? {
        "prompt" => AgentEvent::Prompt(text("text").unwrap_or_default()),
        "tool" => AgentEvent::Tool {
            name: text("tool")?,
            file: text("file"),
            command: text("command"),
            todos: params.get("todos").and_then(|t| t.as_array()).map(|todos| {
                todos
                    .iter()
                    .map(|t| {
                        let field = |k: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or("");
                        Todo {
                            content: clean(field("content"), MAX_TEXT),
                            status: TodoStatus::from_name(field("status")),
                            active: clean(field("active"), MAX_TEXT),
                        }
                    })
                    .collect()
            }),
        },
        "question" => AgentEvent::Question(text("text").unwrap_or_default()),
        "done" => AgentEvent::Done(text("text")),
        _ => return None,
    })
}

/// The tools that change a file.
fn edits(tool: &str) -> bool {
    matches!(tool, "Edit" | "Write" | "MultiEdit" | "NotebookEdit")
}

/// The last part of a path, for a short step.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

impl AgentInfo {
    fn step(&mut self, at: Instant, kind: StepKind, text: String) {
        if self.timeline.len() == MAX_STEPS {
            self.timeline.pop_front();
        }
        self.timeline.push_back(Step { at, kind, text });
    }

    /// Takes one event at `now`.
    pub fn apply(&mut self, event: &AgentEvent, now: Instant) {
        match event {
            AgentEvent::Prompt(text) => {
                let text = clean(text, MAX_TEXT);
                if self.first_prompt.is_none() && !text.is_empty() {
                    self.first_prompt = Some(text.clone());
                }
                self.question = None;
                self.step(now, StepKind::Prompt, text);
            }
            AgentEvent::Tool {
                name,
                file,
                command,
                todos,
            } => {
                self.question = None;
                if let Some(todos) = todos {
                    self.todos = todos.clone();
                }
                if let Some(file) = file.as_deref().filter(|_| edits(name)) {
                    *self.files.entry(file.to_owned()).or_default() += 1;
                }
                let text = match (name.as_str(), file, command) {
                    ("TodoWrite", ..) => match self.todo_progress() {
                        Some((done, total, _)) => format!("todos {done}/{total}"),
                        None => "todos".to_owned(),
                    },
                    (tool, Some(file), _) if edits(tool) => format!("edited {}", file_name(file)),
                    (tool, Some(file), _) => format!("{} {}", tool.to_lowercase(), file_name(file)),
                    (_, _, Some(command)) => format!("ran {command}"),
                    (tool, ..) => tool.to_owned(),
                };
                self.step(now, StepKind::Tool, clean(&text, MAX_TEXT));
            }
            AgentEvent::Question(text) => {
                let text = clean(text, MAX_TEXT);
                self.question = Some(text.clone());
                self.step(now, StepKind::Question, text);
            }
            AgentEvent::Done(message) => {
                self.question = None;
                let message = message.as_deref().map(|m| clean(m, MAX_TEXT));
                if let Some(m) = &message {
                    self.last_message = Some(m.clone());
                }
                self.step(now, StepKind::Done, message.unwrap_or_default());
            }
            AgentEvent::Usage(usage) => self.usage = Some(usage.clone()),
        }
    }

    /// Done todos, all todos, and the one in progress (its active text). `None` with no todos.
    pub fn todo_progress(&self) -> Option<(usize, usize, Option<&str>)> {
        if self.todos.is_empty() {
            return None;
        }
        let done = self
            .todos
            .iter()
            .filter(|t| t.status == TodoStatus::Completed)
            .count();
        let current = self
            .todos
            .iter()
            .find(|t| t.status == TodoStatus::InProgress)
            .map(|t| {
                if t.active.is_empty() {
                    t.content.as_str()
                } else {
                    t.active.as_str()
                }
            });
        Some((done, self.todos.len(), current))
    }

    /// The text of the newest step.
    pub fn last_step(&self) -> Option<&str> {
        self.timeline.back().map(|s| s.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn tool(name: &str, file: Option<&str>, command: Option<&str>) -> AgentEvent {
        AgentEvent::Tool {
            name: name.into(),
            file: file.map(str::to_owned),
            command: command.map(str::to_owned),
            todos: None,
        }
    }

    fn todo(content: &str, status: TodoStatus, active: &str) -> Todo {
        Todo {
            content: content.into(),
            status,
            active: active.into(),
        }
    }

    #[test]
    fn steps_of_an_agent() {
        let now = Instant::now();
        let mut info = AgentInfo::default();
        info.apply(&AgentEvent::Prompt("Fix the\ntests please".into()), now);
        info.apply(&tool("Edit", Some("C:/w/src/app.rs"), None), now);
        info.apply(&tool("Edit", Some("C:/w/src/app.rs"), None), now);
        info.apply(&tool("Write", Some("/w/src/new.rs"), None), now);
        info.apply(&tool("Read", Some("C:\\w\\README.md"), None), now);
        info.apply(&tool("Bash", None, Some("cargo test")), now);
        info.apply(&tool("WebSearch", None, None), now);
        let texts: Vec<&str> = info.timeline.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Fix the tests please",
                "edited app.rs",
                "edited app.rs",
                "edited new.rs",
                "read README.md",
                "ran cargo test",
                "WebSearch"
            ]
        );
        assert_eq!(info.first_prompt.as_deref(), Some("Fix the tests please"));
        // Only changed files, with how many times.
        assert_eq!(
            info.files
                .iter()
                .map(|(f, n)| (f.as_str(), *n))
                .collect::<Vec<_>>(),
            [("/w/src/new.rs", 1), ("C:/w/src/app.rs", 2)]
        );
        info.apply(&AgentEvent::Prompt("next one".into()), now);
        assert_eq!(
            info.first_prompt.as_deref(),
            Some("Fix the tests please"),
            "the first stays"
        );
    }

    #[test]
    fn a_question_until_the_agent_goes_on() {
        let now = Instant::now();
        let mut info = AgentInfo::default();
        info.apply(
            &AgentEvent::Question("Claude needs your permission to use Bash".into()),
            now,
        );
        assert_eq!(
            info.question.as_deref(),
            Some("Claude needs your permission to use Bash")
        );
        info.apply(&tool("Bash", None, Some("git push")), now);
        assert_eq!(info.question, None, "answered: it goes on");
        info.apply(&AgentEvent::Question("Allow?".into()), now);
        info.apply(
            &AgentEvent::Done(Some("All done.\n\nTests pass.".into())),
            now,
        );
        assert_eq!(info.question, None);
        assert_eq!(info.last_message.as_deref(), Some("All done. Tests pass."));
        assert_eq!(info.last_step(), Some("All done. Tests pass."));
    }

    #[test]
    fn todos_and_their_progress() {
        let now = Instant::now();
        let mut info = AgentInfo::default();
        assert_eq!(info.todo_progress(), None);
        let todos = vec![
            todo("Read the code", TodoStatus::Completed, "Reading the code"),
            todo("Write tests", TodoStatus::InProgress, "Writing tests"),
            todo("Fix the bug", TodoStatus::Pending, ""),
        ];
        info.apply(
            &AgentEvent::Tool {
                name: "TodoWrite".into(),
                file: None,
                command: None,
                todos: Some(todos),
            },
            now,
        );
        assert_eq!(info.todo_progress(), Some((1, 3, Some("Writing tests"))));
        assert_eq!(info.last_step(), Some("todos 1/3"));
        assert_eq!(TodoStatus::from_name("in_progress"), TodoStatus::InProgress);
        assert_eq!(TodoStatus::from_name("what"), TodoStatus::Pending);
    }

    #[test]
    fn texts_are_short_and_safe_and_the_timeline_is_capped() {
        assert_eq!(clean("a\u{1b}[31mb\u{7}\tc", 50), "a[31mb c");
        let long = "x".repeat(500);
        let cut = clean(&long, MAX_TEXT);
        assert_eq!(cut.chars().count(), MAX_TEXT);
        assert!(cut.ends_with('…'));
        let t0 = Instant::now();
        let mut info = AgentInfo::default();
        for i in 0..80 {
            info.apply(
                &tool("Bash", None, Some(&format!("step {i}"))),
                t0 + Duration::from_secs(i),
            );
        }
        assert_eq!(info.timeline.len(), MAX_STEPS);
        assert_eq!(
            info.timeline.front().unwrap().text,
            "ran step 30",
            "the oldest go"
        );
        assert_eq!(info.last_step(), Some("ran step 79"));
    }

    #[test]
    fn events_from_the_api() {
        use serde_json::json;
        assert_eq!(
            event_from_json(&json!({ "kind": "prompt", "text": "hi" })),
            Some(AgentEvent::Prompt("hi".into()))
        );
        assert_eq!(
            event_from_json(&json!({ "kind": "tool", "tool": "Bash", "command": "ls" })),
            Some(tool("Bash", None, Some("ls")))
        );
        let Some(AgentEvent::Tool {
            todos: Some(todos), ..
        }) = event_from_json(&json!({
            "kind": "tool", "tool": "TodoWrite",
            "todos": [{ "content": "a", "status": "completed", "active": "A" }, { "content": "b" }]
        }))
        else {
            panic!();
        };
        assert_eq!(todos[0], todo("a", TodoStatus::Completed, "A"));
        assert_eq!(todos[1].status, TodoStatus::Pending);
        assert_eq!(
            event_from_json(&json!({ "kind": "question", "text": "Allow?" })),
            Some(AgentEvent::Question("Allow?".into()))
        );
        assert_eq!(
            event_from_json(&json!({ "kind": "done" })),
            Some(AgentEvent::Done(None))
        );
        assert_eq!(event_from_json(&json!({ "kind": "what" })), None);
        assert_eq!(
            event_from_json(&json!({ "kind": "tool" })),
            None,
            "a tool needs its name"
        );
    }

    #[test]
    fn usage_from_the_status_line() {
        let mut info = AgentInfo::default();
        let usage = Usage {
            cost_usd: Some(0.42),
            context_pct: Some(37.0),
            model: Some("Opus".into()),
            ..Default::default()
        };
        info.apply(&AgentEvent::Usage(usage.clone()), Instant::now());
        assert_eq!(info.usage, Some(usage));
        assert!(info.timeline.is_empty(), "numbers are not a step");
    }
}
