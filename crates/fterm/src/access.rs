//! Who may read or type into which pane (Phase 7).
//!
//! A client may always use its own pane (the pane where it runs) and the panes it opened.
//! For other panes, fterm asks the user once per client. A pane with "no remote control" is never used.

use fterm_config::tr;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Ask the user first.
    Ask,
    Deny(&'static str),
}

/// What fterm knows to decide.
pub struct Check<'a> {
    /// The client's own pane, or a pane that it opened.
    pub own: bool,
    /// The pane allows remote control.
    pub remote: bool,
    /// What the user said for this client before (`None` = not asked yet).
    pub said: Option<bool>,
    /// The client name, and the names that the user allowed "always".
    pub name: &'a str,
    pub always: &'a HashSet<String>,
    /// `api.ask` in the config.
    pub ask: bool,
}

pub fn check(c: &Check) -> Verdict {
    if c.own {
        Verdict::Allow
    } else if !c.remote {
        Verdict::Deny("this pane does not allow remote control")
    } else if c.said == Some(false) {
        Verdict::Deny("the user said no to this client")
    } else if c.said == Some(true) || c.always.contains(c.name) || !c.ask {
        Verdict::Allow
    } else {
        Verdict::Ask
    }
}

/// The lines of the question box.
pub fn question(name: &str, tab: Option<usize>) -> Vec<String> {
    let who = match tab {
        Some(tab) => tr!("box.in_tab", what = name, tab = tab),
        None => name.to_owned(),
    };
    vec![
        tr!("box.access", who = who),
        tr!("box.access_why"),
        String::new(),
        tr!("box.access_keys", name = name),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(always: &HashSet<String>) -> Check<'_> {
        Check {
            own: false,
            remote: true,
            said: None,
            name: "claude",
            always,
            ask: true,
        }
    }

    #[test]
    fn own_panes_are_free() {
        let none = HashSet::new();
        assert_eq!(
            check(&Check {
                own: true,
                ..base(&none)
            }),
            Verdict::Allow
        );
        // Even without remote control: it is the client's own pane.
        assert_eq!(
            check(&Check {
                own: true,
                remote: false,
                ..base(&none)
            }),
            Verdict::Allow
        );
    }

    #[test]
    fn other_panes_ask_once() {
        let none = HashSet::new();
        assert_eq!(check(&base(&none)), Verdict::Ask);
        assert_eq!(
            check(&Check {
                said: Some(true),
                ..base(&none)
            }),
            Verdict::Allow
        );
        assert!(matches!(
            check(&Check {
                said: Some(false),
                ..base(&none)
            }),
            Verdict::Deny(_)
        ));
    }

    #[test]
    fn always_and_the_config() {
        let always: HashSet<String> = ["claude".to_owned()].into();
        assert_eq!(check(&base(&always)), Verdict::Allow);
        let none = HashSet::new();
        assert_eq!(
            check(&Check {
                ask: false,
                ..base(&none)
            }),
            Verdict::Allow
        );
    }

    #[test]
    fn no_remote_control_wins() {
        let always: HashSet<String> = ["claude".to_owned()].into();
        let c = Check {
            remote: false,
            said: Some(true),
            ask: false,
            ..base(&always)
        };
        assert!(matches!(check(&c), Verdict::Deny(_)));
    }

    #[test]
    fn the_question() {
        let lines = question("claude", Some(2));
        assert_eq!(
            lines[0],
            "Allow claude (tab 2) to read and type into other panes?"
        );
        assert_eq!(
            lines.last().unwrap(),
            "Enter = allow, A = always allow claude, Esc = no"
        );
        assert!(question("ftermctl", None)[0].starts_with("Allow ftermctl to"));
    }
}
