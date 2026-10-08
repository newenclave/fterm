//! Closing the whole window: ask first when something runs (Phase 6c).
//! Pure: what runs, what to do, and the text of the question.

use fterm_config::load::ConfirmClose;
use fterm_config::{tr, trn};

/// What runs in the window now. Tab numbers start at 1.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowState {
    pub tabs: usize,
    pub panes: usize,
    /// (tab, program) for each program that runs in a pane (not only the shell).
    pub running: Vec<(usize, String)>,
    /// (tab, name, state) for each pane with an agent state.
    pub agents: Vec<(usize, String, String)>,
}

impl WindowState {
    /// A program runs, or an agent works or waits for an answer.
    pub fn busy(&self) -> bool {
        !self.running.is_empty()
            || self
                .agents
                .iter()
                .any(|(_, _, state)| state == "working" || state == "waiting")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseDecision {
    /// Close at once.
    Now,
    /// Ask with these lines.
    Ask(Vec<String>),
    /// Do not close (the Lua hook said no).
    Stop,
}

/// `hook` is what `on_close_window` returned: `Some(true)` = close, `Some(false)` = stop, `None` = the rule.
pub fn decide(state: &WindowState, policy: ConfirmClose, hook: Option<bool>) -> CloseDecision {
    match (hook, policy) {
        (Some(true), _) => CloseDecision::Now,
        (Some(false), _) => CloseDecision::Stop,
        (None, ConfirmClose::Never) => CloseDecision::Now,
        (None, ConfirmClose::Always) => CloseDecision::Ask(question(state)),
        (None, ConfirmClose::Running) if state.busy() => CloseDecision::Ask(question(state)),
        (None, ConfirmClose::Running) => CloseDecision::Now,
    }
}

/// The × again, soon after the hook stopped the close: ask instead of stopping again.
pub fn decide_again(
    state: &WindowState,
    policy: ConfirmClose,
    hook: Option<bool>,
) -> CloseDecision {
    match decide(state, policy, hook) {
        CloseDecision::Stop => CloseDecision::Ask(question(state)),
        other => other,
    }
}

/// The lines of the question box.
pub fn question(state: &WindowState) -> Vec<String> {
    let mut lines = vec![
        tr!("box.close_fterm"),
        tr!(
            "box.tabs_and_panes",
            tabs = trn!("count.tabs", state.tabs),
            panes = trn!("count.panes", state.panes)
        ),
    ];
    if !state.running.is_empty() {
        let list: Vec<String> = state
            .running
            .iter()
            .map(|(tab, program)| tr!("box.in_tab", what = program, tab = tab))
            .collect();
        lines.push(tr!("box.running", list = list.join(", ")));
    }
    let agents: Vec<String> = state
        .agents
        .iter()
        .filter(|(_, _, s)| s == "working" || s == "waiting")
        .map(|(tab, name, s)| {
            let what = if s == "working" {
                tr!("box.agent_working", name = name)
            } else {
                tr!("box.agent_waiting", name = name)
            };
            tr!("box.in_tab", what = what, tab = tab)
        })
        .collect();
    if !agents.is_empty() {
        lines.push(tr!("box.agents", list = agents.join(", ")));
    }
    lines.push(String::new());
    lines.push(tr!("box.close_keys"));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle() -> WindowState {
        WindowState {
            tabs: 2,
            panes: 3,
            running: vec![],
            agents: vec![],
        }
    }

    fn busy() -> WindowState {
        WindowState {
            tabs: 3,
            panes: 5,
            running: vec![(2, "claude".into()), (3, "cargo".into())],
            agents: vec![(2, "claude".into(), "working".into())],
        }
    }

    #[test]
    fn what_is_busy() {
        assert!(!idle().busy());
        assert!(busy().busy());
        let done_agent = WindowState {
            agents: vec![(1, "claude".into(), "done".into())],
            ..idle()
        };
        assert!(!done_agent.busy(), "a done agent does not stop the close");
        let waiting = WindowState {
            agents: vec![(1, "claude".into(), "waiting".into())],
            ..idle()
        };
        assert!(waiting.busy());
    }

    #[test]
    fn the_rule() {
        use ConfirmClose::*;
        assert_eq!(decide(&idle(), Running, None), CloseDecision::Now);
        assert!(matches!(
            decide(&busy(), Running, None),
            CloseDecision::Ask(_)
        ));
        assert!(matches!(
            decide(&idle(), Always, None),
            CloseDecision::Ask(_)
        ));
        assert_eq!(decide(&busy(), Never, None), CloseDecision::Now);
    }

    #[test]
    fn the_hook_wins() {
        use ConfirmClose::*;
        assert_eq!(decide(&busy(), Always, Some(true)), CloseDecision::Now);
        assert_eq!(decide(&idle(), Never, Some(false)), CloseDecision::Stop);
    }

    #[test]
    fn a_second_x_after_a_stop_asks() {
        // A hook that always says no must not lock the user in: the second × asks.
        assert!(matches!(
            decide_again(&idle(), ConfirmClose::Never, Some(false)),
            CloseDecision::Ask(_)
        ));
        assert_eq!(
            decide_again(&idle(), ConfirmClose::Running, None),
            decide(&idle(), ConfirmClose::Running, None)
        );
    }

    #[test]
    fn the_question_says_what_runs() {
        let lines = question(&busy());
        assert_eq!(lines[0], "Close fterm?");
        assert_eq!(lines[1], "3 tabs, 5 panes.");
        assert_eq!(lines[2], "Running: claude (tab 2), cargo (tab 3)");
        assert_eq!(lines[3], "Agents: claude is working (tab 2)");
        assert_eq!(lines.last().unwrap(), "Enter = close, Esc = cancel");
        let lines = question(&WindowState {
            tabs: 1,
            panes: 1,
            ..idle()
        });
        assert_eq!(lines[1], "1 tab, 1 pane.");
        assert!(
            !lines
                .iter()
                .any(|l| l.starts_with("Running") || l.starts_with("Agents"))
        );
    }
}
