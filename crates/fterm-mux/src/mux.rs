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
    /// A program over a pane of this tab for a moment (not in the layout).
    pub overlay: Option<Overlay>,
    /// Panes that belong to a pane of this tab (not in the layout): a strip in the rect of their pane.
    pub companions: Vec<Companions>,
}

/// The companions of one pane (a log, a watcher, a second shell). One of them is shown at a time, as a strip in the
/// rect of the pane; hidden, their programs live on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Companions {
    pub owner: PaneId,
    pub panes: Vec<PaneId>,
    /// The one that is shown now (an index in `panes`).
    pub shown: Option<usize>,
    /// The one that was shown last (it comes back on a toggle).
    last: usize,
}

impl Companions {
    fn shown_pane(&self) -> Option<PaneId> {
        self.shown.and_then(|i| self.panes.get(i).copied())
    }
}

/// A pane over another pane for a moment (a file manager, lazygit, htop). It is not in the layout; it gets the
/// keys while it is there, and goes when its program ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overlay {
    pub pane: PaneId,
    /// The pane under it (the active pane when it was opened).
    pub over: PaneId,
    /// Its size in percent of the pane under it (100 = all of it).
    pub size: u8,
    /// It is shown; hidden, its program lives on and the pane under it has the keys.
    pub shown: bool,
}

/// The overlay of the window: a program over the tab in front (any tab), shown or hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WindowOverlay {
    pane: PaneId,
    shown: bool,
    /// Its size in percent of the tab area.
    size: u8,
}

/// The drop-down shell of the window: shown over the tab in front, or hidden (its shell lives on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scratch {
    pane: PaneId,
    shown: bool,
    /// Its height in percent of the tab.
    size: u8,
}

/// The smallest size of a scratch or an overlay that Alt+Shift+arrows makes, in percent.
const FLOATING_MIN_SIZE: u8 = 10;

/// The place of an overlay: `size` percent of the pane under it, in its middle, at least `min` (width, height)
/// and at most the pane.
pub fn overlay_rect(pane: Rect, size: u8, min: (f32, f32)) -> Rect {
    let part = f32::from(size.clamp(1, 100)) / 100.0;
    let width = (pane.width * part).max(min.0).min(pane.width);
    let height = (pane.height * part).max(min.1).min(pane.height);
    Rect::new(
        pane.x + (pane.width - width) / 2.0,
        pane.y + (pane.height - height) / 2.0,
        width,
        height,
    )
}

/// The place of the scratch: the whole width of `area`, `size` percent of its height (at least `min_height`), at its
/// top.
pub fn scratch_rect(area: Rect, size: u8, min_height: f32) -> Rect {
    let part = f32::from(size.clamp(1, 100)) / 100.0;
    let height = (area.height * part).max(min_height).min(area.height);
    Rect::new(area.x, area.y, area.width, height)
}

/// The places of a pane and its shown companion: the strip is `size` percent (10–90) of `pane`, below it
/// (`Direction::Down`) or at its right (`Direction::Right`).
pub fn companion_rects(pane: Rect, side: Direction, size: u8) -> (Rect, Rect) {
    let part = f32::from(size.clamp(10, 90)) / 100.0;
    match side {
        Direction::Down => {
            let strip = (pane.height * part).round();
            let owner = pane.height - strip;
            (
                Rect::new(pane.x, pane.y, pane.width, owner),
                Rect::new(pane.x, pane.y + owner, pane.width, strip),
            )
        }
        Direction::Right => {
            let strip = (pane.width * part).round();
            let owner = pane.width - strip;
            (
                Rect::new(pane.x, pane.y, owner, pane.height),
                Rect::new(pane.x + owner, pane.y, strip, pane.height),
            )
        }
    }
}

impl Tab {
    /// The panes of this tab that are seen (all of them, or the zoomed one) with the shown companions, and their
    /// places in `area`. Companions take `size` percent of their pane at the `side`.
    pub fn rects(&self, area: Rect, side: Direction, size: u8) -> Vec<(PaneId, Rect)> {
        let rects = match self.zoomed {
            Some(pane) => vec![(pane, area)],
            None => self.layout.rects(area),
        };
        let mut out = Vec::with_capacity(rects.len());
        for (pane, rect) in rects {
            match self
                .companions
                .iter()
                .find(|c| c.owner == pane)
                .and_then(Companions::shown_pane)
            {
                Some(companion) => {
                    let (top, strip) = companion_rects(rect, side, size);
                    out.push((pane, top));
                    out.push((companion, strip));
                }
                None => out.push((pane, rect)),
            }
        }
        out
    }

    /// The owner of a companion in this tab.
    fn owner_of(&self, pane: PaneId) -> Option<PaneId> {
        self.companions
            .iter()
            .find(|c| c.panes.contains(&pane))
            .map(|c| c.owner)
    }

    /// The pane in the layout for `pane`: itself, or its owner when it is a companion.
    fn layout_pane(&self, pane: PaneId) -> PaneId {
        self.owner_of(pane).unwrap_or(pane)
    }
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
    /// An overlay or the scratch (no layout changed).
    Floating,
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
    scratch: Option<Scratch>,
    window_overlay: Option<WindowOverlay>,
    /// The dashboard of the window (a scene over the whole tab area) and whether it is shown.
    dashboard: Option<(PaneId, bool)>,
    /// Where companions are in their pane, and their size in percent.
    companion_place: (Direction, u8),
}

impl Default for Mux {
    fn default() -> Self {
        Self {
            workspaces: vec![Workspace::new(MAIN)],
            current: 0,
            next_id: 0,
            scratch: None,
            window_overlay: None,
            dashboard: None,
            companion_place: (Direction::Down, 40),
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
            overlay: None,
            companions: Vec::new(),
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

    /// The pane that gets the keys: the scratch when it is shown, else the window overlay when shown, else the
    /// overlay of the active tab when shown, else the active pane.
    pub fn input_pane(&self) -> Option<PaneId> {
        if let Some(s) = self.scratch.filter(|s| s.shown) {
            return Some(s.pane);
        }
        if let Some((pane, true)) = self.dashboard {
            return Some(pane);
        }
        if let Some(w) = self.window_overlay.filter(|w| w.shown) {
            return Some(w.pane);
        }
        self.overlay()
            .filter(|o| o.shown)
            .map(|o| o.pane)
            .or_else(|| self.active_pane())
    }

    /// Shows or hides the overlay of the active tab: `Some(shown)` after the change, `None` when it has none.
    pub fn toggle_overlay(&mut self) -> Option<bool> {
        let o = self.active_tab_mut()?.overlay.as_mut()?;
        o.shown = !o.shown;
        Some(o.shown)
    }

    /// Makes `pane` the overlay of the window, shown, `size` percent of the tab area.
    pub fn set_window_overlay(&mut self, pane: PaneId, size: u8) {
        self.window_overlay = Some(WindowOverlay {
            pane,
            shown: true,
            size: size.clamp(1, 100),
        });
    }

    /// Shows or hides the overlay of the window: `Some(shown)` after the change, `None` when there is none.
    pub fn toggle_window_overlay(&mut self) -> Option<bool> {
        let w = self.window_overlay.as_mut()?;
        w.shown = !w.shown;
        Some(w.shown)
    }

    /// Makes `pane` the dashboard of the window, shown.
    pub fn set_dashboard(&mut self, pane: PaneId) {
        self.dashboard = Some((pane, true));
    }

    /// Shows or hides the dashboard: `Some(shown)` after the change, `None` when there is none.
    pub fn toggle_dashboard(&mut self) -> Option<bool> {
        let (_, shown) = self.dashboard.as_mut()?;
        *shown = !*shown;
        Some(*shown)
    }

    /// The dashboard and whether it is shown.
    pub fn dashboard(&self) -> Option<(PaneId, bool)> {
        self.dashboard
    }

    /// The overlay of the window and whether it is shown.
    pub fn window_overlay(&self) -> Option<(PaneId, bool)> {
        self.window_overlay.map(|w| (w.pane, w.shown))
    }

    /// Opens `pane` as the overlay over the active pane (over its pane when it is a companion). `false` when the tab
    /// has one already (or there is no tab).
    pub fn open_overlay(&mut self, pane: PaneId, size: u8) -> bool {
        let Some(tab) = self.active_tab_mut() else {
            return false;
        };
        if tab.overlay.is_some() {
            return false;
        }
        // Over a pane of the layout: from a companion, over its pane (a companion ends with its own program).
        let over = tab.layout_pane(tab.active_pane);
        tab.overlay = Some(Overlay {
            pane,
            over,
            size: size.clamp(1, 100),
            shown: true,
        });
        true
    }

    /// The overlay of the active tab.
    pub fn overlay(&self) -> Option<&Overlay> {
        self.active_tab().and_then(|t| t.overlay.as_ref())
    }

    /// The overlay over `pane` (in any tab).
    pub fn overlay_of(&self, pane: PaneId) -> Option<PaneId> {
        self.all_tabs()
            .filter_map(|t| t.overlay)
            .find(|o| o.over == pane)
            .map(|o| o.pane)
    }

    /// `pane` is an overlay or the scratch (not in a layout).
    pub fn is_floating(&self, pane: PaneId) -> bool {
        self.scratch_pane() == Some(pane)
            || self.window_overlay.is_some_and(|w| w.pane == pane)
            || self.dashboard.is_some_and(|(p, _)| p == pane)
            || self
                .all_tabs()
                .any(|t| t.overlay.is_some_and(|o| o.pane == pane))
    }

    /// The scratch of the window, shown or not.
    pub fn scratch_pane(&self) -> Option<PaneId> {
        self.scratch.map(|s| s.pane)
    }

    /// Makes `pane` the scratch, shown, `size` percent of the tab high.
    pub fn set_scratch(&mut self, pane: PaneId, size: u8) {
        self.scratch = Some(Scratch {
            pane,
            shown: true,
            size: size.clamp(1, 100),
        });
    }

    /// Shows or hides the scratch: `Some(shown)` after the change, `None` when there is no scratch yet.
    pub fn toggle_scratch(&mut self) -> Option<bool> {
        let s = self.scratch.as_mut()?;
        s.shown = !s.shown;
        Some(s.shown)
    }

    /// Makes the shown scratch or overlay `pane` bigger or smaller by `step` percent, as if its `edge` moved
    /// (Alt+Shift+arrows). The scratch is at the top, so only its bottom edge moves: Down makes it higher, Up lower.
    /// An overlay is in the middle: Right and Down make it bigger, Left and Up smaller. `false` when `pane` is
    /// none of them, or the edge does not move it.
    pub fn resize_floating(&mut self, pane: PaneId, edge: Edge, step: u8) -> bool {
        let grow = matches!(edge, Edge::Right | Edge::Down);
        let scratch = self.scratch.as_mut().filter(|s| s.shown && s.pane == pane);
        let size = if let Some(s) = scratch {
            if matches!(edge, Edge::Left | Edge::Right) {
                return false;
            }
            &mut s.size
        } else if let Some(w) = self
            .window_overlay
            .as_mut()
            .filter(|w| w.shown && w.pane == pane)
        {
            &mut w.size
        } else if let Some(o) = self
            .active_tab_mut()
            .and_then(|t| t.overlay.as_mut())
            .filter(|o| o.shown && o.pane == pane)
        {
            &mut o.size
        } else {
            return false;
        };
        let new = if grow {
            size.saturating_add(step).min(100)
        } else {
            size.saturating_sub(step).max(FLOATING_MIN_SIZE)
        };
        let changed = new != *size;
        *size = new;
        changed
    }

    /// The shown floating panes with their places in `area`, from the bottom to the top: the overlay of the active
    /// tab (over its pane, when that pane is seen), the overlay of the window, the scratch. `min` is the smallest
    /// size of a floating pane.
    pub fn floating_rects(&self, area: Rect, min: (f32, f32)) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        if let Some(o) = self.overlay().filter(|o| o.shown)
            && let Some((_, rect)) = self
                .pane_rects(area)
                .into_iter()
                .find(|(p, _)| *p == o.over)
        {
            out.push((o.pane, overlay_rect(rect, o.size, min)));
        }
        if let Some(w) = self.window_overlay.filter(|w| w.shown) {
            out.push((w.pane, overlay_rect(area, w.size, min)));
        }
        if let Some((pane, true)) = self.dashboard {
            out.push((pane, area));
        }
        if let Some(s) = self.scratch.filter(|s| s.shown) {
            out.push((s.pane, scratch_rect(area, s.size, min.1)));
        }
        out
    }

    /// Where companions are shown in their pane (`Direction::Down` = at the bottom), and their size in percent.
    pub fn set_companion_place(&mut self, side: Direction, size: u8) {
        self.companion_place = (side, size.clamp(10, 90));
    }

    /// The panes of `tab` that are seen, with the shown companions, and their places in `area`.
    pub fn tab_rects(&self, tab: &Tab, area: Rect) -> Vec<(PaneId, Rect)> {
        let (side, size) = self.companion_place;
        tab.rects(area, side, size)
    }

    /// Adds `pane` as a companion of `owner` (in the active tab), shows it, and gives it the keys. `false` when
    /// `owner` is not a pane in the layout of the active tab.
    pub fn add_companion(&mut self, owner: PaneId, pane: PaneId) -> bool {
        let Some(tab) = self.active_tab_mut() else {
            return false;
        };
        if !tab.layout.contains(owner) {
            return false;
        }
        let list = match tab.companions.iter().position(|c| c.owner == owner) {
            Some(i) => &mut tab.companions[i],
            None => {
                tab.companions.push(Companions {
                    owner,
                    panes: Vec::new(),
                    shown: None,
                    last: 0,
                });
                tab.companions.last_mut().expect("just added")
            }
        };
        list.panes.push(pane);
        let index = list.panes.len() - 1;
        list.shown = Some(index);
        list.last = index;
        tab.active_pane = pane;
        if tab.zoomed.is_some_and(|z| z != owner) {
            tab.zoomed = None;
        }
        true
    }

    /// The companions of the active pane (or of its owner, when it is a companion).
    fn active_companions(&mut self) -> Option<&mut Tab> {
        let tab = self.active_tab_mut()?;
        let owner = tab.layout_pane(tab.active_pane);
        tab.companions
            .iter()
            .any(|c| c.owner == owner && !c.panes.is_empty())
            .then_some(tab)
    }

    /// Shows or hides the companion of the active pane: `Some(shown)` after the change, `None` when it has none.
    /// Shown, the companion has the keys; hidden, its pane has them.
    pub fn toggle_companion(&mut self) -> Option<bool> {
        let tab = self.active_companions()?;
        let owner = tab.layout_pane(tab.active_pane);
        let list = tab.companions.iter_mut().find(|c| c.owner == owner)?;
        if list.shown.is_some() {
            list.shown = None;
            tab.active_pane = owner;
            Some(false)
        } else {
            let index = list.last.min(list.panes.len() - 1);
            list.shown = Some(index);
            tab.active_pane = list.panes[index];
            Some(true)
        }
    }

    /// Shows the next companion of the active pane (the first one after the shown or the last shown one) and gives
    /// it the keys. `None` when the pane has no companions.
    pub fn next_companion(&mut self) -> Option<PaneId> {
        let tab = self.active_companions()?;
        let owner = tab.layout_pane(tab.active_pane);
        let list = tab.companions.iter_mut().find(|c| c.owner == owner)?;
        let index = (list.shown.unwrap_or(list.last) + 1) % list.panes.len();
        list.shown = Some(index);
        list.last = index;
        tab.active_pane = list.panes[index];
        Some(list.panes[index])
    }

    /// The companions of `owner` (in any tab), shown or not.
    pub fn companions_of(&self, owner: PaneId) -> Vec<PaneId> {
        self.all_tabs()
            .flat_map(|t| t.companions.iter())
            .find(|c| c.owner == owner)
            .map(|c| c.panes.clone())
            .unwrap_or_default()
    }

    /// The pane that a companion belongs to.
    pub fn owner_of(&self, pane: PaneId) -> Option<PaneId> {
        self.all_tabs().find_map(|t| t.owner_of(pane))
    }

    /// The lines between the panes of the active tab and their shown companions (1 pixel, at the edge of the strip).
    pub fn companion_dividers(&self, area: Rect) -> Vec<Rect> {
        let Some(tab) = self.active_tab() else {
            return Vec::new();
        };
        let side = self.companion_place.0;
        self.tab_rects(tab, area)
            .into_iter()
            .filter(|(p, _)| tab.owner_of(*p).is_some())
            .map(|(_, r)| match side {
                Direction::Down => Rect::new(r.x, r.y, r.width, 1.0),
                Direction::Right => Rect::new(r.x, r.y, 1.0, r.height),
            })
            .collect()
    }

    /// The active pane is a companion: its strip gets bigger or smaller by `step` percent, as if the line between
    /// it and its pane moved to `edge` (Alt+Shift+arrows). The size is the same for all companions. `true` when
    /// the keys are for the strip (also at its smallest or biggest size); `false` when the active pane is no
    /// companion, or `edge` does not cross its strip.
    pub fn resize_companion(&mut self, edge: Edge, step: u8) -> bool {
        let Some(tab) = self.active_tab() else {
            return false;
        };
        if tab.owner_of(tab.active_pane).is_none() {
            return false;
        }
        let (side, size) = self.companion_place;
        let grow = match (side, edge) {
            (Direction::Down, Edge::Up) | (Direction::Right, Edge::Left) => true,
            (Direction::Down, Edge::Down) | (Direction::Right, Edge::Right) => false,
            _ => return false,
        };
        let new = if grow {
            size.saturating_add(step).min(90)
        } else {
            size.saturating_sub(step).max(10)
        };
        self.companion_place.1 = new;
        true
    }

    /// The companion is shown now.
    pub fn companion_shown(&self, pane: PaneId) -> bool {
        self.all_tabs()
            .flat_map(|t| t.companions.iter())
            .any(|c| c.shown_pane() == Some(pane))
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

    /// The tab of a pane: in its layout, or its overlay.
    fn find_pane(&self, pane: PaneId) -> Option<(usize, usize)> {
        self.workspaces.iter().enumerate().find_map(|(w, ws)| {
            ws.tabs
                .iter()
                .position(|t| {
                    t.layout.contains(pane)
                        || t.overlay.is_some_and(|o| o.pane == pane)
                        || t.owner_of(pane).is_some()
                })
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
        let tab = &mut self.workspaces[w].tabs[i];
        if let Some(list) = tab.companions.iter_mut().find(|c| c.panes.contains(&pane)) {
            let index = list.panes.iter().position(|p| *p == pane).expect("found");
            list.shown = Some(index);
            list.last = index;
        }
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

    /// Closes a pane (for example, its shell ended). The overlay over it goes too (`overlay_of` tells which one, so
    /// its program can be ended).
    pub fn close_pane(&mut self, pane: PaneId) -> Closed {
        if self.scratch_pane() == Some(pane) {
            self.scratch = None;
            return Closed::Floating;
        }
        if self.window_overlay.is_some_and(|w| w.pane == pane) {
            self.window_overlay = None;
            return Closed::Floating;
        }
        if self.dashboard.is_some_and(|(p, _)| p == pane) {
            self.dashboard = None;
            return Closed::Floating;
        }
        let Some((w, index)) = self.find_pane(pane) else {
            return Closed::Nothing;
        };
        let tab = &mut self.workspaces[w].tabs[index];
        if tab.overlay.is_some_and(|o| o.pane == pane) {
            tab.overlay = None;
            return Closed::Floating;
        }
        if tab.overlay.is_some_and(|o| o.over == pane) {
            tab.overlay = None;
        }
        if let Some(list) = tab.companions.iter_mut().find(|c| c.panes.contains(&pane)) {
            let index = list.panes.iter().position(|p| *p == pane).expect("found");
            list.panes.remove(index);
            // The shown one ended: none is shown. Another one: the shown index moves with the list.
            list.shown = match list.shown {
                Some(s) if s == index => None,
                Some(s) if s > index => Some(s - 1),
                s => s,
            };
            list.last = list.last.min(list.panes.len().saturating_sub(1));
            let owner = list.owner;
            tab.companions.retain(|c| !c.panes.is_empty());
            if tab.active_pane == pane {
                tab.active_pane = owner;
            }
            return Closed::Floating;
        }
        // The companions of the pane go with it (the app ends their programs, see `companions_of`).
        tab.companions.retain(|c| c.owner != pane);
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

    /// Closes a tab and returns its panes (to stop their sessions): the layout, its overlay, and the companions.
    pub fn close_tab(&mut self, tab: TabId) -> Vec<PaneId> {
        let Some((w, index)) = self.find_tab(tab) else {
            return Vec::new();
        };
        let tab = self.take_tab(w, index);
        let mut panes = tab.layout.panes();
        panes.extend(tab.overlay.map(|o| o.pane));
        panes.extend(tab.companions.iter().flat_map(|c| c.panes.iter().copied()));
        panes
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
            && tab
                .layout
                .split(tab.layout_pane(tab.active_pane), new, direction)
        {
            tab.active_pane = new;
            tab.zoomed = None;
        }
    }

    /// Splits `pane` (in any tab and workspace) and puts `new` in its second half. Unlike `split_active`, the
    /// focus does not move: no tab, workspace, or active pane changes. For panes that a program or an agent opens
    /// while the user works elsewhere. `false` = no such pane.
    pub fn split_pane(&mut self, pane: PaneId, new: PaneId, direction: Direction) -> bool {
        let Some((w, i)) = self.find_pane(pane) else {
            return false;
        };
        let tab = &mut self.workspaces[w].tabs[i];
        // An overlay or a companion has no place in the tree: the split goes next to its pane.
        let at = match tab.overlay {
            Some(o) if o.pane == pane => o.over,
            _ => tab.layout_pane(pane),
        };
        tab.layout.split(at, new, direction)
    }

    /// Gives the focus to `pane` (in the active tab).
    pub fn focus(&mut self, pane: PaneId) {
        if let Some(tab) = self.active_tab_mut()
            && (tab.layout.contains(pane)
                || tab.companions.iter().any(|c| c.shown_pane() == Some(pane)))
        {
            tab.active_pane = pane;
            let owner = tab.layout_pane(pane);
            if tab.zoomed.is_some_and(|z| z != owner) {
                tab.zoomed = None;
            }
        }
    }

    /// Gives the focus to the neighbor of the active pane toward `edge`.
    pub fn focus_direction(&mut self, edge: Edge, area: Rect) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        if let Some(neighbor) = tab
            .layout
            .neighbor(tab.layout_pane(tab.active_pane), edge, area)
        {
            self.focus(neighbor);
        }
    }

    /// The active pane takes the whole tab, or the tab goes back to all panes.
    pub fn toggle_zoom(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
            tab.zoomed = match tab.zoomed {
                Some(_) => None,
                None if tab.layout.panes().len() > 1 => Some(tab.layout_pane(tab.active_pane)),
                None => None,
            };
        }
    }

    /// The panes of the active tab that are seen now, and their places in `area`.
    pub fn pane_rects(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        match self.active_tab() {
            Some(tab) => self.tab_rects(tab, area),
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
    fn an_overlay_is_over_the_active_pane_and_gets_the_keys() {
        let (mut mux, panes) = mux_with(2);
        let over = panes[0];
        let overlay = mux.new_pane_id();
        assert!(mux.open_overlay(overlay, 80));
        assert_eq!(
            mux.overlay().map(|o| (o.pane, o.over, o.size)),
            Some((overlay, over, 80))
        );
        assert_eq!(mux.input_pane(), Some(overlay));
        assert_eq!(
            mux.active_pane(),
            Some(over),
            "the pane under it stays the active one"
        );
        // One overlay per tab.
        let second = mux.new_pane_id();
        assert!(!mux.open_overlay(second, 50));
        // It belongs to its tab: another tab has its own keys.
        mux.select(1);
        assert_eq!(mux.overlay(), None);
        assert_eq!(mux.input_pane(), Some(panes[1]));
        mux.select(0);
        assert_eq!(mux.pane_tab(overlay), mux.pane_tab(over));
        assert!(mux.is_floating(overlay) && !mux.is_floating(over));
    }

    #[test]
    fn an_overlay_hides_and_its_program_lives_on() {
        let area = Rect::new(0.0, 0.0, 1000.0, 600.0);
        let (mut mux, panes) = mux_with(1);
        assert_eq!(mux.toggle_overlay(), None, "no overlay yet");
        let overlay = mux.new_pane_id();
        mux.open_overlay(overlay, 80);
        assert_eq!(mux.toggle_overlay(), Some(false));
        // Hidden: the pane under it has the keys, and it is not drawn; it is still the overlay of the tab.
        assert_eq!(mux.input_pane(), Some(panes[0]));
        assert!(mux.floating_rects(area, (0.0, 0.0)).is_empty());
        assert_eq!(
            mux.overlay().map(|o| (o.pane, o.shown)),
            Some((overlay, false))
        );
        assert!(mux.is_floating(overlay));
        assert_eq!(mux.toggle_overlay(), Some(true));
        assert_eq!(mux.input_pane(), Some(overlay));
        assert_eq!(mux.floating_rects(area, (0.0, 0.0)).len(), 1);
    }

    #[test]
    fn the_window_overlay_is_over_every_tab() {
        let area = Rect::new(0.0, 0.0, 1000.0, 600.0);
        let (mut mux, panes) = mux_with(2);
        assert_eq!(mux.toggle_window_overlay(), None, "no window overlay yet");
        let far = mux.new_pane_id();
        mux.set_window_overlay(far, 90);
        assert_eq!(mux.input_pane(), Some(far));
        // 90% of the tab area, in its middle.
        assert_eq!(
            mux.floating_rects(area, (0.0, 0.0)),
            [(far, Rect::new(50.0, 30.0, 900.0, 540.0))]
        );
        mux.select(1);
        assert_eq!(mux.input_pane(), Some(far), "the same one in another tab");
        // A tab overlay is under it; the scratch is above it.
        let tab_overlay = mux.new_pane_id();
        mux.open_overlay(tab_overlay, 80);
        assert_eq!(mux.input_pane(), Some(far));
        let rects = mux.floating_rects(area, (0.0, 0.0));
        assert_eq!(
            rects.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
            [tab_overlay, far]
        );
        let scratch = mux.new_pane_id();
        mux.set_scratch(scratch, 50);
        assert_eq!(mux.input_pane(), Some(scratch));
        mux.toggle_scratch();
        // Hidden: the tab overlay has the keys again.
        assert_eq!(mux.toggle_window_overlay(), Some(false));
        assert_eq!(mux.input_pane(), Some(tab_overlay));
        assert!(mux.is_floating(far));
        assert_eq!(mux.window_overlay(), Some((far, false)));
        // Its program ends: it goes.
        assert_eq!(mux.close_pane(far), Closed::Floating);
        assert_eq!(mux.window_overlay(), None);
        let _ = panes;
    }

    #[test]
    fn closing_an_overlay_leaves_its_pane() {
        let (mut mux, panes) = mux_with(1);
        let overlay = mux.new_pane_id();
        mux.open_overlay(overlay, 80);
        assert_eq!(mux.close_pane(overlay), Closed::Floating);
        assert_eq!(mux.overlay(), None);
        assert_eq!(mux.input_pane(), Some(panes[0]));
        assert_eq!(mux.tabs().len(), 1, "the tab stays");
    }

    #[test]
    fn the_overlay_of_a_pane_that_closes_is_known() {
        let (mut mux, panes) = mux_with(1);
        let b = mux.new_pane_id();
        mux.split_active(b, Direction::Right);
        let overlay = mux.new_pane_id();
        mux.open_overlay(overlay, 80);
        assert_eq!(mux.overlay_of(b), Some(overlay), "over the active pane b");
        assert_eq!(mux.overlay_of(panes[0]), None);
        // The pane under it closes: its overlay goes too (the app ends its program).
        mux.close_pane(b);
        assert_eq!(mux.overlay(), None);
    }

    #[test]
    fn the_scratch_shows_and_hides() {
        let (mut mux, panes) = mux_with(2);
        assert_eq!(
            mux.toggle_scratch(),
            None,
            "no scratch yet: the app makes one"
        );
        let scratch = mux.new_pane_id();
        mux.set_scratch(scratch, 50);
        assert_eq!(mux.input_pane(), Some(scratch), "a new scratch is shown");
        assert_eq!(mux.toggle_scratch(), Some(false));
        assert_eq!(mux.input_pane(), Some(panes[0]));
        assert_eq!(mux.toggle_scratch(), Some(true));
        // Over every tab: it is the same shell in another tab.
        mux.select(1);
        assert_eq!(mux.input_pane(), Some(scratch));
        assert!(mux.is_floating(scratch));
        // Over an overlay too.
        let overlay = mux.new_pane_id();
        mux.open_overlay(overlay, 80);
        assert_eq!(mux.input_pane(), Some(scratch));
        // `exit` in it: it goes.
        assert_eq!(mux.close_pane(scratch), Closed::Floating);
        assert_eq!(mux.scratch_pane(), None);
        assert_eq!(mux.input_pane(), Some(overlay));
    }

    #[test]
    fn the_floating_panes_and_their_places() {
        let area = Rect::new(0.0, 0.0, 1000.0, 600.0);
        let (mut mux, panes) = mux_with(1);
        let b = mux.new_pane_id();
        mux.split_active(b, Direction::Right);
        assert!(mux.floating_rects(area, (100.0, 60.0)).is_empty());
        let overlay = mux.new_pane_id();
        mux.open_overlay(overlay, 80);
        // Over b (the right half: x 500..1000): 80% of it, in the middle.
        let rects = mux.floating_rects(area, (100.0, 60.0));
        assert_eq!(rects, [(overlay, Rect::new(550.0, 60.0, 400.0, 480.0))]);
        let scratch = mux.new_pane_id();
        mux.set_scratch(scratch, 50);
        let rects = mux.floating_rects(area, (100.0, 60.0));
        assert_eq!(rects.len(), 2);
        assert_eq!(
            rects[1],
            (scratch, Rect::new(0.0, 0.0, 1000.0, 300.0)),
            "the scratch is on top"
        );
        let _ = panes;
    }

    #[test]
    fn alt_shift_arrows_resize_the_scratch_and_overlays() {
        let (mut mux, panes) = mux_with(1);
        let area = Rect::new(0.0, 0.0, 100.0, 100.0);
        let overlay = mux.new_pane_id();
        mux.open_overlay(overlay, 50);
        assert!(mux.resize_floating(overlay, Edge::Right, 5));
        assert_eq!(mux.overlay().map(|o| o.size), Some(55));
        assert!(mux.resize_floating(overlay, Edge::Up, 10));
        assert_eq!(mux.overlay().map(|o| o.size), Some(45));
        // A pane of the layout is not floating: its dividers move (in the app), not this.
        assert!(!mux.resize_floating(panes[0], Edge::Down, 5));

        let scratch = mux.new_pane_id();
        mux.set_scratch(scratch, 50);
        // The scratch has the whole width: only its bottom edge moves.
        assert!(!mux.resize_floating(scratch, Edge::Right, 5));
        assert!(mux.resize_floating(scratch, Edge::Down, 10));
        let height = |mux: &Mux| {
            mux.floating_rects(area, (0.0, 0.0))
                .into_iter()
                .find(|(p, _)| *p == scratch)
                .map(|(_, r)| r.height.round())
        };
        assert_eq!(height(&mux), Some(60.0));
        // Not bigger than the tab, not smaller than the minimum.
        for _ in 0..20 {
            mux.resize_floating(scratch, Edge::Down, 10);
        }
        assert_eq!(height(&mux), Some(100.0));
        assert!(!mux.resize_floating(scratch, Edge::Down, 10));
        for _ in 0..20 {
            mux.resize_floating(scratch, Edge::Up, 10);
        }
        assert_eq!(height(&mux), Some(f32::from(FLOATING_MIN_SIZE)));
        // A hidden scratch does not change.
        mux.toggle_scratch();
        assert!(!mux.resize_floating(scratch, Edge::Down, 10));
    }

    #[test]
    fn overlay_and_scratch_places() {
        let pane = Rect::new(100.0, 50.0, 400.0, 200.0);
        assert_eq!(
            overlay_rect(pane, 100, (0.0, 0.0)),
            pane,
            "100% is the whole pane"
        );
        assert_eq!(
            overlay_rect(pane, 50, (0.0, 0.0)),
            Rect::new(200.0, 100.0, 200.0, 100.0)
        );
        // Never smaller than the minimum, never bigger than the pane.
        assert_eq!(
            overlay_rect(pane, 10, (300.0, 150.0)),
            Rect::new(150.0, 75.0, 300.0, 150.0)
        );
        assert_eq!(overlay_rect(pane, 10, (900.0, 900.0)), pane);
        let area = Rect::new(0.0, 30.0, 800.0, 600.0);
        assert_eq!(
            scratch_rect(area, 50, 0.0),
            Rect::new(0.0, 30.0, 800.0, 300.0)
        );
        assert_eq!(scratch_rect(area, 200, 0.0), area, "at most all of it");
        assert_eq!(
            scratch_rect(area, 1, 120.0),
            Rect::new(0.0, 30.0, 800.0, 120.0)
        );
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

    #[test]
    fn the_dashboard_covers_the_tab_area_and_has_the_keys() {
        let (mut mux, panes) = mux_with(1);
        let w = mux.new_pane_id();
        mux.set_window_overlay(w, 80);
        let d = mux.new_pane_id();
        assert_eq!(mux.toggle_dashboard(), None);
        mux.set_dashboard(d);
        assert_eq!(mux.input_pane(), Some(d), "over the window overlay");
        assert!(mux.is_floating(d));
        let rects = mux.floating_rects(AREA, (0.0, 0.0));
        assert_eq!(rects.last(), Some(&(d, AREA)), "on top, the whole area");
        // The scratch is still above it.
        let s = mux.new_pane_id();
        mux.set_scratch(s, 50);
        assert_eq!(mux.input_pane(), Some(s));
        mux.toggle_scratch();
        assert_eq!(mux.toggle_dashboard(), Some(false));
        assert_eq!(
            mux.input_pane(),
            Some(w),
            "hidden: the window overlay has the keys"
        );
        assert!(
            !mux.floating_rects(AREA, (0.0, 0.0))
                .iter()
                .any(|(p, _)| *p == d)
        );
        assert_eq!(mux.dashboard(), Some((d, false)));
        assert_eq!(mux.close_pane(d), Closed::Floating);
        assert_eq!(mux.dashboard(), None);
        let _ = panes;
    }

    #[test]
    fn companion_places() {
        let pane = Rect::new(0.0, 0.0, 100.0, 200.0);
        assert_eq!(
            companion_rects(pane, Direction::Down, 40),
            (
                Rect::new(0.0, 0.0, 100.0, 120.0),
                Rect::new(0.0, 120.0, 100.0, 80.0)
            )
        );
        assert_eq!(
            companion_rects(pane, Direction::Right, 40),
            (
                Rect::new(0.0, 0.0, 60.0, 200.0),
                Rect::new(60.0, 0.0, 40.0, 200.0)
            )
        );
        // The size is cut to 10..=90, so both always have a part.
        assert_eq!(companion_rects(pane, Direction::Down, 100).1.height, 180.0);
        assert_eq!(companion_rects(pane, Direction::Down, 0).1.height, 20.0);
    }

    #[test]
    fn alt_shift_arrows_resize_the_companions() {
        let (mut mux, panes) = mux_with(1);
        let a = panes[0];
        // Not in a companion: nothing changes here.
        assert!(!mux.resize_companion(Edge::Up, 5));
        let c = mux.new_pane_id();
        assert!(mux.add_companion(a, c));
        let strip = |mux: &Mux| {
            mux.pane_rects(AREA)
                .into_iter()
                .find(|(p, _)| *p == c)
                .map(|(_, r)| r.height / AREA.height)
        };
        let before = strip(&mux).unwrap();
        // A strip at the bottom: Up moves its line up, so it gets higher.
        assert!(mux.resize_companion(Edge::Up, 10));
        assert!(strip(&mux).unwrap() > before);
        assert!(mux.resize_companion(Edge::Down, 10));
        assert_eq!(strip(&mux), Some(before));
        // Left and Right do not cross a strip at the bottom.
        assert!(!mux.resize_companion(Edge::Right, 10));
        // Not bigger than 90%; at the limit the keys still go to the strip, not to the lines of its pane.
        for _ in 0..20 {
            mux.resize_companion(Edge::Up, 10);
        }
        let biggest = strip(&mux);
        assert!(mux.resize_companion(Edge::Up, 10));
        assert_eq!(strip(&mux), biggest);
        // A strip at the right: Left makes it wider.
        mux.set_companion_place(Direction::Right, 40);
        assert!(mux.resize_companion(Edge::Left, 5));
        assert!(!mux.resize_companion(Edge::Up, 5));
    }

    #[test]
    fn a_companion_slides_out_inside_its_pane() {
        let (mut mux, panes) = mux_with(1);
        let a = panes[0];
        let b = mux.new_pane_id();
        mux.split_active(b, Direction::Right);
        mux.focus(a);
        let before = mux.pane_rects(AREA);
        let c = mux.new_pane_id();
        assert!(mux.add_companion(a, c));
        assert_eq!(mux.active_pane(), Some(c), "the new companion has the keys");
        assert_eq!(mux.owner_of(c), Some(a));
        assert_eq!(mux.pane_tab(c), mux.pane_tab(a));
        let rects = mux.pane_rects(AREA);
        let rect = |p| rects.iter().find(|(id, _)| *id == p).map(|(_, r)| *r);
        let owner = before.iter().find(|(id, _)| *id == a).unwrap().1;
        assert_eq!(
            (rect(a), rect(c)),
            {
                let (top, strip) = companion_rects(owner, Direction::Down, 40);
                (Some(top), Some(strip))
            },
            "the strip is in the rect of its pane"
        );
        assert_eq!(
            rect(b),
            before.iter().find(|(id, _)| *id == b).map(|(_, r)| *r),
            "the layout does not change"
        );
        // Hide: the program lives on, the pane is whole again and has the keys.
        assert_eq!(mux.toggle_companion(), Some(false));
        assert_eq!(mux.pane_rects(AREA), before);
        assert_eq!(mux.active_pane(), Some(a));
        assert_eq!(mux.companions_of(a), [c]);
        // Show it again from the owner.
        assert_eq!(mux.toggle_companion(), Some(true));
        assert_eq!(mux.active_pane(), Some(c));
        // The keys from the companion: it hides.
        assert_eq!(mux.toggle_companion(), Some(false));
        // A pane with no companions.
        mux.focus(b);
        assert_eq!(mux.toggle_companion(), None);
        assert_eq!(mux.next_companion(), None);
    }

    #[test]
    fn the_side_and_size_of_companions_can_change() {
        let (mut mux, panes) = mux_with(1);
        let c = mux.new_pane_id();
        mux.add_companion(panes[0], c);
        mux.set_companion_place(Direction::Right, 25);
        assert_eq!(
            mux.pane_rects(AREA),
            [
                (panes[0], Rect::new(0.0, 0.0, 600.0, 600.0)),
                (c, Rect::new(600.0, 0.0, 200.0, 600.0))
            ]
        );
    }

    #[test]
    fn several_companions_one_at_a_time() {
        let (mut mux, panes) = mux_with(1);
        let a = panes[0];
        let (c1, c2) = (mux.new_pane_id(), mux.new_pane_id());
        mux.add_companion(a, c1);
        mux.add_companion(a, c2);
        let shown = |mux: &Mux| {
            mux.pane_rects(AREA)
                .into_iter()
                .map(|(p, _)| p)
                .collect::<Vec<_>>()
        };
        assert_eq!(shown(&mux), [a, c2], "the new one is shown");
        assert_eq!(mux.next_companion(), Some(c1));
        assert_eq!(shown(&mux), [a, c1]);
        assert_eq!(mux.active_pane(), Some(c1));
        assert_eq!(mux.next_companion(), Some(c2), "it wraps around");
        // Hidden, `next` shows the next one.
        mux.toggle_companion();
        assert_eq!(mux.next_companion(), Some(c1));
        assert_eq!(shown(&mux), [a, c1]);
        // Hide and show again: the last shown one comes back.
        mux.toggle_companion();
        mux.toggle_companion();
        assert_eq!(shown(&mux), [a, c1]);
    }

    #[test]
    fn companions_close_alone_or_with_their_pane() {
        let (mut mux, panes) = mux_with(1);
        let a = panes[0];
        let b = mux.new_pane_id();
        mux.split_active(b, Direction::Down);
        mux.focus(a);
        let (c1, c2) = (mux.new_pane_id(), mux.new_pane_id());
        mux.add_companion(a, c1);
        mux.add_companion(a, c2);
        // The shown one ends: the owner has the keys, the other one is hidden.
        assert_eq!(mux.close_pane(c2), Closed::Floating);
        assert_eq!(mux.active_pane(), Some(a));
        assert_eq!(mux.companions_of(a), [c1]);
        assert!(!mux.pane_rects(AREA).iter().any(|(p, _)| *p == c1));
        // A hidden one ends: nothing else changes.
        mux.add_companion(a, c2);
        mux.toggle_companion();
        assert_eq!(mux.close_pane(c1), Closed::Floating);
        assert_eq!(mux.companions_of(a), [c2]);
        assert_eq!(mux.toggle_companion(), Some(true));
        // The owner closes: its companions go with it.
        assert_eq!(mux.close_pane(a), Closed::Pane);
        assert_eq!(mux.companions_of(a), []);
        assert_eq!(mux.owner_of(c2), None);
        assert_eq!(mux.pane_tab(c2), None);
        assert_eq!(mux.pane_rects(AREA), [(b, AREA)]);
    }

    #[test]
    fn a_companion_zooms_and_moves_with_its_pane() {
        let (mut mux, panes) = mux_with(1);
        let a = panes[0];
        let b = mux.new_pane_id();
        mux.split_active(b, Direction::Right);
        mux.focus(a);
        mux.toggle_zoom();
        let c = mux.new_pane_id();
        mux.add_companion(a, c);
        let (top, strip) = companion_rects(AREA, Direction::Down, 40);
        assert_eq!(mux.pane_rects(AREA), [(a, top), (c, strip)]);
        // A split from the companion splits its owner.
        let d = mux.new_pane_id();
        mux.split_active(d, Direction::Right);
        assert_eq!(mux.active_tab().unwrap().layout.panes(), [a, d, b]);
        // The tab moves: the companion goes with it.
        let tab = mux.pane_tab(a).unwrap();
        mux.move_tab_to(tab, "work");
        assert_eq!(mux.pane_place(c).map(|(w, _)| w), Some("work"));
    }

    #[test]
    fn closing_a_tab_gives_its_overlay_and_companions_too() {
        let (mut mux, panes) = mux_with(2);
        let (o, c) = (mux.new_pane_id(), mux.new_pane_id());
        mux.add_companion(panes[0], c);
        mux.toggle_companion();
        assert!(mux.open_overlay(o, 80));
        let tab = mux.pane_tab(panes[0]).unwrap();
        assert_eq!(mux.close_tab(tab), [panes[0], o, c]);
    }

    #[test]
    fn a_line_between_a_pane_and_its_strip() {
        let (mut mux, panes) = mux_with(1);
        assert_eq!(mux.companion_dividers(AREA), []);
        let c = mux.new_pane_id();
        mux.add_companion(panes[0], c);
        assert_eq!(
            mux.companion_dividers(AREA),
            [Rect::new(0.0, 360.0, 800.0, 1.0)]
        );
        mux.set_companion_place(Direction::Right, 50);
        assert_eq!(
            mux.companion_dividers(AREA),
            [Rect::new(400.0, 0.0, 1.0, 600.0)]
        );
        mux.toggle_companion();
        assert_eq!(mux.companion_dividers(AREA), [], "hidden: no line");
    }

    #[test]
    fn a_split_of_a_pane_in_another_tab_keeps_the_focus_of_the_user() {
        // The bug: an agent in tab 0 opened a pane while the user worked in tab 1; the new pane went into tab 1
        // and took the keys.
        let (mut mux, panes) = mux_with(2);
        let (agent, user) = (panes[0], panes[1]);
        mux.select(1);
        let new = mux.new_pane_id();
        assert!(mux.split_pane(agent, new, Direction::Right));
        assert_eq!(mux.active_index(), 1, "the user stays in their tab");
        assert_eq!(mux.active_pane(), Some(user), "and keeps the keys");
        let tab0 = &mux.tabs()[0];
        assert_eq!(
            tab0.layout.panes(),
            [agent, new],
            "the new pane is next to the agent"
        );
        assert_eq!(
            tab0.active_pane, agent,
            "the agent's tab keeps its active pane too"
        );
        assert_eq!(mux.tabs()[1].layout.panes(), [user]);
    }

    #[test]
    fn a_split_from_an_overlay_or_a_companion_goes_next_to_its_pane() {
        let (mut mux, panes) = mux_with(2);
        let (a, user) = (panes[0], panes[1]);
        let o = mux.new_pane_id();
        assert!(mux.open_overlay(o, 80), "an overlay over `a` in tab 0");
        let c = mux.new_pane_id();
        mux.add_companion(a, c);
        mux.select(1);
        let from_overlay = mux.new_pane_id();
        assert!(mux.split_pane(o, from_overlay, Direction::Right));
        let from_companion = mux.new_pane_id();
        assert!(mux.split_pane(c, from_companion, Direction::Down));
        let mut tab0 = mux.tabs()[0].layout.panes();
        tab0.sort();
        let mut want = vec![a, from_overlay, from_companion];
        want.sort();
        assert_eq!(tab0, want, "both next to `a`, in tab 0");
        assert_eq!(mux.tabs()[1].layout.panes(), [user]);
        assert_eq!(mux.active_pane(), Some(user));
    }

    #[test]
    fn a_split_of_a_pane_in_the_active_tab_keeps_the_focus_too() {
        let (mut mux, panes) = mux_with(1);
        let a = panes[0];
        let new = mux.new_pane_id();
        assert!(mux.split_pane(a, new, Direction::Down));
        assert_eq!(mux.active_pane(), Some(a));
        assert_eq!(mux.tabs()[0].layout.panes(), [a, new]);
        let other = mux.new_pane_id();
        assert!(
            !mux.split_pane(PaneId(999), other, Direction::Down),
            "no such pane"
        );
    }

    #[test]
    fn an_overlay_from_a_companion_goes_over_its_pane() {
        let (mut mux, panes) = mux_with(1);
        let a = panes[0];
        let c = mux.new_pane_id();
        mux.add_companion(a, c);
        assert_eq!(mux.active_pane(), Some(c), "the companion has the keys");
        let o = mux.new_pane_id();
        assert!(mux.open_overlay(o, 80));
        assert_eq!(
            mux.overlay().map(|o| o.over),
            Some(a),
            "over the pane, not over its companion"
        );
        let rects = mux.floating_rects(AREA, (0.0, 0.0));
        let owner_rect = mux
            .pane_rects(AREA)
            .into_iter()
            .find(|(p, _)| *p == a)
            .unwrap()
            .1;
        assert_eq!(rects, [(o, overlay_rect(owner_rect, 80, (0.0, 0.0)))]);
        // The companion ends: the overlay of its pane stays.
        assert_eq!(mux.close_pane(c), Closed::Floating);
        assert_eq!(mux.overlay().map(|o| o.pane), Some(o));
    }

    #[test]
    fn focus_and_reveal_of_a_companion() {
        let (mut mux, panes) = mux_with(2);
        let a = panes[0];
        let c = mux.new_pane_id();
        mux.add_companion(a, c);
        mux.focus(a);
        // A shown one takes the focus (a click on it).
        mux.focus(c);
        assert_eq!(mux.active_pane(), Some(c));
        // A hidden one is shown by `reveal` (from another tab too).
        mux.toggle_companion();
        mux.select(1);
        assert!(mux.reveal(c));
        assert_eq!(mux.active_pane(), Some(c));
        assert!(mux.pane_rects(AREA).iter().any(|(p, _)| *p == c));
        // `focus` of a hidden one does nothing.
        mux.toggle_companion();
        mux.focus(c);
        assert_eq!(mux.active_pane(), Some(a));
    }
}
