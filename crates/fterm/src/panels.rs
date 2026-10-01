//! The dock model: which panel shows, the keyboard focus, the selected row, and the scroll.
//! Pure, without drawing (the drawing is in `fterm_render::dock`).

use std::time::{Duration, Instant};

use fterm_mux::PaneId;
use fterm_render::dock::{DockRow, DockSide};
use fterm_render::toasts::ToastLevel;
use fterm_term::alacritty_terminal::vte::ansi::Rgb;

use crate::agent::{AgentKind, AgentState, badge_color};
use crate::notify::{Level, Notification};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    Events,
    Agents,
    /// The AI chat (Phase 8).
    Ai,
}

impl PanelKind {
    pub const ALL: [PanelKind; 3] = [PanelKind::Events, PanelKind::Agents, PanelKind::Ai];

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "events" => Some(Self::Events),
            "agents" => Some(Self::Agents),
            "ai" => Some(Self::Ai),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Events => "Events",
            Self::Agents => "Agents",
            Self::Ai => "AI",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Events => 0,
            Self::Agents => 1,
            Self::Ai => 2,
        }
    }
}

/// Which events the Events panel shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EventFilter {
    #[default]
    All,
    /// Only warnings, errors, and "attention".
    Important,
}

impl EventFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Important,
            Self::Important => Self::All,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dock {
    pub open: bool,
    pub side: DockSide,
    /// The dock part of the window (0.1 ..= 0.9).
    pub ratio: f32,
    pub active: PanelKind,
    /// The keyboard goes to the dock, not to the terminal.
    pub focused: bool,
    pub filter: EventFilter,
    /// The selected row and the first row on the screen, for each panel.
    selected: [usize; 3],
    scroll: [usize; 3],
}

impl Dock {
    pub fn new(side: DockSide, ratio: f32, open: Option<PanelKind>) -> Self {
        Self {
            open: open.is_some(),
            side,
            ratio: ratio.clamp(0.1, 0.9),
            active: open.unwrap_or(PanelKind::Events),
            focused: false,
            filter: EventFilter::All,
            selected: [0; 3],
            scroll: [0; 3],
        }
    }

    /// The active panel is on the screen.
    pub fn showing(&self, kind: PanelKind) -> bool {
        self.open && self.active == kind
    }

    /// `toggle_dock`: open or close. It does not take the keyboard.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.focused = false;
    }

    /// `panel_events` and `panel_agents`: show the panel and take the keyboard.
    /// When it already has the keyboard, close the dock.
    pub fn show(&mut self, kind: PanelKind) {
        if self.showing(kind) && self.focused {
            self.open = false;
            self.focused = false;
        } else {
            self.open = true;
            self.active = kind;
            self.focused = true;
        }
    }

    /// `focus_dock`: move the keyboard between the terminal and the dock (opens it).
    pub fn toggle_focus(&mut self) {
        if self.open {
            self.focused = !self.focused;
        } else {
            self.open = true;
            self.focused = true;
        }
    }

    pub fn next_panel(&mut self) {
        let next = (self.active.index() + 1) % PanelKind::ALL.len();
        self.active = PanelKind::ALL[next];
    }

    pub fn selected(&self) -> usize {
        self.selected[self.active.index()]
    }

    pub fn scroll(&self) -> usize {
        self.scroll[self.active.index()]
    }

    /// Up / Down / Page keys: move the selection and keep it on the screen.
    pub fn move_by(&mut self, delta: isize, rows: usize, visible: usize) {
        let selected = self.selected().saturating_add_signed(delta);
        self.select(selected, rows, visible);
    }

    /// Select a row (a click), and keep it on the screen.
    pub fn select(&mut self, index: usize, rows: usize, visible: usize) {
        let i = self.active.index();
        if rows == 0 {
            self.selected[i] = 0;
            self.scroll[i] = 0;
            return;
        }
        let selected = index.min(rows - 1);
        let mut scroll = self.scroll[i].min(rows.saturating_sub(visible));
        if selected < scroll {
            scroll = selected;
        } else if visible > 0 && selected >= scroll + visible {
            scroll = selected + 1 - visible;
        }
        self.selected[i] = selected;
        self.scroll[i] = scroll;
    }

    /// The mouse wheel: scroll, but do not move the selection.
    pub fn scroll_by(&mut self, delta: isize, rows: usize, visible: usize) {
        let i = self.active.index();
        let max = rows.saturating_sub(visible);
        self.scroll[i] = self.scroll[i].saturating_add_signed(delta).min(max);
    }
}

/// "now", "40 s", "5 min", "2 h": how long ago, short.
pub fn short_ago(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..5 => "now".to_owned(),
        5..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        _ => format!("{} h", secs / 3600),
    }
}

/// The toast color of a level.
pub fn toast_level(level: Level) -> ToastLevel {
    match level {
        Level::Info => ToastLevel::Info,
        Level::Success => ToastLevel::Success,
        Level::Warning => ToastLevel::Warning,
        Level::Error => ToastLevel::Error,
        Level::Attention => ToastLevel::Attention,
    }
}

/// The color bar of a notification level (the same as the toasts).
pub fn level_color(level: Level) -> Rgb {
    toast_level(level).color()
}

/// The rows of the Events panel (newest first), and the pane of each row.
pub fn event_rows<'a>(
    history: impl Iterator<Item = &'a Notification>,
    filter: EventFilter,
    now: Instant,
) -> Vec<(DockRow, Option<PaneId>)> {
    history
        .filter(|n| match filter {
            EventFilter::All => true,
            EventFilter::Important => {
                matches!(n.level, Level::Warning | Level::Error | Level::Attention)
            }
        })
        .map(|n| {
            let detail = if n.body.trim().is_empty() {
                n.source.name().to_owned()
            } else {
                n.body.clone()
            };
            let row = DockRow {
                marker: level_color(n.level),
                title: n.title.clone(),
                detail,
                right: short_ago(now.saturating_duration_since(n.time)),
                new: !n.read,
            };
            (row, n.pane)
        })
        .collect()
}

/// A pane with an agent, for the Agents panel.
pub struct AgentEntry<'a> {
    pub pane: PaneId,
    /// The tab title.
    pub name: &'a str,
    pub state: &'a AgentState,
    /// Unread messages in its inbox.
    pub messages: usize,
}

pub fn agent_rows(entries: &[AgentEntry], now: Instant) -> Vec<(DockRow, Option<PaneId>)> {
    entries
        .iter()
        .map(|e| {
            let detail = if e.state.message.trim().is_empty() {
                match e.state.kind {
                    AgentKind::Working => "Working",
                    AgentKind::Waiting => "Waits for you",
                    AgentKind::Done => "Done",
                    AgentKind::Error => "Failed",
                }
                .to_owned()
            } else {
                e.state.message.clone()
            };
            let title = if e.messages > 0 {
                format!("{} · ✉ {}", e.name, e.messages)
            } else {
                e.name.to_owned()
            };
            let row = DockRow {
                marker: badge_color(e.state.kind),
                title,
                detail,
                right: short_ago(now.saturating_duration_since(e.state.since)),
                new: !e.state.seen,
            };
            (row, Some(e.pane))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::{Center, Source};

    #[test]
    fn short_times() {
        assert_eq!(short_ago(Duration::from_secs(2)), "now");
        assert_eq!(short_ago(Duration::from_secs(40)), "40 s");
        assert_eq!(short_ago(Duration::from_secs(300)), "5 min");
        assert_eq!(short_ago(Duration::from_secs(7300)), "2 h");
    }

    #[test]
    fn event_rows_are_newest_first_and_can_be_filtered() {
        let t0 = Instant::now();
        let mut center = Center::new(4, false);
        let pane = PaneId(7);
        center.push(
            t0,
            Some(pane),
            "Build",
            "All tests pass",
            Level::Success,
            Source::Command,
            false,
        );
        center.push(t0, None, "Oops", "", Level::Error, Source::App, false);
        let now = t0 + Duration::from_secs(120);
        let rows = event_rows(center.history(), EventFilter::All, now);
        assert_eq!(rows.len(), 2);
        let (oops, oops_pane) = &rows[0];
        assert_eq!(oops.title, "Oops");
        assert_eq!(oops.detail, "app", "no body: the source");
        assert_eq!(oops.marker, level_color(Level::Error));
        assert!(oops.new);
        assert_eq!(*oops_pane, None);
        let (build, build_pane) = &rows[1];
        assert_eq!(build.detail, "All tests pass");
        assert_eq!(build.right, "2 min");
        assert_eq!(*build_pane, Some(pane));

        let rows = event_rows(center.history(), EventFilter::Important, now);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.title, "Oops");

        center.mark_all_read();
        assert!(
            event_rows(center.history(), EventFilter::All, now)
                .iter()
                .all(|(r, _)| !r.new)
        );
    }

    #[test]
    fn agent_rows_show_the_state() {
        let t0 = Instant::now();
        let waiting = AgentState {
            kind: AgentKind::Waiting,
            message: String::new(),
            since: t0,
            seen: false,
        };
        let done = AgentState {
            kind: AgentKind::Done,
            message: "Tests are green".into(),
            since: t0,
            seen: true,
        };
        let entries = [
            AgentEntry {
                pane: PaneId(1),
                name: "claude",
                state: &waiting,
                messages: 0,
            },
            AgentEntry {
                pane: PaneId(2),
                name: "api",
                state: &done,
                messages: 2,
            },
        ];
        let rows = agent_rows(&entries, t0 + Duration::from_secs(30));
        assert_eq!(rows.len(), 2);
        let (row, pane) = &rows[0];
        assert_eq!(row.title, "claude");
        assert_eq!(row.detail, "Waits for you");
        assert_eq!(row.right, "30 s");
        assert_eq!(row.marker, badge_color(AgentKind::Waiting));
        assert!(row.new);
        assert_eq!(*pane, Some(PaneId(1)));
        assert_eq!(rows[1].0.detail, "Tests are green");
        assert_eq!(
            rows[1].0.title, "api · ✉ 2",
            "unread messages show in the title"
        );
        assert!(!rows[1].0.new);
    }

    fn dock() -> Dock {
        Dock::new(DockSide::Right, 0.3, None)
    }

    #[test]
    fn starts_closed_or_open_on_a_panel() {
        let d = dock();
        assert!(!d.open && !d.focused);
        let d = Dock::new(DockSide::Left, 0.3, Some(PanelKind::Agents));
        assert!(d.showing(PanelKind::Agents));
        assert!(!d.showing(PanelKind::Events));
        assert!(!d.focused, "the dock at start does not take the keyboard");
        assert_eq!(Dock::new(DockSide::Right, 5.0, None).ratio, 0.9);
    }

    #[test]
    fn toggle_opens_and_closes_without_the_keyboard() {
        let mut d = dock();
        d.toggle();
        assert!(d.open && !d.focused);
        d.focused = true;
        d.toggle();
        assert!(!d.open && !d.focused, "a closed dock has no keyboard");
    }

    #[test]
    fn show_a_panel() {
        let mut d = dock();
        d.show(PanelKind::Agents);
        assert!(d.showing(PanelKind::Agents) && d.focused);
        // The other panel: switch, keep the keyboard.
        d.show(PanelKind::Events);
        assert!(d.showing(PanelKind::Events) && d.focused);
        // The same panel with the keyboard: close.
        d.show(PanelKind::Events);
        assert!(!d.open && !d.focused);
        // Open (by toggle), without the keyboard: the key gives it the keyboard.
        d.toggle();
        d.show(PanelKind::Events);
        assert!(d.open && d.focused);
    }

    #[test]
    fn toggle_focus_opens_the_dock() {
        let mut d = dock();
        d.toggle_focus();
        assert!(d.open && d.focused);
        d.toggle_focus();
        assert!(
            d.open && !d.focused,
            "the dock stays, the terminal gets the keyboard"
        );
    }

    #[test]
    fn next_panel_goes_round() {
        let mut d = dock();
        d.show(PanelKind::Events);
        d.next_panel();
        assert_eq!(d.active, PanelKind::Agents);
        d.next_panel();
        assert_eq!(d.active, PanelKind::Ai);
        d.next_panel();
        assert_eq!(d.active, PanelKind::Events);
    }

    #[test]
    fn the_selection_stays_in_the_list_and_on_the_screen() {
        let mut d = dock();
        d.show(PanelKind::Events);
        d.move_by(-1, 10, 4);
        assert_eq!(d.selected(), 0);
        d.move_by(5, 10, 4);
        assert_eq!((d.selected(), d.scroll()), (5, 2));
        d.move_by(100, 10, 4);
        assert_eq!((d.selected(), d.scroll()), (9, 6));
        d.move_by(-8, 10, 4);
        assert_eq!((d.selected(), d.scroll()), (1, 1));
        // The list got shorter.
        d.move_by(0, 0, 4);
        assert_eq!((d.selected(), d.scroll()), (0, 0));
    }

    #[test]
    fn each_panel_keeps_its_selection() {
        let mut d = dock();
        d.show(PanelKind::Events);
        d.select(3, 10, 4);
        d.next_panel();
        assert_eq!(d.selected(), 0, "Agents has its own selection");
        d.next_panel();
        d.next_panel();
        assert_eq!(d.active, PanelKind::Events);
        assert_eq!(d.selected(), 3);
    }

    #[test]
    fn the_wheel_scrolls_inside_the_list() {
        let mut d = dock();
        d.show(PanelKind::Events);
        d.scroll_by(3, 10, 4);
        assert_eq!(
            (d.selected(), d.scroll()),
            (0, 3),
            "the selection does not move"
        );
        d.scroll_by(100, 10, 4);
        assert_eq!(d.scroll(), 6);
        d.scroll_by(-100, 10, 4);
        assert_eq!(d.scroll(), 0);
        d.scroll_by(3, 2, 4);
        assert_eq!(d.scroll(), 0, "all rows fit");
    }

    #[test]
    fn panel_names() {
        assert_eq!(PanelKind::from_name("events"), Some(PanelKind::Events));
        assert_eq!(PanelKind::from_name("agents"), Some(PanelKind::Agents));
        assert_eq!(PanelKind::from_name("ai"), Some(PanelKind::Ai));
        assert_eq!(PanelKind::Ai.label(), "AI");
        assert_eq!(PanelKind::from_name("x"), None);
        assert_eq!(EventFilter::All.next(), EventFilter::Important);
        assert_eq!(EventFilter::Important.next(), EventFilter::All);
    }
}
