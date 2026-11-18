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
    /// The event hooks of the config.
    Hooks,
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
        // A program that got a No can start a helper that fterm can not find in a pane: that one waits too.
        if matches!(bucket, Bucket::Pane(_)) {
            self.0.insert(Bucket::NoPane, now);
        }
    }

    pub fn refused(&self, bucket: Bucket, now: std::time::Instant) -> bool {
        self.0
            .get(&bucket)
            .is_some_and(|at| now.saturating_duration_since(*at) <= REFUSAL)
    }
}

/// The calls that need a yes for a pane that is not the client's own: they read it, type into it, or act on it. A
/// dialog (`review`, `form`) or a picture (`show_image`) only when it asks to open at a pane (`pane`): next to
/// another program it can take the keys and look like that program asks for something (a password).
pub fn gated(method: &str, params: &serde_json::Value) -> bool {
    match method {
        "send_text" | "get_text" | "close" | "wait_for" | "read_messages" | "spawn"
        | "screenshot" | "ai_read" | "ai_ask" | "ai_clear" | "agent_session" | "agent_event"
        | "agent_usage" => true,
        "review" | "form" | "show_image" => params.get("pane").is_some_and(|p| !p.is_null()),
        _ => false,
    }
}

/// The pane is the client's own: its pane (from `hello`), or a pane that it opened. Not a pane that it opened for a
/// dialog: a scene or a picture needs no yes and can sit in the tab of another program, and a dialog over it would
/// sit there too.
pub fn owns(method: &str, its_pane: bool, opened_it: bool) -> bool {
    its_pane || (opened_it && !matches!(method, "review" | "form" | "show_image"))
}

/// An answer to the access question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Allow,
    Always,
    No,
}

/// A button of the question box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Allow,
    Always,
    No,
}

/// A key while the question is answered (the app makes it from the key of winit).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerKey {
    /// Y or 1.
    Allow,
    /// A or 2 (then Enter).
    Always,
    /// N, 3, or Esc.
    No,
    /// Enter.
    Confirm,
    /// Shift or another key alone, or a key that repeats: nothing happens.
    Ignore,
    /// Any other key: answer mode ends.
    Other,
}

/// The access question is answered on purpose: it never takes the keys of the user. A chord (or a click on a
/// button) starts answer mode for the question in front; it ends after one answer, when that question changes or
/// goes, on a click outside the box, when the window loses the focus, or after a few seconds.
#[derive(Debug, Default)]
pub struct AnswerMode {
    /// The question in front and when it came (clicks count a moment later).
    front: Option<(u64, std::time::Instant)>,
    /// Answer mode is on, since (the chord, or the last choice).
    on: Option<std::time::Instant>,
    /// When the chord started it (keys right after it answer nothing).
    started: Option<std::time::Instant>,
    /// The answer that a key chose: Enter takes it.
    chosen: Option<Answer>,
    /// Where the box is (a hash): when it moves, clicks wait again.
    place: Option<u64>,
    /// A button that the mouse pressed.
    pressed: Option<Button>,
}

/// How long answer mode stays with no key.
pub const ANSWER_TIME: std::time::Duration = std::time::Duration::from_secs(5);
/// A click on a button counts only this long after the box came or changed.
pub const CLICK_WAIT: std::time::Duration = std::time::Duration::from_secs(1);
/// Keys this soon after the chord answer nothing (the user typed on, maybe by habit): answer mode ends.
pub const ANSWER_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

impl AnswerMode {
    /// The question in front is `question` now (`None`: no question). Another question ends answer mode.
    pub fn front(&mut self, question: Option<u64>, now: std::time::Instant) {
        if self.front.map(|(id, _)| id) != question {
            self.front = question.map(|id| (id, now));
            self.end();
            self.pressed = None;
        }
    }

    /// The chord: answer mode for the question in front. `false` when there is no question, or when it came (or
    /// the window came back) less than `CLICK_WAIT` ago: then the chord is the key of the program, as with no box.
    pub fn start(&mut self, now: std::time::Instant) -> bool {
        let ready = self
            .front
            .is_some_and(|(_, at)| now.saturating_duration_since(at) >= CLICK_WAIT);
        if !ready {
            return false;
        }
        self.on = Some(now);
        self.started = Some(now);
        self.chosen = None;
        true
    }

    pub fn active(&self, now: std::time::Instant) -> bool {
        self.front.is_some()
            && self
                .on
                .is_some_and(|at| now.saturating_duration_since(at) <= ANSWER_TIME)
    }

    /// The answer that waits for Enter.
    pub fn chosen(&self) -> Option<Answer> {
        self.chosen
    }

    /// The box is at `place` now (a hash of where it and its buttons are). When it moved, clicks wait again; answer
    /// mode stays (keys do not move).
    pub fn moved(&mut self, place: u64, now: std::time::Instant) {
        if self.place != Some(place) {
            if self.place.is_some() {
                self.restart_wait(now);
            }
            self.place = Some(place);
        }
    }

    /// The window came back (it got the focus): the click that brings it back must not answer, so the wait for
    /// clicks starts again.
    pub fn restart_wait(&mut self, now: std::time::Instant) {
        if let Some((id, _)) = self.front {
            self.front = Some((id, now));
        }
        self.pressed = None;
    }

    pub fn end(&mut self) {
        self.on = None;
        self.chosen = None;
    }

    /// A key in answer mode. `always_ok`: the question offers "Always" (a client with a checked pane).
    pub fn key(
        &mut self,
        key: AnswerKey,
        always_ok: bool,
        now: std::time::Instant,
    ) -> Option<Answer> {
        if !self.active(now) {
            return None;
        }
        let early = self
            .started
            .is_some_and(|at| now.saturating_duration_since(at) < ANSWER_DELAY);
        let key = match key {
            AnswerKey::Allow | AnswerKey::Always | AnswerKey::No | AnswerKey::Confirm if early => {
                AnswerKey::Other
            }
            key => key,
        };
        // A yes needs two keys (the choice, then Enter): a word typed on by habit ("yes") is no answer.
        match key {
            AnswerKey::Ignore => None,
            AnswerKey::Allow => {
                self.chosen = Some(Answer::Allow);
                self.on = Some(now);
                None
            }
            AnswerKey::Always => {
                if always_ok {
                    self.chosen = Some(Answer::Always);
                    self.on = Some(now);
                }
                None
            }
            AnswerKey::Confirm => {
                let chosen = self.chosen?;
                self.end();
                Some(chosen)
            }
            AnswerKey::No => {
                self.end();
                Some(Answer::No)
            }
            AnswerKey::Other => {
                self.end();
                None
            }
        }
    }

    /// The mouse went down on `button` (`None`: outside the box).
    pub fn press(&mut self, button: Option<Button>, now: std::time::Instant) {
        match button {
            None => {
                self.end();
                self.pressed = None;
            }
            Some(button) => {
                let ready = self
                    .front
                    .is_some_and(|(_, at)| now.saturating_duration_since(at) >= CLICK_WAIT);
                self.pressed = ready.then_some(button);
            }
        }
    }

    /// The mouse went up on `button`: an answer when it is the button where it went down.
    pub fn release(&mut self, button: Option<Button>) -> Option<Answer> {
        let pressed = self.pressed.take()?;
        if Some(pressed) != button {
            return None;
        }
        self.end();
        Some(match pressed {
            Button::Allow => Answer::Allow,
            Button::Always => Answer::Always,
            Button::No => Answer::No,
        })
    }

    /// When answer mode ends by itself (to draw the box again then).
    pub fn ends_at(&self) -> Option<std::time::Instant> {
        self.on.map(|at| at + ANSWER_TIME)
    }
}

/// Who asks, for the question box.
pub struct Asker<'a> {
    /// The name that the client gave itself (for a hook: the name of the hook).
    pub name: &'a str,
    pub place: Place<'a>,
}

/// Where the asker runs, as fterm checked it.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum Place<'a> {
    /// In a pane: its tab (`None`: a pane that is in no tab, as the scratch shell), the workspace of that tab when
    /// there are more, and the program there. Only then "Always" is offered.
    Pane {
        tab: Option<usize>,
        workspace: Option<&'a str>,
        program: &'a str,
    },
    /// Not in a pane of this window, or not known.
    Outside,
    /// An event hook of the config (`fterm.lua`).
    Config,
}

/// Calls of a Lua event hook that need the access question: they type into a pane, start programs, or run actions.
/// The others (a toast, a tab color, the clipboard) do not.
pub fn hook_call_gated(call: &fterm_config::load::ApiCall) -> bool {
    use fterm_config::load::ApiCall;
    !matches!(
        call,
        ApiCall::Notify { .. } | ApiCall::SetTabColor { .. } | ApiCall::Copy(_)
    )
}

/// The lines of a question box, each with its look.
pub type QuestionLines = Vec<(String, fterm_render::overlay::QuestionLook)>;

/// The lines and buttons of the access question. `answering`: answer mode is on; `always_chosen`: "Always" waits
/// for Enter.
pub fn question_box(
    asker: &Asker,
    answering: bool,
    chosen: Option<Answer>,
) -> (QuestionLines, Vec<(Button, String)>) {
    use fterm_render::overlay::QuestionLook as L;
    // Names come from outside (the client, the process): one short line.
    let short = |text: &str| fterm_ui::text::safe_name(text, 32);
    let always_ok = matches!(asker.place, Place::Pane { .. });
    let fact = match asker.place {
        Place::Pane {
            tab: Some(tab),
            workspace: None,
            program,
        } => tr!("box.access_where", tab = tab, program = short(program)),
        Place::Pane {
            tab: Some(tab),
            workspace: Some(workspace),
            program,
        } => tr!(
            "box.access_where_ws",
            tab = tab,
            workspace = short(workspace),
            program = short(program)
        ),
        Place::Pane {
            tab: None, program, ..
        } => tr!("box.access_where_float", program = short(program)),
        Place::Outside => tr!("box.access_outside"),
        Place::Config => tr!("box.access_config"),
    };
    let keys = if chosen == Some(Answer::Always) {
        tr!("box.access_confirm")
    } else if chosen == Some(Answer::Allow) {
        tr!("box.access_confirm_allow")
    } else if !answering {
        tr!("box.access_start")
    } else if always_ok {
        tr!("box.access_keys")
    } else {
        tr!("box.access_keys_no_always")
    };
    let asks = match asker.place {
        Place::Config => tr!("box.access_hook", hook = short(asker.name)),
        _ => tr!("box.access_asks", name = short(asker.name)),
    };
    let lines = vec![
        (asks, L::Text),
        (fact, L::Fact),
        (tr!("box.access_why"), L::Text),
        (keys, L::Dim),
    ];
    let mut buttons = vec![(Button::Allow, tr!("box.access_allow"))];
    if always_ok {
        buttons.push((Button::Always, tr!("box.access_always")));
    }
    buttons.push((Button::No, tr!("box.access_no")));
    (lines, buttons)
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
        // A picture at another pane takes the keys there and shows the text of the client.
        assert!(gated("show_image", &json!({ "pane": 3, "path": "a.png" })));
        assert!(
            !gated("show_image", &json!({ "path": "a.png" })),
            "at its own pane"
        );
    }

    #[test]
    fn a_pane_that_a_client_opened_is_not_its_own_for_a_dialog() {
        // A scene needs no yes and can sit in the tab of another program: a dialog over it would sit there too.
        assert!(owns("send_text", false, true));
        assert!(!owns("form", false, true));
        assert!(!owns("review", false, true));
        assert!(owns("form", true, false));
        assert!(
            !owns("show_image", false, true),
            "a picture next to a scene that it opened"
        );
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
        assert!(
            no.refused(Bucket::NoPane, t),
            "a helper that fterm can not find in a pane does not ask again"
        );
        assert!(!no.refused(Bucket::Hooks, t));
        let mut no = Refusals::default();
        no.note(Bucket::NoPane, t);
        assert!(no.refused(Bucket::NoPane, t), "all clients with no pane");
        assert!(!no.refused(Bucket::Pane(3), t));
    }

    fn secs(n: u64) -> std::time::Duration {
        std::time::Duration::from_secs(n)
    }

    fn ms(n: u64) -> std::time::Duration {
        std::time::Duration::from_millis(n)
    }

    #[test]
    fn keys_answer_only_in_answer_mode() {
        let t = std::time::Instant::now();
        let mut m = AnswerMode::default();
        assert!(!m.start(t), "no question: no answer mode");
        m.front(Some(1), t);
        assert!(!m.active(t), "a question alone takes no keys");
        assert_eq!(m.key(AnswerKey::Allow, true, t), None, "not in answer mode");
        let s = t + CLICK_WAIT;
        assert!(m.start(s) && m.active(s));
        let k = s + ANSWER_DELAY;
        assert_eq!(
            m.key(AnswerKey::Ignore, true, k),
            None,
            "Shift alone, or a repeat"
        );
        assert!(m.active(k));
        assert_eq!(m.key(AnswerKey::Allow, true, k), None, "Y chooses Allow");
        assert_eq!(m.chosen(), Some(Answer::Allow));
        assert_eq!(
            m.key(AnswerKey::Confirm, true, k),
            Some(Answer::Allow),
            "and Enter takes it"
        );
        assert!(!m.active(k), "one answer, then it ends");
    }

    #[test]
    fn the_chord_counts_only_when_the_box_was_there_a_moment() {
        // An agent that times its question just before the user presses Ctrl+Shift+Enter (a new line for them).
        let t = std::time::Instant::now();
        let mut m = AnswerMode::default();
        m.front(Some(1), t);
        assert!(!m.start(t + ms(500)), "the box just came");
        assert!(m.start(t + CLICK_WAIT));
        m.end();
        let back = t + secs(10);
        m.restart_wait(back);
        assert!(!m.start(back + ms(100)), "the window just came back");
        assert!(m.start(back + CLICK_WAIT));
    }

    #[test]
    fn always_needs_enter_and_a_checked_pane() {
        let t = std::time::Instant::now();
        let mut m = AnswerMode::default();
        m.front(Some(1), t);
        let s = t + CLICK_WAIT;
        m.start(s);
        let k = s + ANSWER_DELAY;
        assert_eq!(m.key(AnswerKey::Always, true, k), None);
        assert!(m.chosen() == Some(Answer::Always) && m.active(k));
        assert_eq!(m.key(AnswerKey::Confirm, true, k), Some(Answer::Always));
        m.start(s);
        assert_eq!(
            m.key(AnswerKey::Always, false, k),
            None,
            "no checked pane: no Always"
        );
        assert_eq!(m.chosen(), None);
        assert_eq!(
            m.key(AnswerKey::Confirm, false, k),
            None,
            "Enter alone answers nothing"
        );
        assert_eq!(
            m.key(AnswerKey::No, false, k),
            Some(Answer::No),
            "No needs no Enter"
        );
    }

    #[test]
    fn another_key_after_a_choice_takes_it_back() {
        // "yes" typed on after the chord: y chooses, e ends it.
        let t = std::time::Instant::now();
        let mut m = AnswerMode::default();
        m.front(Some(1), t);
        let s = t + CLICK_WAIT;
        m.start(s);
        let k = s + ANSWER_DELAY;
        assert_eq!(m.key(AnswerKey::Allow, true, k), None);
        assert_eq!(m.key(AnswerKey::Other, true, k), None);
        assert!(!m.active(k));
        assert_eq!(m.chosen(), None);
    }

    #[test]
    fn a_key_right_after_the_chord_answers_nothing() {
        // Ctrl+Shift+Enter is a new line in some programs: a user may type on after it.
        let t = std::time::Instant::now();
        let mut m = AnswerMode::default();
        m.front(Some(1), t);
        let s = t + CLICK_WAIT;
        m.start(s);
        assert_eq!(m.key(AnswerKey::Allow, true, s + ms(100)), None);
        assert!(!m.active(s + ms(100)), "typed on: answer mode ends");
        assert_eq!(m.chosen(), None);
        let s = s + secs(1);
        m.start(s);
        assert_eq!(m.key(AnswerKey::Allow, true, s + ANSWER_DELAY), None);
        assert_eq!(
            m.key(AnswerKey::Confirm, true, s + ANSWER_DELAY),
            Some(Answer::Allow)
        );
    }

    #[test]
    fn the_click_wait_starts_again_when_the_window_comes_back() {
        let t = std::time::Instant::now();
        let mut m = AnswerMode::default();
        m.front(Some(1), t);
        let back = t + secs(30);
        m.restart_wait(back);
        m.press(Some(Button::Allow), back + ms(200));
        assert_eq!(
            m.release(Some(Button::Allow)),
            None,
            "the click that brings the window back"
        );
        m.press(Some(Button::Allow), back + secs(2));
        assert_eq!(m.release(Some(Button::Allow)), Some(Answer::Allow));
    }

    #[test]
    fn a_box_that_moves_waits_for_clicks_again_but_keeps_its_keys() {
        // A client can move the box (the dock opens): a button must not move under the mouse, and the box must not
        // be kept from an answer by keys either.
        let t = std::time::Instant::now();
        let mut m = AnswerMode::default();
        m.front(Some(1), t);
        m.moved(7, t);
        let s = t + secs(2);
        assert!(m.start(s));
        m.moved(8, s);
        assert!(m.active(s), "keys still answer");
        m.press(Some(Button::Allow), s + ms(300));
        assert_eq!(
            m.release(Some(Button::Allow)),
            None,
            "too soon after it moved"
        );
        m.press(Some(Button::Allow), s + secs(2));
        assert_eq!(m.release(Some(Button::Allow)), Some(Answer::Allow));
    }

    #[test]
    fn answer_mode_ends_by_itself() {
        let t = std::time::Instant::now();
        let mut m = AnswerMode::default();
        m.front(Some(1), t);
        let s = t + CLICK_WAIT;
        m.start(s);
        assert_eq!(m.key(AnswerKey::Other, true, s), None);
        assert!(!m.active(s), "another key ends it");
        m.start(s);
        assert!(!m.active(s + ANSWER_TIME + secs(1)), "after a few seconds");
        assert_eq!(m.ends_at(), Some(s + ANSWER_TIME));
        m.start(s);
        m.front(Some(2), s);
        assert!(!m.active(s), "another question in front");
        m.front(None, s);
        assert!(!m.active(s), "no question");
        m.front(Some(3), t);
        m.start(s);
        m.end();
        assert!(!m.active(s), "focus lost, or a click outside");
    }

    #[test]
    fn a_click_answers_on_a_button_after_a_moment() {
        let t = std::time::Instant::now();
        let mut m = AnswerMode::default();
        m.front(Some(1), t);
        m.press(Some(Button::Allow), t + ms(500));
        assert_eq!(
            m.release(Some(Button::Allow)),
            None,
            "too soon after the box came"
        );
        m.press(Some(Button::Allow), t + secs(2));
        assert_eq!(m.release(Some(Button::No)), None, "up on another button");
        m.press(Some(Button::Always), t + secs(2));
        assert_eq!(m.release(Some(Button::Always)), Some(Answer::Always));
        m.front(Some(2), t + secs(3));
        m.press(Some(Button::No), t + secs(3));
        assert_eq!(
            m.release(Some(Button::No)),
            None,
            "a new question: the wait starts again"
        );
        assert!(m.start(t + secs(4)));
        m.press(None, t + secs(5));
        assert!(
            !m.active(t + secs(5)),
            "a click outside the box ends answer mode"
        );
    }

    #[test]
    fn the_question_box_says_what_fterm_checked() {
        use fterm_render::overlay::QuestionLook as L;
        let checked = Asker {
            name: "claude",
            place: Place::Pane {
                tab: Some(2),
                workspace: None,
                program: "claude.exe",
            },
        };
        let (lines, buttons) = question_box(&checked, false, None);
        assert!(
            lines[0].0.contains("\"claude\""),
            "the name, quoted: {lines:?}"
        );
        assert_eq!(lines[0].1, L::Text);
        assert!(
            lines[1].0.contains("tab 2") && lines[1].0.contains("claude.exe"),
            "{lines:?}"
        );
        assert_eq!(lines[1].1, L::Fact, "fterm's own line has its own color");
        let keys = lines.last().unwrap();
        assert_eq!(keys.1, L::Dim);
        assert!(
            keys.0.contains("Ctrl+Shift+Enter"),
            "how to answer: {keys:?}"
        );
        let which: Vec<Button> = buttons.iter().map(|(b, _)| *b).collect();
        assert_eq!(which, [Button::Allow, Button::Always, Button::No]);
        assert_eq!(buttons[1].1, "Always for this pane");

        let (lines, _) = question_box(&checked, true, None);
        assert!(lines.last().unwrap().0.contains("A or 2"), "{lines:?}");
        let (lines, _) = question_box(&checked, true, Some(Answer::Always));
        assert!(lines.last().unwrap().0.contains("Enter"), "{lines:?}");
        let (lines, _) = question_box(&checked, true, Some(Answer::Allow));
        assert!(
            lines.last().unwrap().0.starts_with("Enter = allow"),
            "{lines:?}"
        );

        let outside = Asker {
            name: "claude",
            place: Place::Outside,
        };
        let (lines, buttons) = question_box(&outside, true, None);
        assert!(lines[1].0.contains("not in a pane"), "{lines:?}");
        assert_eq!(lines[1].1, L::Fact);
        assert!(!lines.last().unwrap().0.contains("A or 2"), "no Always");
        let which: Vec<Button> = buttons.iter().map(|(b, _)| *b).collect();
        assert_eq!(which, [Button::Allow, Button::No]);
    }

    #[test]
    fn a_hook_of_the_config_says_so() {
        let hook = Asker {
            name: "on_agent_event",
            place: Place::Config,
        };
        let (lines, buttons) = question_box(&hook, false, None);
        assert!(lines[0].0.contains("on_agent_event"), "{lines:?}");
        assert!(
            !lines[0].0.contains("calls itself"),
            "fterm knows what it is: {lines:?}"
        );
        assert!(lines[1].0.contains("fterm.lua"), "{lines:?}");
        assert_eq!(lines[1].1, fterm_render::overlay::QuestionLook::Fact);
        let which: Vec<Button> = buttons.iter().map(|(b, _)| *b).collect();
        assert_eq!(which, [Button::Allow, Button::No], "no Always for a hook");
    }

    #[test]
    fn which_calls_of_a_hook_ask() {
        use fterm_config::keys::{BuiltinAction, SpawnWhere};
        use fterm_config::load::ApiCall;
        for call in [
            ApiCall::SendText("y\r".into()),
            ApiCall::Spawn {
                profile: None,
                place: SpawnWhere::Tab,
            },
            ApiCall::Run {
                steps: vec![vec!["cmd".into()]],
                overlay: false,
            },
            ApiCall::Action(BuiltinAction::Paste),
            ApiCall::CopyMode(fterm_config::copy_keys::CopyCommand::Exit),
            ApiCall::Open("file:///C:/x.exe".into()),
        ] {
            assert!(hook_call_gated(&call), "{call:?}");
        }
        for call in [
            ApiCall::Notify {
                title: "t".into(),
                body: "b".into(),
                level: "info".into(),
            },
            ApiCall::SetTabColor {
                pane: None,
                color: None,
            },
            ApiCall::Copy("text".into()),
        ] {
            assert!(!hook_call_gated(&call), "{call:?}");
        }
    }

    #[test]
    fn the_fact_says_the_workspace_and_a_pane_in_no_tab() {
        let fact = |place| {
            question_box(&Asker { name: "x", place }, false, None).0[1]
                .0
                .clone()
        };
        let other = fact(Place::Pane {
            tab: Some(2),
            workspace: Some("work"),
            program: "claude.exe",
        });
        assert!(other.contains("tab 2") && other.contains("work"), "{other}");
        let scratch = fact(Place::Pane {
            tab: None,
            workspace: None,
            program: "claude.exe",
        });
        assert!(
            scratch.contains("a pane of this window") && scratch.contains("claude.exe"),
            "{scratch}"
        );
        assert!(!scratch.contains("tab 0"), "{scratch}");
    }

    #[test]
    fn a_program_name_is_one_short_line() {
        let asker = Asker {
            name: "x",
            place: Place::Pane {
                tab: Some(1),
                workspace: None,
                program: "evil\nfterm checked: it is fine.exe and a very long name after that",
            },
        };
        let (lines, _) = question_box(&asker, false, None);
        assert!(!lines[1].0.contains('\n'), "{lines:?}");
        assert!(lines[1].0.chars().count() < 100, "{lines:?}");
    }
}
