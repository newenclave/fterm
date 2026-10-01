//! Tabs: which tabs there are, which one is active, and their titles.

use crate::layout::{Direction, Edge, Layout, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

#[derive(Debug)]
pub struct Tab {
    pub id: TabId,
    pub layout: Layout,
    /// The pane that gets the keys.
    pub active_pane: PaneId,
    /// A name from the user. `None` = the title comes from the app or the program.
    pub custom_title: Option<String>,
    /// One pane that takes the whole tab for now (zoom).
    pub zoomed: Option<PaneId>,
}

/// What `close_pane` closed.
#[derive(Debug, PartialEq, Eq)]
pub enum Closed {
    /// Only the pane. The tab has other panes.
    Pane,
    /// The pane and its tab.
    Tab,
    /// The last tab. The window can close now.
    LastTab,
    /// The pane was not found.
    Nothing,
}

#[derive(Default)]
pub struct Mux {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
}

impl Mux {
    /// A new id for a pane. Ids are never used twice.
    pub fn new_pane_id(&mut self) -> PaneId {
        PaneId(self.next())
    }

    fn next(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Opens a tab with one pane, after the active tab, and makes it active.
    pub fn new_tab(&mut self, pane: PaneId) -> TabId {
        let id = TabId(self.next());
        let tab = Tab {
            id,
            layout: Layout::Pane(pane),
            active_pane: pane,
            custom_title: None,
            zoomed: None,
        };
        let at = if self.tabs.is_empty() {
            0
        } else {
            self.active + 1
        };
        self.tabs.insert(at, tab);
        self.active = at;
        id
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_pane(&self) -> Option<PaneId> {
        self.active_tab().map(|tab| tab.active_pane)
    }

    /// The tab that has this pane.
    pub fn pane_tab(&self, pane: PaneId) -> Option<TabId> {
        self.tabs
            .iter()
            .find(|tab| tab.layout.contains(pane))
            .map(|tab| tab.id)
    }

    fn tab_index(&self, tab: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == tab)
    }

    /// Closes a pane (for example, its shell ended).
    pub fn close_pane(&mut self, pane: PaneId) -> Closed {
        let Some(index) = self.tabs.iter().position(|t| t.layout.contains(pane)) else {
            return Closed::Nothing;
        };
        let tab = &mut self.tabs[index];
        if let Some(neighbor) = tab.layout.remove(pane) {
            if tab.active_pane == pane {
                tab.active_pane = neighbor;
            }
            if tab.zoomed == Some(pane) {
                tab.zoomed = None;
            }
            return Closed::Pane;
        }
        self.remove_tab_at(index);
        if self.tabs.is_empty() {
            Closed::LastTab
        } else {
            Closed::Tab
        }
    }

    /// Closes a tab and returns its panes (to stop their sessions).
    pub fn close_tab(&mut self, tab: TabId) -> Vec<PaneId> {
        match self.tab_index(tab) {
            Some(index) => self.remove_tab_at(index).layout.panes(),
            None => Vec::new(),
        }
    }

    fn remove_tab_at(&mut self, index: usize) -> Tab {
        let tab = self.tabs.remove(index);
        // The right neighbor gets the place; at the end it is the left neighbor.
        if index < self.active || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        tab
    }

    pub fn select(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    pub fn select_last(&mut self) {
        self.active = self.tabs.len().saturating_sub(1);
    }

    /// The next tab (`+1`) or the previous one (`-1`). It wraps around.
    pub fn cycle(&mut self, step: i32) {
        let len = self.tabs.len() as i32;
        if len > 0 {
            self.active = (self.active as i32 + step).rem_euclid(len) as usize;
        }
    }

    /// Moves the active tab left (`-1`) or right (`+1`). It stops at the edges.
    pub fn move_active(&mut self, step: i32) {
        let target = self.active as i32 + step;
        if target >= 0 && (target as usize) < self.tabs.len() {
            self.tabs.swap(self.active, target as usize);
            self.active = target as usize;
        }
    }

    /// Splits the active pane. `new` gets the second half and the focus.
    pub fn split_active(&mut self, new: PaneId, direction: Direction) {
        let active = self.active;
        if let Some(tab) = self.tabs.get_mut(active)
            && tab.layout.split(tab.active_pane, new, direction)
        {
            tab.active_pane = new;
            tab.zoomed = None;
        }
    }

    /// Gives the focus to `pane` (in the active tab).
    pub fn focus(&mut self, pane: PaneId) {
        let active = self.active;
        if let Some(tab) = self.tabs.get_mut(active)
            && tab.layout.contains(pane)
        {
            tab.active_pane = pane;
            if tab.zoomed.is_some_and(|z| z != pane) {
                tab.zoomed = None;
            }
        }
    }

    /// Gives the focus to the neighbor of the active pane toward `edge`.
    pub fn focus_direction(&mut self, edge: Edge, area: Rect) {
        let active = self.active;
        let Some(tab) = self.tabs.get(active) else {
            return;
        };
        if let Some(neighbor) = tab.layout.neighbor(tab.active_pane, edge, area) {
            self.focus(neighbor);
        }
    }

    /// The active pane takes the whole tab, or the tab goes back to all panes.
    pub fn toggle_zoom(&mut self) {
        let active = self.active;
        if let Some(tab) = self.tabs.get_mut(active) {
            tab.zoomed = match tab.zoomed {
                Some(_) => None,
                None if tab.layout.panes().len() > 1 => Some(tab.active_pane),
                None => None,
            };
        }
    }

    /// The panes of the active tab that are seen now, and their places in `area`.
    pub fn pane_rects(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        match self.active_tab() {
            Some(tab) => match tab.zoomed {
                Some(pane) => vec![(pane, area)],
                None => tab.layout.rects(area),
            },
            None => Vec::new(),
        }
    }

    pub fn active_layout_mut(&mut self) -> Option<&mut Layout> {
        let active = self.active;
        self.tabs.get_mut(active).map(|tab| &mut tab.layout)
    }

    /// Sets the user's name for a tab. An empty name goes back to the auto title.
    pub fn rename(&mut self, tab: TabId, name: &str) {
        if let Some(index) = self.tab_index(tab) {
            let name = name.trim();
            self.tabs[index].custom_title = (!name.is_empty()).then(|| name.to_owned());
        }
    }
}

/// The title of a tab: the user's name, else the title from the app (OSC 0/2), else the program name.
pub fn tab_title<'a>(
    custom: Option<&'a str>,
    app_title: Option<&'a str>,
    program: &'a str,
) -> &'a str {
    custom
        .or(app_title
            .filter(|title| !title.trim().is_empty())
            .map(program_from_path))
        .unwrap_or(program)
}

/// `C:\a\b\powershell.exe` -> `powershell`. Other titles do not change.
fn program_from_path(title: &str) -> &str {
    let is_exe_path = title.contains('\\')
        && !title.contains(' ')
        && title.len() > 4
        && title[title.len() - 4..].eq_ignore_ascii_case(".exe");
    if !is_exe_path {
        return title;
    }
    let file = title.rsplit('\\').next().unwrap_or(title);
    &file[..file.len() - 4]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mux with `n` tabs; the first tab is active.
    fn mux_with(n: usize) -> (Mux, Vec<PaneId>) {
        let mut mux = Mux::default();
        let mut panes = Vec::new();
        for _ in 0..n {
            let pane = mux.new_pane_id();
            mux.new_tab(pane);
            panes.push(pane);
        }
        mux.select(0);
        (mux, panes)
    }

    fn order(mux: &Mux) -> Vec<PaneId> {
        mux.tabs().iter().map(|t| t.active_pane).collect()
    }

    #[test]
    fn ids_are_never_used_twice() {
        let mut mux = Mux::default();
        let a = mux.new_pane_id();
        let b = mux.new_pane_id();
        assert_ne!(a, b);
        let t1 = mux.new_tab(a);
        let t2 = mux.new_tab(b);
        assert_ne!(t1, t2);
    }

    #[test]
    fn new_tab_goes_after_the_active_one_and_is_active() {
        let (mut mux, panes) = mux_with(3);
        let new = mux.new_pane_id();
        mux.new_tab(new);
        assert_eq!(order(&mux), [panes[0], new, panes[1], panes[2]]);
        assert_eq!(mux.active_index(), 1);
        assert_eq!(mux.active_pane(), Some(new));
    }

    #[test]
    fn closing_the_active_tab_selects_the_right_neighbor() {
        let (mut mux, panes) = mux_with(3);
        mux.select(1);
        assert_eq!(mux.close_pane(panes[1]), Closed::Tab);
        assert_eq!(mux.active_pane(), Some(panes[2]));
    }

    #[test]
    fn closing_the_last_tab_in_the_row_selects_the_left_neighbor() {
        let (mut mux, panes) = mux_with(3);
        mux.select(2);
        mux.close_pane(panes[2]);
        assert_eq!(mux.active_pane(), Some(panes[1]));
    }

    #[test]
    fn closing_another_tab_keeps_the_active_tab() {
        let (mut mux, panes) = mux_with(3);
        mux.select(2);
        mux.close_pane(panes[0]);
        assert_eq!(mux.active_pane(), Some(panes[2]));
    }

    #[test]
    fn closing_the_only_tab_is_the_last_tab() {
        let (mut mux, panes) = mux_with(1);
        assert_eq!(mux.close_pane(panes[0]), Closed::LastTab);
        assert!(mux.tabs().is_empty());
        assert_eq!(mux.active_pane(), None);
        assert_eq!(mux.close_pane(panes[0]), Closed::Nothing);
    }

    #[test]
    fn close_tab_returns_its_panes() {
        let (mut mux, panes) = mux_with(2);
        let tab = mux.pane_tab(panes[1]).unwrap();
        assert_eq!(mux.close_tab(tab), [panes[1]]);
        assert_eq!(mux.tabs().len(), 1);
    }

    #[test]
    fn cycle_wraps_around() {
        let (mut mux, panes) = mux_with(3);
        mux.cycle(-1);
        assert_eq!(mux.active_pane(), Some(panes[2]));
        mux.cycle(1);
        assert_eq!(mux.active_pane(), Some(panes[0]));
    }

    #[test]
    fn select_out_of_range_does_nothing() {
        let (mut mux, panes) = mux_with(2);
        mux.select(5);
        assert_eq!(mux.active_pane(), Some(panes[0]));
        mux.select_last();
        assert_eq!(mux.active_pane(), Some(panes[1]));
    }

    #[test]
    fn move_active_tab_stops_at_the_edges() {
        let (mut mux, panes) = mux_with(3);
        mux.move_active(-1);
        assert_eq!(order(&mux), panes);
        mux.move_active(1);
        assert_eq!(order(&mux), [panes[1], panes[0], panes[2]]);
        assert_eq!(mux.active_index(), 1, "the moved tab stays active");
        mux.move_active(1);
        mux.move_active(1);
        assert_eq!(order(&mux), [panes[1], panes[2], panes[0]]);
    }

    #[test]
    fn rename_and_back_to_auto() {
        let (mut mux, panes) = mux_with(1);
        let tab = mux.pane_tab(panes[0]).unwrap();
        mux.rename(tab, "  build  ");
        assert_eq!(mux.tabs()[0].custom_title.as_deref(), Some("build"));
        mux.rename(tab, "   ");
        assert_eq!(mux.tabs()[0].custom_title, None);
    }

    #[test]
    fn title_rule() {
        assert_eq!(tab_title(Some("mine"), Some("app"), "pwsh"), "mine");
        assert_eq!(tab_title(None, Some("app"), "pwsh"), "app");
        assert_eq!(tab_title(None, Some(""), "pwsh"), "pwsh");
        assert_eq!(tab_title(None, None, "pwsh"), "pwsh");
    }

    #[test]
    fn a_title_that_is_only_a_program_path_shows_the_program_name() {
        // PowerShell sets its title to the full path of its exe.
        let path = r"C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe";
        assert_eq!(tab_title(None, Some(path), "x"), "powershell");
        assert_eq!(tab_title(None, Some(r"C:\Tools\NODE.EXE"), "x"), "NODE");
        // Other titles stay as they are.
        assert_eq!(
            tab_title(None, Some("user@host: ~/code"), "x"),
            "user@host: ~/code"
        );
        assert_eq!(tab_title(None, Some("vim notes.txt"), "x"), "vim notes.txt");
    }

    const AREA: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };

    #[test]
    fn split_gives_the_focus_to_the_new_pane() {
        let (mut mux, panes) = mux_with(1);
        let new = mux.new_pane_id();
        mux.split_active(new, Direction::Right);
        assert_eq!(mux.active_pane(), Some(new));
        assert_eq!(mux.tabs()[0].layout.panes(), [panes[0], new]);
        assert_eq!(mux.pane_tab(new), Some(mux.tabs()[0].id));
    }

    #[test]
    fn closing_the_active_pane_focuses_the_neighbor() {
        let (mut mux, panes) = mux_with(1);
        let b = mux.new_pane_id();
        mux.split_active(b, Direction::Right);
        assert_eq!(mux.close_pane(b), Closed::Pane);
        assert_eq!(mux.active_pane(), Some(panes[0]));
        assert_eq!(mux.close_pane(panes[0]), Closed::LastTab);
    }

    #[test]
    fn focus_moves_between_neighbors() {
        let (mut mux, panes) = mux_with(1);
        let b = mux.new_pane_id();
        mux.split_active(b, Direction::Right);
        mux.focus_direction(Edge::Left, AREA);
        assert_eq!(mux.active_pane(), Some(panes[0]));
        // Nothing on the left: the focus stays.
        mux.focus_direction(Edge::Left, AREA);
        assert_eq!(mux.active_pane(), Some(panes[0]));
        mux.focus(b);
        assert_eq!(mux.active_pane(), Some(b));
        // A pane of another tab cannot get the focus here.
        mux.focus(PaneId(999));
        assert_eq!(mux.active_pane(), Some(b));
    }

    #[test]
    fn zoom_shows_only_the_active_pane() {
        let (mut mux, panes) = mux_with(1);
        let b = mux.new_pane_id();
        mux.split_active(b, Direction::Right);
        assert_eq!(mux.pane_rects(AREA).len(), 2);
        mux.toggle_zoom();
        assert_eq!(mux.pane_rects(AREA), [(b, AREA)]);
        mux.toggle_zoom();
        assert_eq!(mux.pane_rects(AREA).len(), 2);
        // A split or a focus change ends the zoom.
        mux.toggle_zoom();
        mux.focus(panes[0]);
        assert_eq!(mux.pane_rects(AREA).len(), 2);
    }

    #[test]
    fn zoom_ends_when_the_zoomed_pane_closes() {
        let (mut mux, panes) = mux_with(1);
        let b = mux.new_pane_id();
        mux.split_active(b, Direction::Down);
        mux.toggle_zoom();
        mux.close_pane(b);
        assert_eq!(mux.pane_rects(AREA), [(panes[0], AREA)]);
        assert_eq!(mux.tabs()[0].zoomed, None);
    }
}
