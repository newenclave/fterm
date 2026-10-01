//! `wait_for`: what a client waits for in a pane, and when it has come.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitKind {
    /// The next command in the pane ends.
    CommandDone,
    /// The agent in the pane is done (or failed).
    AgentDone,
    /// The agent in the pane waits for an answer.
    AgentWaiting,
    /// A message comes to the pane.
    Message,
    /// This text is on the screen (now or later).
    Text(String),
}

/// Something that happened in a pane.
#[derive(Clone, Copy, Debug)]
pub enum Happening<'a> {
    CommandDone,
    Agent(&'a str),
    Message,
    /// The screen changed: its text now.
    Screen(&'a str),
}

/// `event` and `pattern` from the params of `wait_for`.
pub fn parse_kind(event: &str, pattern: Option<String>) -> Result<WaitKind, String> {
    match event {
        "command_done" => Ok(WaitKind::CommandDone),
        "agent_done" => Ok(WaitKind::AgentDone),
        "agent_waiting" => Ok(WaitKind::AgentWaiting),
        "message" => Ok(WaitKind::Message),
        "text" => match pattern {
            Some(text) if !text.is_empty() => Ok(WaitKind::Text(text)),
            _ => Err("`text` needs a `pattern`".to_owned()),
        },
        other => Err(format!(
            "`event` must be command_done, agent_done, agent_waiting, message, or text, got `{other}`"
        )),
    }
}

pub fn matches(kind: &WaitKind, happening: Happening) -> bool {
    match (kind, happening) {
        (WaitKind::CommandDone, Happening::CommandDone) => true,
        (WaitKind::AgentDone, Happening::Agent(state)) => state == "done" || state == "error",
        (WaitKind::AgentWaiting, Happening::Agent(state)) => state == "waiting",
        (WaitKind::Message, Happening::Message) => true,
        (WaitKind::Text(pattern), Happening::Screen(text)) => text.contains(pattern.as_str()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_from_params() {
        assert_eq!(parse_kind("command_done", None), Ok(WaitKind::CommandDone));
        assert_eq!(parse_kind("agent_done", None), Ok(WaitKind::AgentDone));
        assert_eq!(
            parse_kind("agent_waiting", None),
            Ok(WaitKind::AgentWaiting)
        );
        assert_eq!(parse_kind("message", None), Ok(WaitKind::Message));
        assert_eq!(
            parse_kind("text", Some("PASS".into())),
            Ok(WaitKind::Text("PASS".into()))
        );
        assert!(parse_kind("text", None).is_err(), "text needs a pattern");
        assert!(parse_kind("text", Some(String::new())).is_err());
        assert!(parse_kind("coffee", None).is_err());
    }

    #[test]
    fn what_matches() {
        use Happening::*;
        assert!(matches(&WaitKind::CommandDone, CommandDone));
        assert!(!matches(&WaitKind::CommandDone, Agent("done")));
        assert!(matches(&WaitKind::AgentDone, Agent("done")));
        assert!(
            matches(&WaitKind::AgentDone, Agent("error")),
            "a failure is an end too"
        );
        assert!(!matches(&WaitKind::AgentDone, Agent("working")));
        assert!(matches(&WaitKind::AgentWaiting, Agent("waiting")));
        assert!(matches(&WaitKind::Message, Message));
        let text = WaitKind::Text("test result: ok".into());
        assert!(matches(&text, Screen("...\ntest result: ok. 5 passed\n")));
        assert!(!matches(&text, Screen("test result: FAILED")));
        assert!(!matches(&text, CommandDone));
    }
}
