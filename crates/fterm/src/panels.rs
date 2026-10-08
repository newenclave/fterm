//! The dock model: which panel shows, the keyboard focus, the selected row, and the scroll.
//! Pure, without drawing (the drawing is in `fterm_render::dock`).

use std::time::{Duration, Instant};

use fterm_mux::PaneId;
use fterm_render::dock::{DockRow, DockSide};
use fterm_render::theme::UiColors;
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

    pub fn label(self) -> String {
        match self {
            Self::Events => fterm_config::tr!("panel.events"),
            Self::Agents => fterm_config::tr!("panel.agents"),
            Self::Ai => fterm_config::tr!("panel.ai"),
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
    /// The event whose full text is open in the Events panel, and its first line on the screen.
    reading: Option<u64>,
    read_scroll: usize,
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
            reading: None,
            read_scroll: 0,
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
        self.close_reader();
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

    /// Opens the full text of an event (its id), from the top.
    pub fn open_reader(&mut self, id: u64) {
        self.reading = Some(id);
        self.read_scroll = 0;
    }

    /// Back from the full text to the list.
    pub fn close_reader(&mut self) {
        self.reading = None;
        self.read_scroll = 0;
    }

    /// The event whose full text is open.
    pub fn reading(&self) -> Option<u64> {
        self.reading
    }

    /// The first line of the full text on the screen.
    pub fn reader_scroll(&self) -> usize {
        self.read_scroll
    }

    /// Scrolls the full text: `lines` in it, `visible` on the screen.
    pub fn scroll_reader(&mut self, delta: isize, lines: usize, visible: usize) {
        let max = lines.saturating_sub(visible);
        self.read_scroll = self.read_scroll.saturating_add_signed(delta).min(max);
    }

    /// The mouse wheel: scroll, but do not move the selection.
    pub fn scroll_by(&mut self, delta: isize, rows: usize, visible: usize) {
        let i = self.active.index();
        let max = rows.saturating_sub(visible);
        self.scroll[i] = self.scroll[i].saturating_add_signed(delta).min(max);
    }
}

/// The events in the list with this filter, newest first, as the Events panel shows them.
pub fn filtered_events<'a>(
    history: impl Iterator<Item = &'a Notification>,
    filter: EventFilter,
) -> Vec<&'a Notification> {
    history
        .filter(|n| match filter {
            EventFilter::All => true,
            EventFilter::Important => {
                matches!(n.level, Level::Warning | Level::Error | Level::Attention)
            }
        })
        .collect()
}

/// The full text of an event, wrapped to `width` cells: the title, where it came from, and the body.
pub fn event_lines(
    n: &Notification,
    place: Option<&str>,
    now: Instant,
    width: usize,
) -> Vec<fterm_render::dock::ChatLine> {
    use fterm_render::dock::{ChatLine, ChatStyle};
    let width = width.max(8);
    let line = |text: String, style| ChatLine { text, style };
    let mut out: Vec<ChatLine> = crate::ai_chat::wrap(&n.title, width)
        .into_iter()
        .map(|t| line(t, ChatStyle::User))
        .collect();
    let ago = long_ago(now.saturating_duration_since(n.time), false);
    let mut meta = fterm_config::tr!(
        "reader.meta",
        level = n.level.label(),
        source = n.source.label(),
        ago = ago
    );
    if let Some(place) = place {
        meta.push_str(&format!(" · {place}"));
    }
    out.extend(
        crate::ai_chat::wrap(&meta, width)
            .into_iter()
            .map(|t| line(t, ChatStyle::Note)),
    );
    out.push(line(String::new(), ChatStyle::Note));
    if n.body.trim().is_empty() {
        out.push(line(fterm_config::tr!("reader.no_text"), ChatStyle::Note));
    }
    for text in n.body.trim_end().lines() {
        if text.trim().is_empty() {
            out.push(line(String::new(), ChatStyle::Answer));
        } else {
            out.extend(
                crate::ai_chat::wrap(text, width)
                    .into_iter()
                    .map(|t| line(t, ChatStyle::Answer)),
            );
        }
    }
    out
}

/// "now", "40 s", "5 min", "2 h": how long ago, short.
pub fn short_ago(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0..5 => fterm_config::tr!("time.now"),
        5..60 => fterm_config::tr!("time.s", s = secs),
        60..3600 => fterm_config::tr!("time.min", m = secs / 60),
        _ => fterm_config::tr!("time.h", h = secs / 3600),
    }
}

/// "now" (or "just now" when `just`), or "5 min ago".
pub fn long_ago(d: Duration, just: bool) -> String {
    match (d.as_secs(), just) {
        (0..5, true) => fterm_config::tr!("time.just_now"),
        (0..5, false) => fterm_config::tr!("time.now"),
        _ => fterm_config::tr!("time.ago", time = short_ago(d)),
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
pub fn level_color(level: Level, ui: &UiColors) -> Rgb {
    toast_level(level).color(ui)
}

/// The rows of the Events panel (newest first), and the pane of each row.
pub fn event_rows<'a>(
    history: impl Iterator<Item = &'a Notification>,
    filter: EventFilter,
    now: Instant,
    ui: &UiColors,
) -> Vec<(DockRow, Option<PaneId>)> {
    filtered_events(history, filter)
        .into_iter()
        .map(|n| {
            let detail = if n.body.trim().is_empty() {
                n.source.name().to_owned()
            } else {
                n.body.clone()
            };
            let row = DockRow {
                marker: level_color(n.level, ui),
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

pub fn agent_rows(
    entries: &[AgentEntry],
    now: Instant,
    ui: &UiColors,
) -> Vec<(DockRow, Option<PaneId>)> {
    entries
        .iter()
        .map(|e| {
            let detail = if e.state.message.trim().is_empty() {
                match e.state.kind {
                    AgentKind::Working => fterm_config::tr!("state.working"),
                    AgentKind::Waiting => fterm_config::tr!("state.waiting"),
                    AgentKind::Done => fterm_config::tr!("state.done"),
                    AgentKind::Error => fterm_config::tr!("state.error"),
                }
            } else {
                e.state.message.clone()
            };
            let title = if e.messages > 0 {
                format!("{} · ✉ {}", e.name, e.messages)
            } else {
                e.name.to_owned()
            };
            let row = DockRow {
                marker: badge_color(e.state.kind, ui),
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
        let rows = event_rows(
            center.history(),
            EventFilter::All,
            now,
            &UiColors::default(),
        );
        assert_eq!(rows.len(), 2);
        let (oops, oops_pane) = &rows[0];
        assert_eq!(oops.title, "Oops");
        assert_eq!(oops.detail, "app", "no body: the source");
        assert_eq!(oops.marker, level_color(Level::Error, &UiColors::default()));
        assert!(oops.new);
        assert_eq!(*oops_pane, None);
        let (build, build_pane) = &rows[1];
        assert_eq!(build.detail, "All tests pass");
        assert_eq!(build.right, "2 min");
        assert_eq!(*build_pane, Some(pane));

        let rows = event_rows(
            center.history(),
            EventFilter::Important,
            now,
            &UiColors::default(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.title, "Oops");

        center.mark_all_read();
        assert!(
            event_rows(
                center.history(),
                EventFilter::All,
                now,
                &UiColors::default()
            )
            .iter()
            .all(|(r, _)| !r.new)
        );
    }

    #[test]
    fn the_full_text_of_an_event() {
        use fterm_render::dock::ChatStyle;
        let t0 = Instant::now();
        let mut center = Center::new(4, false);
        let body = "first line\nsecond line is a long line that does not fit in twenty cells";
        center.push(
            t0,
            Some(PaneId(5)),
            "Message for Claude",
            body,
            Level::Attention,
            Source::Api,
            false,
        );
        let n = center.history().next().unwrap();
        let lines = event_lines(n, Some("tab 2, pane 5"), t0 + Duration::from_secs(420), 20);
        assert_eq!(lines[0].text, "Message for Claude");
        assert_eq!(lines[0].style, ChatStyle::User, "the title stands out");
        assert_eq!(lines[1].style, ChatStyle::Note);
        // Where it came from: the level, the source, the time, and the pane (wrapped too).
        let meta: String = lines
            .iter()
            .filter(|l| l.style == ChatStyle::Note)
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for part in ["attention", "api", "7 min ago", "tab 2,", "pane 5"] {
            assert!(meta.contains(part), "{part}: {meta}");
        }
        let body_lines: Vec<&str> = lines
            .iter()
            .filter(|l| l.style == ChatStyle::Answer)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(body_lines[0], "first line");
        assert!(
            body_lines.len() >= 4,
            "the long line is wrapped: {body_lines:?}"
        );
        assert!(body_lines.iter().all(|l| l.chars().count() <= 20));
        // No body: a note says so.
        center.push(t0, None, "Bell", "", Level::Info, Source::Terminal, false);
        let bell = center.history().next().unwrap();
        let lines = event_lines(bell, None, t0, 40);
        assert!(lines.iter().any(|l| l.text.contains("no text")));
    }

    #[test]
    fn the_event_of_a_row() {
        let t0 = Instant::now();
        let mut center = Center::new(4, false);
        center.push(t0, None, "a", "", Level::Info, Source::App, false);
        center.push(t0, None, "b", "", Level::Error, Source::App, false);
        center.push(t0, None, "c", "", Level::Info, Source::App, false);
        let all = filtered_events(center.history(), EventFilter::All);
        let titles: Vec<&str> = all.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(titles, ["c", "b", "a"], "the same order as the rows");
        let important = filtered_events(center.history(), EventFilter::Important);
        assert_eq!(important.len(), 1);
        assert_eq!(important[0].title, "b");
    }

    #[test]
    fn the_reader_opens_scrolls_and_closes() {
        let mut dock = Dock::new(DockSide::Right, 0.3, Some(PanelKind::Events));
        assert_eq!(dock.reading(), None);
        dock.open_reader(42);
        assert_eq!(dock.reading(), Some(42));
        assert_eq!(dock.reader_scroll(), 0);
        // 30 lines, 10 on the screen: the last screen starts at line 20.
        dock.scroll_reader(5, 30, 10);
        assert_eq!(dock.reader_scroll(), 5);
        dock.scroll_reader(100, 30, 10);
        assert_eq!(dock.reader_scroll(), 20);
        dock.scroll_reader(-100, 30, 10);
        assert_eq!(dock.reader_scroll(), 0);
        dock.scroll_reader(3, 5, 10);
        assert_eq!(dock.reader_scroll(), 0, "it all fits");
        // Another event starts at the top.
        dock.scroll_reader(4, 30, 10);
        dock.open_reader(43);
        assert_eq!(dock.reader_scroll(), 0);
        dock.close_reader();
        assert_eq!(dock.reading(), None);
        // Another panel closes it too.
        dock.open_reader(43);
        dock.next_panel();
        assert_eq!(dock.reading(), None);
    }

    #[test]
    fn agent_rows_show_the_state() {
        let t0 = Instant::now();
        let waiting = AgentState {
            kind: AgentKind::Waiting,
            message: String::new(),
            since: t0,
            seen: false,
            auto: false,
        };
        let done = AgentState {
            kind: AgentKind::Done,
            message: "Tests are green".into(),
            since: t0,
            seen: true,
            auto: false,
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
        let rows = agent_rows(&entries, t0 + Duration::from_secs(30), &UiColors::default());
        assert_eq!(rows.len(), 2);
        let (row, pane) = &rows[0];
        assert_eq!(row.title, "claude");
        assert_eq!(row.detail, "Waits for you");
        assert_eq!(row.right, "30 s");
        assert_eq!(
            row.marker,
            badge_color(AgentKind::Waiting, &UiColors::default())
        );
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
