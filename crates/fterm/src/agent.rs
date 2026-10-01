//! Agent states (Claude Code and other tools): what each pane's agent does now.
//! They come from `OSC 777;fterm-agent;<state>;<message>`, for example from Claude Code hooks.

use std::time::Instant;

use fterm_term::alacritty_terminal::vte::ansi::Rgb;

use crate::notify::Level;

/// The Claude Code hooks for `settings.json` (the action `copy_claude_hooks`).
pub const CLAUDE_HOOKS: &str = include_str!("../../../assets/claude/hooks.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentKind {
    Done,
    Working,
    Waiting,
    Error,
}

impl AgentKind {
    /// The name for Lua and the logs.
    pub fn name(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Error => "error",
        }
    }

    /// `idle` (or an empty state) = no agent state any more.
    pub fn parse(state: &str) -> Option<Option<Self>> {
        match state.trim().to_ascii_lowercase().as_str() {
            "working" | "busy" | "running" => Some(Some(Self::Working)),
            "waiting" | "input" | "attention" => Some(Some(Self::Waiting)),
            "done" | "finished" | "stop" => Some(Some(Self::Done)),
            "error" | "failed" => Some(Some(Self::Error)),
            "idle" | "" => Some(None),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentState {
    pub kind: AgentKind,
    pub message: String,
    pub since: Instant,
    /// The user saw it (the pane was on the screen and fterm was in front).
    pub seen: bool,
}

/// The badge of a tab: the most important state of its panes.
/// `Done` and `Error` that the user already saw give no badge.
pub fn tab_badge<'a>(states: impl IntoIterator<Item = &'a AgentState>) -> Option<AgentKind> {
    states
        .into_iter()
        .filter(|s| !(s.seen && matches!(s.kind, AgentKind::Done | AgentKind::Error)))
        .map(|s| s.kind)
        .max()
}

/// The color of the tab dot: working blue, waiting yellow, done green, error red.
pub fn badge_color(kind: AgentKind) -> Rgb {
    let (r, g, b) = match kind {
        AgentKind::Working => (0x4c, 0x9a, 0xff),
        AgentKind::Waiting => (0xf5, 0xc2, 0x18),
        AgentKind::Done => (0x3f, 0xc5, 0x6b),
        AgentKind::Error => (0xf0, 0x4a, 0x4a),
    };
    Rgb { r, g, b }
}

/// The notification for a new state: (title, body, level). `None` = no notification.
pub fn notification_for(
    kind: AgentKind,
    message: &str,
    name: &str,
) -> Option<(String, String, Level)> {
    let text = |fallback: &str| {
        if message.trim().is_empty() {
            fallback.to_owned()
        } else {
            message.to_owned()
        }
    };
    match kind {
        AgentKind::Working => None,
        AgentKind::Waiting => Some((
            format!("{name} waits for you"),
            text("It needs your answer."),
            Level::Attention,
        )),
        AgentKind::Done => Some((
            format!("{name} is done"),
            text("The task is finished."),
            Level::Success,
        )),
        AgentKind::Error => Some((
            format!("{name} failed"),
            text("The task ended with an error."),
            Level::Error,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(kind: AgentKind, seen: bool) -> AgentState {
        AgentState {
            kind,
            message: String::new(),
            since: Instant::now(),
            seen,
        }
    }

    #[test]
    fn each_state_has_its_own_badge_color() {
        let kinds = [
            AgentKind::Done,
            AgentKind::Working,
            AgentKind::Waiting,
            AgentKind::Error,
        ];
        for (i, a) in kinds.iter().enumerate() {
            for b in &kinds[i + 1..] {
                assert_ne!(badge_color(*a), badge_color(*b));
            }
        }
        assert!(
            badge_color(AgentKind::Error).r > badge_color(AgentKind::Error).g,
            "error is red"
        );
    }

    #[test]
    fn claude_hooks_send_known_states() {
        let states: Vec<&str> = CLAUDE_HOOKS
            .split("fterm-agent;")
            .skip(1)
            .map(|rest| rest.split(';').next().unwrap())
            .collect();
        assert_eq!(states.len(), 6);
        for state in states {
            assert!(AgentKind::parse(state).is_some(), "unknown state {state}");
        }
    }

    #[test]
    fn a_name_parses_back() {
        for kind in [
            AgentKind::Done,
            AgentKind::Working,
            AgentKind::Waiting,
            AgentKind::Error,
        ] {
            assert_eq!(AgentKind::parse(kind.name()), Some(Some(kind)));
        }
    }

    #[test]
    fn state_names() {
        assert_eq!(AgentKind::parse("working"), Some(Some(AgentKind::Working)));
        assert_eq!(AgentKind::parse("waiting"), Some(Some(AgentKind::Waiting)));
        assert_eq!(AgentKind::parse("done"), Some(Some(AgentKind::Done)));
        assert_eq!(AgentKind::parse("error"), Some(Some(AgentKind::Error)));
        assert_eq!(AgentKind::parse("idle"), Some(None));
        assert_eq!(AgentKind::parse(""), Some(None));
        assert_eq!(AgentKind::parse("dancing"), None);
    }

    #[test]
    fn badge_shows_the_most_important_state() {
        let states = [
            state(AgentKind::Done, false),
            state(AgentKind::Working, false),
            state(AgentKind::Waiting, false),
        ];
        assert_eq!(tab_badge(&states), Some(AgentKind::Waiting));
        let states = [
            state(AgentKind::Waiting, false),
            state(AgentKind::Error, false),
        ];
        assert_eq!(tab_badge(&states), Some(AgentKind::Error));
        assert_eq!(tab_badge(&[]), None);
    }

    #[test]
    fn seen_done_and_error_have_no_badge() {
        assert_eq!(tab_badge(&[state(AgentKind::Done, true)]), None);
        assert_eq!(tab_badge(&[state(AgentKind::Error, true)]), None);
        // Working and waiting stay: they are still true when you look at them.
        assert_eq!(
            tab_badge(&[state(AgentKind::Waiting, true)]),
            Some(AgentKind::Waiting)
        );
        assert_eq!(
            tab_badge(&[state(AgentKind::Working, true)]),
            Some(AgentKind::Working)
        );
    }

    #[test]
    fn notifications_for_states() {
        let (title, body, level) = notification_for(AgentKind::Waiting, "", "claude").unwrap();
        assert_eq!(level, Level::Attention);
        assert!(title.contains("claude") && !body.is_empty());
        let (_, body, level) =
            notification_for(AgentKind::Done, "All tests pass.", "claude").unwrap();
        assert_eq!(level, Level::Success);
        assert_eq!(body, "All tests pass.");
        assert_eq!(
            notification_for(AgentKind::Error, "", "x").unwrap().2,
            Level::Error
        );
        assert_eq!(
            notification_for(AgentKind::Working, "", "x"),
            None,
            "working is not news"
        );
    }
}
