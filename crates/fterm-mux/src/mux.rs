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
    /// A color of the tab (RGB) from an agent, Lua, a profile, or an escape sequence.
    pub color: Option<[u8; 3]>,
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

/// The name of the first workspace.
pub const MAIN: &str = "main";

/// A workspace: a set of tabs with a name. The tab bar shows the tabs of one workspace.
#[derive(Debug)]
struct Workspace {
    name: String,
    tabs: Vec<Tab>,
    active: usize,
}

impl Workspace {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            tabs: Vec::new(),
            active: 0,
        }
    }
}

/// The tabs of the window, in workspaces. All index methods (`tabs`, `select`, `cycle`, ...) work on the tabs of
/// the current workspace; methods with a pane or a tab id find it in any workspace.
pub struct Mux {
    workspaces: Vec<Workspace>,
    current: usize,
    next_id: u64,
}

impl Default for Mux {
    fn default() -> Self {
        Self {
            workspaces: vec![Workspace::new(MAIN)],
            current: 0,
            next_id: 0,
        }
    }
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

    fn ws(&self) -> Option<&Workspace> {
        self.workspaces.get(self.current)
    }

    /// The current workspace (there is always one while tabs are added).
    fn ws_mut(&mut self) -> &mut Workspace {
        if self.workspaces.is_empty() {
            self.workspaces.push(Workspace::new(MAIN));
            self.current = 0;
        }
        &mut self.workspaces[self.current]
    }

    /// The index of the workspace `name`, made when it is not there.
    fn ws_index(&mut self, name: &str) -> usize {
        match self.workspaces.iter().position(|w| w.name == name) {
            Some(i) => i,
            None => {
                self.workspaces.push(Workspace::new(name));
                self.workspaces.len() - 1
            }
        }
    }

    fn make_tab(&mut self, layout: Layout, active_pane: PaneId, title: Option<String>) -> Tab {
        Tab {
            id: TabId(self.next()),
            layout,
            active_pane,
            custom_title: title,
            zoomed: None,
            color: None,
        }
    }

    /// Adds a tab with this layout at the end of the current workspace (to restore a session). It does not
    /// become active.
    pub fn add_tab(&mut self, layout: Layout, active_pane: PaneId, title: Option<String>) -> TabId {
        let tab = self.make_tab(layout, active_pane, title);
        let id = tab.id;
        self.ws_mut().tabs.push(tab);
        id
    }

    /// Adds a tab at the end of the workspace `name` (made when it is not there). It does not become active.
    pub fn add_tab_in(
        &mut self,
        name: &str,
        layout: Layout,
        active_pane: PaneId,
        title: Option<String>,
    ) -> TabId {
        let tab = self.make_tab(layout, active_pane, title);
        let id = tab.id;
        let i = self.ws_index(name);
        self.workspaces[i].tabs.push(tab);
        id
    }

    /// Opens a tab with one pane, after the active tab, and makes it active.
    pub fn new_tab(&mut self, pane: PaneId) -> TabId {
        let tab = self.make_tab(Layout::Pane(pane), pane, None);
        let id = tab.id;
        let ws = self.ws_mut();
        let at = if ws.tabs.is_empty() { 0 } else { ws.active + 1 };
        ws.tabs.insert(at, tab);
        ws.active = at;
        id
    }

    /// Opens a tab with one pane in the workspace `name` (made when it is not there) and goes there.
    pub fn new_tab_in(&mut self, name: &str, pane: PaneId) -> TabId {
        self.current = self.ws_index(name);
        self.new_tab(pane)
    }

    /// The tabs of the current workspace.
    pub fn tabs(&self) -> &[Tab] {
        self.ws().map_or(&[], |w| &w.tabs)
    }

    /// The tabs of all workspaces.
    pub fn all_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.workspaces.iter().flat_map(|w| w.tabs.iter())
    }

    /// The index of the active tab in the current workspace.
    pub fn active_index(&self) -> usize {
        self.ws().map_or(0, |w| w.active)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.ws().and_then(|w| w.tabs.get(w.active))
    }

    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let ws = self.workspaces.get_mut(self.current)?;
        ws.tabs.get_mut(ws.active)
    }

    pub fn active_pane(&self) -> Option<PaneId> {
        self.active_tab().map(|tab| tab.active_pane)
    }

    /// The name of the current workspace.
    pub fn workspace(&self) -> &str {
        self.ws().map_or(MAIN, |w| &w.name)
    }

    /// The workspaces and how many tabs each has.
    pub fn workspaces(&self) -> Vec<(String, usize)> {
        self.workspaces
            .iter()
            .map(|w| (w.name.clone(), w.tabs.len()))
            .collect()
    }

    /// Goes to the workspace `name`. False when there is none.
    pub fn switch_workspace(&mut self, name: &str) -> bool {
        match self.workspaces.iter().position(|w| w.name == name) {
            Some(i) => {
                self.current = i;
                true
            }
            None => false,
        }
    }

    /// The next workspace (`+1`) or the previous one (`-1`). It wraps around.
    pub fn cycle_workspace(&mut self, step: i32) {
        let len = self.workspaces.len() as i32;
        if len > 0 {
            self.current = (self.current as i32 + step).rem_euclid(len) as usize;
        }
    }

    /// A new name for a workspace. False when there is no such workspace, or the name is empty or taken.
    pub fn rename_workspace(&mut self, old: &str, new: &str) -> bool {
        let new = new.trim();
        if new.is_empty() || self.workspaces.iter().any(|w| w.name == new) {
            return false;
        }
        match self.workspaces.iter_mut().find(|w| w.name == old) {
            Some(w) => {
                w.name = new.to_owned();
                true
            }
            None => false,
        }
    }

    /// Where a tab is: (workspace index, tab index).
    fn find_tab(&self, tab: TabId) -> Option<(usize, usize)> {
        self.workspaces
            .iter()
            .enumerate()
            .find_map(|(w, ws)| ws.tabs.iter().position(|t| t.id == tab).map(|i| (w, i)))
    }

    fn find_pane(&self, pane: PaneId) -> Option<(usize, usize)> {
        self.workspaces.iter().enumerate().find_map(|(w, ws)| {
            ws.tabs
                .iter()
                .position(|t| t.layout.contains(pane))
                .map(|i| (w, i))
        })
    }

    /// The tab that has this pane (in any workspace).
    pub fn pane_tab(&self, pane: PaneId) -> Option<TabId> {
        self.find_pane(pane)
            .map(|(w, i)| self.workspaces[w].tabs[i].id)
    }

    /// The workspace of a pane and the index of its tab there.
    pub fn pane_place(&self, pane: PaneId) -> Option<(&str, usize)> {
        self.find_pane(pane)
            .map(|(w, i)| (self.workspaces[w].name.as_str(), i))
    }

    /// The workspace of a tab.
    pub fn tab_workspace(&self, tab: TabId) -> Option<&str> {
        self.find_tab(tab)
            .map(|(w, _)| self.workspaces[w].name.as_str())
    }

    /// Shows a pane: its workspace, its tab, and the focus. False when there is no such pane.
    pub fn reveal(&mut self, pane: PaneId) -> bool {
        let Some((w, i)) = self.find_pane(pane) else {
            return false;
        };
        self.current = w;
        self.workspaces[w].active = i;
        self.focus(pane);
        true
    }

    /// Moves a tab to the workspace `name` (made when it is not there), and goes there with it.
    pub fn move_tab_to(&mut self, tab: TabId, name: &str) {
        let Some((w, i)) = self.find_tab(tab) else {
            return;
        };
        if self.workspaces[w].name == name {
            return;
        }
        let moved = self.take_tab(w, i);
        let to = self.ws_index(name);
        let ws = &mut self.workspaces[to];
        ws.tabs.push(moved);
        ws.active = ws.tabs.len() - 1;
        self.current = to;
    }

    /// Takes a tab out. A workspace with no tabs left goes; when it was the current one, the next one (or the
    /// one before) is current.
    fn take_tab(&mut self, w: usize, index: usize) -> Tab {
        let ws = &mut self.workspaces[w];
        let tab = ws.tabs.remove(index);
        // The right neighbor gets the place; at the end it is the left neighbor.
        if index < ws.active || ws.active >= ws.tabs.len() {
            ws.active = ws.active.saturating_sub(1);
        }
        if ws.tabs.is_empty() && self.workspaces.len() > 1 {
            self.workspaces.remove(w);
            if w < self.current || self.current >= self.workspaces.len() {
                self.current = self.current.saturating_sub(1);
            }
        }
        tab
    }

    /// No tabs in any workspace.
    fn no_tabs(&self) -> bool {
        self.workspaces.iter().all(|w| w.tabs.is_empty())
    }

    /// Closes a pane (for example, its shell ended).
    pub fn close_pane(&mut self, pane: PaneId) -> Closed {
        let Some((w, index)) = self.find_pane(pane) else {
            return Closed::Nothing;
        };
        let tab = &mut self.workspaces[w].tabs[index];
        if let Some(neighbor) = tab.layout.remove(pane) {
            if tab.active_pane == pane {
                tab.active_pane = neighbor;
            }
            if tab.zoomed == Some(pane) {
                tab.zoomed = None;
            }
            return Closed::Pane;
        }
        self.take_tab(w, index);
        if self.no_tabs() {
            Closed::LastTab
        } else {
            Closed::Tab
        }
    }

    /// Closes a tab and returns its panes (to stop their sessions).
    pub fn close_tab(&mut self, tab: TabId) -> Vec<PaneId> {
        match self.find_tab(tab) {
            Some((w, index)) => self.take_tab(w, index).layout.panes(),
            None => Vec::new(),
        }
    }

    pub fn select(&mut self, index: usize) {
        if let Some(ws) = self.workspaces.get_mut(self.current)
            && index < ws.tabs.len()
        {
            ws.active = index;
        }
    }

    pub fn select_last(&mut self) {
        if let Some(ws) = self.workspaces.get_mut(self.current) {
            ws.active = ws.tabs.len().saturating_sub(1);
        }
    }

    /// The next tab (`+1`) or the previous one (`-1`). It wraps around.
    pub fn cycle(&mut self, step: i32) {
        if let Some(ws) = self.workspaces.get_mut(self.current) {
            let len = ws.tabs.len() as i32;
            if len > 0 {
                ws.active = (ws.active as i32 + step).rem_euclid(len) as usize;
            }
        }
    }

    /// Moves the active tab left (`-1`) or right (`+1`). It stops at the edges.
    pub fn move_active(&mut self, step: i32) {
        if let Some(ws) = self.workspaces.get_mut(self.current) {
            let target = ws.active as i32 + step;
            if target >= 0 && (target as usize) < ws.tabs.len() {
                ws.tabs.swap(ws.active, target as usize);
                ws.active = target as usize;
            }
        }
    }

    /// Splits the active pane. `new` gets the second half and the focus.
    pub fn split_active(&mut self, new: PaneId, direction: Direction) {
        if let Some(tab) = self.active_tab_mut()
            && tab.layout.split(tab.active_pane, new, direction)
        {
            tab.active_pane = new;
            tab.zoomed = None;
        }
    }

    /// Gives the focus to `pane` (in the active tab).
    pub fn focus(&mut self, pane: PaneId) {
        if let Some(tab) = self.active_tab_mut()
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
        let Some(tab) = self.active_tab() else {
            return;
        };
        if let Some(neighbor) = tab.layout.neighbor(tab.active_pane, edge, area) {
            self.focus(neighbor);
        }
    }

    /// The active pane takes the whole tab, or the tab goes back to all panes.
    pub fn toggle_zoom(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
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
        self.active_tab_mut().map(|tab| &mut tab.layout)
    }

    fn tab_mut(&mut self, tab: TabId) -> Option<&mut Tab> {
        let (w, i) = self.find_tab(tab)?;
        Some(&mut self.workspaces[w].tabs[i])
    }

    /// Sets the color of a tab. `None` = no color.
    pub fn set_color(&mut self, tab: TabId, color: Option<[u8; 3]>) {
        if let Some(tab) = self.tab_mut(tab) {
            tab.color = color;
        }
    }

    /// Sets the user's name for a tab. An empty name goes back to the auto title.
    pub fn rename(&mut self, tab: TabId, name: &str) {
        if let Some(tab) = self.tab_mut(tab) {
            let name = name.trim();
            tab.custom_title = (!name.is_empty()).then(|| name.to_owned());
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
    fn a_saved_tab_comes_back_at_the_end() {
        let (mut mux, _) = mux_with(1);
        let (a, b) = (mux.new_pane_id(), mux.new_pane_id());
        let layout = Layout::Split {
            direction: Direction::Right,
            ratio: 0.3,
            first: Box::new(Layout::Pane(a)),
            second: Box::new(Layout::Pane(b)),
        };
        let tab = mux.add_tab(layout.clone(), b, Some("build".into()));
        assert_eq!(mux.tabs().len(), 2);
        let added = mux.tabs().last().unwrap();
        assert_eq!(added.id, tab);
        assert_eq!(added.layout, layout);
        assert_eq!(added.active_pane, b);
        assert_eq!(added.custom_title.as_deref(), Some("build"));
        assert_eq!(mux.active_index(), 0, "it does not take the focus");
        // A new id after it is still new.
        let next = mux.new_pane_id();
        assert!(next != a && next != b);
    }

    /// `main` with 2 tabs and `work` with 2 tabs; `work` is the current one.
    fn two_workspaces() -> (Mux, Vec<PaneId>) {
        let (mut mux, mut panes) = mux_with(2);
        for _ in 0..2 {
            let pane = mux.new_pane_id();
            mux.new_tab_in("work", pane);
            panes.push(pane);
        }
        (mux, panes)
    }

    #[test]
    fn the_first_workspace_is_main() {
        let (mux, _) = mux_with(2);
        assert_eq!(mux.workspace(), "main");
        assert_eq!(mux.workspaces(), [("main".to_owned(), 2)]);
    }

    #[test]
    fn the_tab_bar_shows_only_the_tabs_of_the_workspace() {
        let (mut mux, p) = two_workspaces();
        assert_eq!(mux.workspace(), "work");
        assert_eq!(order(&mux), [p[2], p[3]]);
        assert_eq!(mux.active_pane(), Some(p[3]));
        assert_eq!(mux.all_tabs().count(), 4);
        assert_eq!(
            mux.workspaces(),
            [("main".to_owned(), 2), ("work".to_owned(), 2)]
        );
        // Each workspace keeps its active tab.
        assert!(mux.switch_workspace("main"));
        assert_eq!(order(&mux), [p[0], p[1]]);
        assert_eq!(mux.active_pane(), Some(p[0]));
        assert!(!mux.switch_workspace("nothing"));
        mux.cycle_workspace(1);
        assert_eq!(mux.workspace(), "work");
        mux.cycle_workspace(1);
        assert_eq!(mux.workspace(), "main", "it wraps around");
    }

    #[test]
    fn tab_keys_stay_inside_the_workspace() {
        let (mut mux, p) = two_workspaces();
        mux.cycle(1);
        assert_eq!(mux.active_pane(), Some(p[2]), "it wraps inside `work`");
        mux.select(1);
        assert_eq!(mux.active_pane(), Some(p[3]));
        mux.select(5);
        assert_eq!(mux.active_pane(), Some(p[3]), "no such tab here");
    }

    #[test]
    fn closing_the_last_tab_of_a_workspace_goes_to_another() {
        let (mut mux, p) = two_workspaces();
        assert_eq!(mux.close_pane(p[3]), Closed::Tab);
        assert_eq!(
            mux.close_pane(p[2]),
            Closed::Tab,
            "not the last tab of the window"
        );
        assert_eq!(mux.workspace(), "main");
        assert_eq!(mux.workspaces(), [("main".to_owned(), 2)], "`work` is gone");
        assert_eq!(mux.close_pane(p[0]), Closed::Tab);
        assert_eq!(mux.close_pane(p[1]), Closed::LastTab);
    }

    #[test]
    fn panes_and_tabs_of_hidden_workspaces_are_found() {
        let (mut mux, p) = two_workspaces();
        let tab = mux.pane_tab(p[0]).unwrap();
        mux.rename(tab, "far");
        mux.set_color(tab, Some([1, 2, 3]));
        assert_eq!(mux.tab_workspace(tab), Some("main"));
        assert!(mux.reveal(p[1]));
        assert_eq!(mux.workspace(), "main");
        assert_eq!(mux.active_pane(), Some(p[1]));
        let far = mux.tabs().iter().find(|t| t.id == tab).unwrap();
        assert_eq!(far.custom_title.as_deref(), Some("far"));
        assert_eq!(far.color, Some([1, 2, 3]));
        // Closing a tab of a hidden workspace keeps the current one.
        let work = mux.pane_tab(p[2]).unwrap();
        assert_eq!(mux.close_tab(work), [p[2]]);
        assert_eq!(mux.workspace(), "main");
        assert_eq!(mux.workspaces()[1], ("work".to_owned(), 1));
    }

    #[test]
    fn the_place_of_a_pane() {
        let (mux, p) = two_workspaces();
        assert_eq!(mux.pane_place(p[1]), Some(("main", 1)));
        assert_eq!(mux.pane_place(p[2]), Some(("work", 0)));
        assert_eq!(mux.pane_place(PaneId(999)), None);
    }

    #[test]
    fn a_tab_moves_to_another_workspace() {
        let (mut mux, p) = two_workspaces();
        let tab = mux.pane_tab(p[3]).unwrap();
        mux.move_tab_to(tab, "agents");
        assert_eq!(mux.workspace(), "agents", "the tab is followed");
        assert_eq!(order(&mux), [p[3]]);
        assert_eq!(mux.tab_workspace(tab), Some("agents"));
        assert!(mux.switch_workspace("work"));
        assert_eq!(order(&mux), [p[2]]);
        // Moving the last tab: its workspace goes.
        let last = mux.pane_tab(p[2]).unwrap();
        mux.move_tab_to(last, "main");
        assert_eq!(
            mux.workspaces(),
            [("main".to_owned(), 3), ("agents".to_owned(), 1)]
        );
    }

    #[test]
    fn a_workspace_gets_a_new_name() {
        let (mut mux, _) = two_workspaces();
        assert!(mux.rename_workspace("work", "fterm"));
        assert_eq!(mux.workspace(), "fterm");
        assert!(!mux.rename_workspace("fterm", "main"), "the name is taken");
        assert!(!mux.rename_workspace("fterm", "  "), "no empty names");
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
    fn a_tab_gets_a_color_and_loses_it() {
        let (mut mux, panes) = mux_with(2);
        let tab = mux.pane_tab(panes[1]).unwrap();
        assert_eq!(mux.tabs()[1].color, None);
        mux.set_color(tab, Some([0xf3, 0x8b, 0xa8]));
        assert_eq!(mux.tabs()[1].color, Some([0xf3, 0x8b, 0xa8]));
        assert_eq!(mux.tabs()[0].color, None, "only that tab");
        mux.set_color(tab, None);
        assert_eq!(mux.tabs()[1].color, None);
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
