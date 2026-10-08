//! Agent states (Claude Code and other tools): what each pane's agent does now.
//! They come from `OSC 777;fterm-agent;<state>;<message>`, for example from Claude Code hooks.

use std::time::Instant;

use fterm_render::theme::UiColors;
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
    /// fterm found it itself (the command and the window title), not from hooks.
    pub auto: bool,
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
pub fn badge_color(kind: AgentKind, ui: &UiColors) -> Rgb {
    match kind {
        AgentKind::Working => ui.agent_working,
        AgentKind::Waiting => ui.agent_waiting,
        AgentKind::Done => ui.agent_done,
        AgentKind::Error => ui.agent_error,
    }
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

/// Agent tools that fterm knows by their command.
const AGENT_PROGRAMS: &[&str] = &["claude", "opencode", "codex", "aider", "gemini"];

/// The agent tool of a command line: `claude --continue` → `claude`.
pub fn agent_program(command: &str) -> Option<&'static str> {
    let first = command.split_whitespace().next()?;
    let file = first
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(first)
        .to_ascii_lowercase();
    let name = [".exe", ".cmd", ".bat", ".ps1"]
        .iter()
        .find_map(|ext| file.strip_suffix(ext))
        .unwrap_or(&file);
    AGENT_PROGRAMS.iter().copied().find(|p| *p == name)
}

/// The state that an agent shows in its window title: a spinner = working, `✳` = it waits for
/// the next prompt (Claude Code does this).
pub fn title_state(title: &str) -> Option<AgentKind> {
    let first = title.trim_start().chars().next()?;
    match first {
        '◐' | '◓' | '◑' | '◒' | '\u{2801}'..='\u{28ff}' => Some(AgentKind::Working),
        '✳' => Some(AgentKind::Done),
        _ => None,
    }
}

/// The title without the state sign: what the agent works on.
pub fn title_topic(title: &str) -> &str {
    let title = title.trim();
    match title_state(title) {
        Some(_) => {
            let mut chars = title.chars();
            chars.next();
            chars.as_str().trim_start()
        }
        None => title,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_tools_by_their_command() {
        assert_eq!(agent_program("claude"), Some("claude"));
        assert_eq!(agent_program("  claude --continue"), Some("claude"));
        assert_eq!(agent_program(r"C:\tools\Claude.exe -r"), Some("claude"));
        assert_eq!(agent_program("claude.cmd"), Some("claude"));
        assert_eq!(
            agent_program("/usr/local/bin/opencode run"),
            Some("opencode")
        );
        assert_eq!(agent_program("codex"), Some("codex"));
        assert_eq!(agent_program("aider --model x"), Some("aider"));
        assert_eq!(agent_program("gemini"), Some("gemini"));
        assert_eq!(agent_program("claudette"), None);
        assert_eq!(agent_program("git status"), None);
        assert_eq!(agent_program(""), None);
    }

    #[test]
    fn the_state_in_the_title() {
        for spinner in ["◐ add-ui-color-themes", "◓ x", "◑ x", "◒ x", "⠋ Thinking"] {
            assert_eq!(title_state(spinner), Some(AgentKind::Working), "{spinner}");
        }
        assert_eq!(title_state("✳ Claude Code"), Some(AgentKind::Done));
        assert_eq!(title_state("powershell"), None);
        assert_eq!(title_state(""), None);
        assert_eq!(title_topic("◐ add-ui-color-themes"), "add-ui-color-themes");
        assert_eq!(title_topic("✳ Claude Code"), "Claude Code");
        assert_eq!(title_topic("plain"), "plain");
    }

    fn state(kind: AgentKind, seen: bool) -> AgentState {
        AgentState {
            kind,
            message: String::new(),
            since: Instant::now(),
            seen,
            auto: false,
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
        let ui = UiColors::default();
        for (i, a) in kinds.iter().enumerate() {
            for b in &kinds[i + 1..] {
                assert_ne!(badge_color(*a, &ui), badge_color(*b, &ui));
            }
        }
        assert!(
            badge_color(AgentKind::Error, &ui).r > badge_color(AgentKind::Error, &ui).g,
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
