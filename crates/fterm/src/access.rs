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
pub struct Check {
    /// The client's own pane (the one its processes show), or a pane that it opened.
    pub own: bool,
    /// The pane allows remote control.
    pub remote: bool,
    /// What the user said for this connection before (`None` = not asked yet).
    pub said: Option<bool>,
    /// The user said "always" for the program of the client in its pane.
    pub always: bool,
    /// The user said no to a client of the same bucket (pane) less than a minute ago.
    pub refused: bool,
    /// `api.ask` in the config.
    pub ask: bool,
    /// The call may show a question (the hooks of an agent never do: they come many times a second).
    pub may_ask: bool,
}

pub fn check(c: &Check) -> Verdict {
    if c.own {
        Verdict::Allow
    } else if !c.remote {
        Verdict::Deny("this pane does not allow remote control")
    } else if c.said == Some(false) {
        Verdict::Deny("the user said no to this client")
    } else if c.said == Some(true) || c.always || !c.ask {
        Verdict::Allow
    } else if c.refused {
        Verdict::Deny(
            "the user said no to this pane a moment ago: ask the user before you try again",
        )
    } else if !c.may_ask {
        Verdict::Deny("not in a pane of this window: an agent hook from here is not taken")
    } else {
        Verdict::Ask
    }
}

/// Calls that never show the question: the hooks of an agent. From a client that is not in a pane of this window
/// they are refused with no question (a hook in tmux must not ask the user on every tool call).
pub fn may_ask(method: &str) -> bool {
    !matches!(method, "agent_event" | "agent_session" | "agent_usage")
}

/// The key of an "Always": the checked pane of the client and its program there (process id and start time). Not
/// the name: a client picks its own name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AlwaysKey {
    pub pane: u64,
    pub pid: u32,
    pub start: u64,
}

/// The "Always" answers of the user, until the pane or its program ends.
#[derive(Debug, Default)]
pub struct AlwaysList(HashSet<AlwaysKey>);

impl AlwaysList {
    pub fn allow(&mut self, key: AlwaysKey) {
        self.0.insert(key);
    }

    pub fn allows(&self, key: Option<AlwaysKey>) -> bool {
        key.is_some_and(|k| self.0.contains(&k))
    }

    /// The pane closed.
    pub fn forget_pane(&mut self, pane: u64) {
        self.0.retain(|k| k.pane != pane);
    }
}

/// Who a "No" counts for: the checked pane of the client, or all clients with no pane (one bucket).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bucket {
    Pane(u64),
    NoPane,
}

/// How long a "No" stops new questions of its bucket.
pub const REFUSAL: std::time::Duration = std::time::Duration::from_secs(60);

/// The "No" answers of the user: for a minute, new questions of the same bucket are refused with no box.
#[derive(Debug, Default)]
pub struct Refusals(std::collections::HashMap<Bucket, std::time::Instant>);

impl Refusals {
    pub fn note(&mut self, bucket: Bucket, now: std::time::Instant) {
        // Old answers go: the list stays small.
        self.0
            .retain(|_, at| now.saturating_duration_since(*at) <= REFUSAL);
        self.0.insert(bucket, now);
    }

    pub fn refused(&self, bucket: Bucket, now: std::time::Instant) -> bool {
        self.0
            .get(&bucket)
            .is_some_and(|at| now.saturating_duration_since(*at) <= REFUSAL)
    }
}

/// The calls that need a yes for a pane that is not the client's own: they read it, type into it, or act on it. A
/// dialog (`review`, `form`) only when it asks to open at a pane (`pane`): over another program it can take the keys
/// and look like that program asks for something (a password).
pub fn gated(method: &str, params: &serde_json::Value) -> bool {
    match method {
        "send_text" | "get_text" | "close" | "wait_for" | "read_messages" | "spawn"
        | "screenshot" | "ai_read" | "ai_ask" | "ai_clear" | "agent_session" | "agent_event"
        | "agent_usage" => true,
        "review" | "form" => params.get("pane").is_some_and(|p| !p.is_null()),
        _ => false,
    }
}

/// The pane is the client's own: its pane (from `hello`), or a pane that it opened. Not a pane that it opened for a
/// dialog: a scene or a picture needs no yes and can sit in the tab of another program, and a dialog over it would
/// sit there too.
pub fn owns(method: &str, its_pane: bool, opened_it: bool) -> bool {
    its_pane || (opened_it && !matches!(method, "review" | "form"))
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

    #[test]
    fn which_calls_need_a_yes() {
        use serde_json::json;
        assert!(gated("send_text", &json!({})));
        assert!(gated("get_text", &json!({ "pane": 3 })));
        assert!(!gated("list", &json!({})));
        // A dialog only when it asks to open at a pane: there it can look like that pane asks for a password.
        assert!(gated("form", &json!({ "pane": 3 })));
        assert!(gated("review", &json!({ "pane": 3 })));
        assert!(!gated("form", &json!({})));
        assert!(!gated("form", &json!({ "pane": null })));
        assert!(!gated("review", &json!({ "items": ["a"] })));
    }

    #[test]
    fn a_pane_that_a_client_opened_is_not_its_own_for_a_dialog() {
        // A scene needs no yes and can sit in the tab of another program: a dialog over it would sit there too.
        assert!(owns("send_text", false, true));
        assert!(!owns("form", false, true));
        assert!(!owns("review", false, true));
        assert!(owns("form", true, false));
        assert!(!owns("get_text", false, false));
    }

    fn base() -> Check {
        Check {
            own: false,
            remote: true,
            said: None,
            always: false,
            refused: false,
            ask: true,
            may_ask: true,
        }
    }

    #[test]
    fn own_panes_are_free() {
        assert_eq!(
            check(&Check {
                own: true,
                ..base()
            }),
            Verdict::Allow
        );
        // Even without remote control: it is the client's own pane.
        assert_eq!(
            check(&Check {
                own: true,
                remote: false,
                ..base()
            }),
            Verdict::Allow
        );
    }

    #[test]
    fn other_panes_ask_once() {
        assert_eq!(check(&base()), Verdict::Ask);
        assert_eq!(
            check(&Check {
                said: Some(true),
                ..base()
            }),
            Verdict::Allow
        );
        assert!(matches!(
            check(&Check {
                said: Some(false),
                ..base()
            }),
            Verdict::Deny(_)
        ));
    }

    #[test]
    fn always_and_the_config() {
        assert_eq!(
            check(&Check {
                always: true,
                ..base()
            }),
            Verdict::Allow
        );
        assert_eq!(
            check(&Check {
                ask: false,
                ..base()
            }),
            Verdict::Allow
        );
    }

    #[test]
    fn no_remote_control_wins() {
        let c = Check {
            remote: false,
            said: Some(true),
            always: true,
            ask: false,
            ..base()
        };
        assert!(matches!(check(&c), Verdict::Deny(_)));
    }

    #[test]
    fn a_recent_no_stops_the_question() {
        assert!(matches!(
            check(&Check {
                refused: true,
                ..base()
            }),
            Verdict::Deny(_)
        ));
        // A yes of this connection, an Always, or no questions at all still allow.
        assert_eq!(
            check(&Check {
                refused: true,
                said: Some(true),
                ..base()
            }),
            Verdict::Allow
        );
        assert_eq!(
            check(&Check {
                refused: true,
                always: true,
                ..base()
            }),
            Verdict::Allow
        );
    }

    #[test]
    fn hooks_never_ask() {
        assert!(!may_ask("agent_event") && !may_ask("agent_session") && !may_ask("agent_usage"));
        assert!(may_ask("send_text") && may_ask("get_text"));
        assert!(matches!(
            check(&Check {
                may_ask: false,
                ..base()
            }),
            Verdict::Deny(_)
        ));
        assert_eq!(
            check(&Check {
                may_ask: false,
                own: true,
                ..base()
            }),
            Verdict::Allow,
            "a hook in its own pane is fine"
        );
    }

    #[test]
    fn always_is_for_one_program_in_one_pane() {
        let claude = AlwaysKey {
            pane: 3,
            pid: 70,
            start: 500,
        };
        let mut always = AlwaysList::default();
        always.allow(claude);
        assert!(always.allows(Some(claude)));
        assert!(
            !always.allows(Some(AlwaysKey { pid: 71, ..claude })),
            "another program in that pane"
        );
        assert!(
            !always.allows(Some(AlwaysKey {
                start: 501,
                ..claude
            })),
            "a new process with that pid"
        );
        assert!(
            !always.allows(Some(AlwaysKey { pane: 4, ..claude })),
            "another pane"
        );
        assert!(!always.allows(None), "a client with no pane");
        always.forget_pane(3);
        assert!(!always.allows(Some(claude)), "the pane closed");
    }

    #[test]
    fn a_no_counts_for_a_minute_in_its_bucket() {
        let t = std::time::Instant::now();
        let mut no = Refusals::default();
        no.note(Bucket::Pane(3), t);
        assert!(no.refused(Bucket::Pane(3), t + std::time::Duration::from_secs(59)));
        assert!(!no.refused(
            Bucket::Pane(3),
            t + REFUSAL + std::time::Duration::from_millis(1)
        ));
        assert!(!no.refused(Bucket::Pane(4), t), "another pane");
        assert!(!no.refused(Bucket::NoPane, t));
        no.note(Bucket::NoPane, t);
        assert!(no.refused(Bucket::NoPane, t), "all clients with no pane");
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
