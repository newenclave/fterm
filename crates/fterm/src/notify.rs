//! The notification center: all notifications (the history), and the toasts on the screen now.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use fterm_mux::PaneId;

/// How many notifications the history keeps.
pub const HISTORY: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
    /// The user must do something (for example, Claude waits for an answer).
    Attention,
}

impl Level {
    /// How long a toast stays. `None` = until the user closes it.
    pub fn timeout(self) -> Option<Duration> {
        match self {
            Level::Info | Level::Success => Some(Duration::from_secs(4)),
            Level::Warning => Some(Duration::from_secs(8)),
            Level::Error | Level::Attention => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Success => "success",
            Level::Warning => "warning",
            Level::Error => "error",
            Level::Attention => "attention",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [
            Level::Info,
            Level::Success,
            Level::Warning,
            Level::Error,
            Level::Attention,
        ]
        .into_iter()
        .find(|l| l.name() == name)
    }
}

/// Where a notification comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// An app in a pane (OSC 9 / 99 / 777).
    Terminal,
    /// A long command ended.
    Command,
    /// An agent changed its state (Claude, ...).
    Agent,
    /// The config (`fterm.notify`).
    Lua,
    /// fterm itself (for example, a config error).
    App,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Terminal => "terminal",
            Source::Command => "command",
            Source::Agent => "agent",
            Source::Lua => "lua",
            Source::App => "app",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub id: u64,
    pub pane: Option<PaneId>,
    pub title: String,
    pub body: String,
    pub level: Level,
    pub source: Source,
    pub time: Instant,
    pub read: bool,
}

/// A toast on the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    /// When it hides. `None` = it stays (or the mouse is over it).
    pub hide_at: Option<Instant>,
    /// Time left, saved while the mouse is over the toast.
    paused_left: Option<Duration>,
}

pub struct Center {
    history: VecDeque<Notification>,
    /// Toasts on the screen, oldest first.
    toasts: Vec<Toast>,
    next_id: u64,
    pub max_toasts: usize,
    pub toasts_on: bool,
}

impl Center {
    pub fn new(max_toasts: usize, toasts_on: bool) -> Self {
        Self {
            history: VecDeque::new(),
            toasts: Vec::new(),
            next_id: 0,
            max_toasts,
            toasts_on,
        }
    }

    /// Adds a notification. With `toast`, it also shows as a toast. Returns its id.
    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &mut self,
        now: Instant,
        pane: Option<PaneId>,
        title: &str,
        body: &str,
        level: Level,
        source: Source,
        toast: bool,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.history.push_front(Notification {
            id,
            pane,
            title: title.to_owned(),
            body: body.to_owned(),
            level,
            source,
            time: now,
            read: false,
        });
        self.history.truncate(HISTORY);
        if toast && self.toasts_on && self.max_toasts > 0 {
            self.toasts.push(Toast {
                id,
                hide_at: level.timeout().map(|t| now + t),
                paused_left: None,
            });
            let extra = self.toasts.len().saturating_sub(self.max_toasts);
            self.toasts.drain(..extra);
        }
        id
    }

    /// Hides toasts whose time is over.
    pub fn tick(&mut self, now: Instant) {
        self.toasts
            .retain(|t| t.paused_left.is_some() || t.hide_at.is_none_or(|at| now < at));
    }

    /// The mouse is over this toast (`None` = over no toast). The timer stops while the mouse is there.
    pub fn hover(&mut self, id: Option<u64>, now: Instant) {
        for toast in &mut self.toasts {
            let over = Some(toast.id) == id;
            match (over, toast.paused_left) {
                // The mouse comes: save the time left and stop the timer.
                (true, None) => {
                    if let Some(at) = toast.hide_at.take() {
                        toast.paused_left = Some(at.saturating_duration_since(now));
                    }
                }
                // The mouse leaves: start the timer again with the time that was left.
                (false, Some(left)) => {
                    toast.hide_at = Some(now + left);
                    toast.paused_left = None;
                }
                _ => {}
            }
        }
    }

    /// Closes a toast. The notification stays in the history.
    pub fn dismiss(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
    }

    pub fn toasts(&self) -> &[Toast] {
        &self.toasts
    }

    /// The history, newest first.
    pub fn history(&self) -> impl Iterator<Item = &Notification> {
        self.history.iter()
    }

    pub fn get(&self, id: u64) -> Option<&Notification> {
        self.history.iter().find(|n| n.id == id)
    }

    pub fn unread(&self) -> usize {
        self.history.iter().filter(|n| !n.read).count()
    }

    pub fn mark_all_read(&mut self) {
        for n in &mut self.history {
            n.read = true;
        }
    }

    /// When the next toast hides (for the event loop to wake up).
    pub fn next_deadline(&self) -> Option<Instant> {
        self.toasts.iter().filter_map(|t| t.hide_at).min()
    }
}

/// A time for people: "8 s", "1 min 3 s", "2 h 5 min".
pub fn human_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    match (h, m, s) {
        (0, 0, s) => format!("{s} s"),
        (0, m, 0) => format!("{m} min"),
        (0, m, s) => format!("{m} min {s} s"),
        (h, m, _) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_for_people() {
        assert_eq!(human_duration(Duration::from_millis(8_400)), "8 s");
        assert_eq!(human_duration(Duration::from_secs(63)), "1 min 3 s");
        assert_eq!(human_duration(Duration::from_secs(120)), "2 min");
        assert_eq!(
            human_duration(Duration::from_secs(2 * 3600 + 5 * 60 + 9)),
            "2 h 5 min"
        );
    }

    fn push(center: &mut Center, now: Instant, title: &str, level: Level) -> u64 {
        center.push(now, None, title, "body", level, Source::Terminal, true)
    }

    fn toast_titles(center: &Center) -> Vec<String> {
        center
            .toasts()
            .iter()
            .map(|t| center.get(t.id).unwrap().title.clone())
            .collect()
    }

    #[test]
    fn a_notification_goes_into_the_history_and_a_toast() {
        let mut center = Center::new(4, true);
        let now = Instant::now();
        let id = push(&mut center, now, "hello", Level::Info);
        assert_eq!(center.toasts().len(), 1);
        assert_eq!(center.history().count(), 1);
        assert_eq!(center.get(id).unwrap().title, "hello");
        assert_eq!(center.unread(), 1);
    }

    #[test]
    fn no_toast_when_toasts_are_off_or_not_wanted() {
        let mut center = Center::new(4, false);
        let now = Instant::now();
        push(&mut center, now, "a", Level::Info);
        assert!(center.toasts().is_empty());
        assert_eq!(center.history().count(), 1);
        let mut center = Center::new(4, true);
        center.push(now, None, "b", "", Level::Info, Source::Lua, false);
        assert!(center.toasts().is_empty());
    }

    #[test]
    fn the_stack_keeps_only_the_newest_toasts() {
        let mut center = Center::new(2, true);
        let now = Instant::now();
        for name in ["a", "b", "c"] {
            push(&mut center, now, name, Level::Error);
        }
        assert_eq!(toast_titles(&center), ["b", "c"]);
        // The old one is still in the history.
        assert_eq!(center.history().count(), 3);
        assert_eq!(center.history().next().unwrap().title, "c", "newest first");
    }

    #[test]
    fn toasts_hide_by_level() {
        let mut center = Center::new(4, true);
        let t0 = Instant::now();
        push(&mut center, t0, "info", Level::Info);
        push(&mut center, t0, "warn", Level::Warning);
        push(&mut center, t0, "error", Level::Error);
        center.tick(t0 + Duration::from_secs(5));
        assert_eq!(toast_titles(&center), ["warn", "error"]);
        center.tick(t0 + Duration::from_secs(9));
        assert_eq!(toast_titles(&center), ["error"]);
        center.tick(t0 + Duration::from_secs(3600));
        assert_eq!(toast_titles(&center), ["error"], "errors stay until closed");
    }

    #[test]
    fn the_mouse_over_a_toast_stops_its_timer() {
        let mut center = Center::new(4, true);
        let t0 = Instant::now();
        let id = push(&mut center, t0, "info", Level::Info);
        center.hover(Some(id), t0 + Duration::from_secs(3));
        center.tick(t0 + Duration::from_secs(60));
        assert_eq!(center.toasts().len(), 1, "the mouse is still there");
        // The mouse leaves: 1 second was left.
        center.hover(None, t0 + Duration::from_secs(60));
        center.tick(t0 + Duration::from_millis(60_500));
        assert_eq!(center.toasts().len(), 1);
        center.tick(t0 + Duration::from_millis(61_100));
        assert!(center.toasts().is_empty());
    }

    #[test]
    fn dismiss_and_read() {
        let mut center = Center::new(4, true);
        let now = Instant::now();
        let id = push(&mut center, now, "x", Level::Attention);
        center.dismiss(id);
        assert!(center.toasts().is_empty());
        assert_eq!(center.unread(), 1);
        center.mark_all_read();
        assert_eq!(center.unread(), 0);
    }

    #[test]
    fn history_has_a_limit() {
        let mut center = Center::new(4, false);
        let now = Instant::now();
        for i in 0..HISTORY + 10 {
            push(&mut center, now, &i.to_string(), Level::Info);
        }
        assert_eq!(center.history().count(), HISTORY);
    }

    #[test]
    fn next_deadline_is_the_first_toast_to_hide() {
        let mut center = Center::new(4, true);
        let t0 = Instant::now();
        assert_eq!(center.next_deadline(), None);
        push(&mut center, t0, "warn", Level::Warning);
        push(&mut center, t0, "info", Level::Info);
        assert_eq!(center.next_deadline(), Some(t0 + Duration::from_secs(4)));
    }

    #[test]
    fn level_names() {
        assert_eq!(Level::from_name("attention"), Some(Level::Attention));
        assert_eq!(Level::from_name("loud"), None);
        assert_eq!(Level::Warning.name(), "warning");
    }
}
