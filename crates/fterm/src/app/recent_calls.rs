//! Recent panes: go back to the previous pane (Alt+`), and the list of panes, the recently used first (Ctrl+Shift+H).

use super::*;
use crate::history_popup::{HistoryPopup, PopupKind, PopupRow};

/// What a row of the pane list shows.
pub(super) struct PaneInfo<'a> {
    pub id: PaneId,
    /// The number of its tab (from 1), and its workspace when there are several.
    pub tab: usize,
    pub workspace: Option<&'a str>,
    /// The title from the program, else the program.
    pub title: &'a str,
    pub cwd: Option<&'a str>,
    /// The state of an agent in it (`working`, `waiting`, ...).
    pub agent: Option<&'a str>,
    pub companion: bool,
}

/// A row of the pane list: "tab: title" and the folder, the agent state, and what it is.
pub(super) fn pane_row(info: &PaneInfo<'_>) -> PopupRow {
    let place = match info.workspace {
        Some(ws) => format!("{ws} {}", info.tab),
        None => info.tab.to_string(),
    };
    let mut hint = Vec::new();
    if info.companion {
        hint.push(tr!("popup.pane_companion"));
    }
    if let Some(agent) = info.agent {
        hint.push(agent.to_owned());
    }
    // Only the name of the folder: a whole path would hide the title.
    if let Some(folder) = info
        .cwd
        .map(|cwd| cwd.trim_end_matches(['/', '\\']))
        .and_then(|cwd| cwd.rsplit(['/', '\\']).next())
        .filter(|f| !f.is_empty())
    {
        hint.push(folder.to_owned());
    }
    PopupRow {
        text: format!("{place}: {}", info.title),
        hint: hint.join(" · "),
        key: info.id.0.to_string(),
        ..Default::default()
    }
}

impl App {
    /// The panes that the list may show, in the order of the tabs: the panes of each layout, then its companions
    /// (not overlays or the scratch: they go with their key).
    fn listed_panes(&self) -> Vec<PaneId> {
        let Some(running) = &self.running else {
            return Vec::new();
        };
        running
            .mux
            .all_tabs()
            .flat_map(|t| {
                t.layout
                    .panes()
                    .into_iter()
                    .chain(t.companions.iter().flat_map(|c| c.panes.clone()))
            })
            .filter(|p| running.panes.contains_key(p))
            .collect()
    }

    /// The keys are in this pane now: it may count as visited (see `Recent`).
    pub(super) fn note_focus(&mut self, now: Instant) {
        let Some(running) = &self.running else {
            return;
        };
        if let Some(pane) = running.mux.input_pane()
            && !running.mux.is_floating(pane)
        {
            self.recent.focus(pane, now);
        }
    }

    /// `previous_pane`: back to the pane that had the keys before.
    pub(super) fn go_previous_pane(&mut self) {
        let listed = self.listed_panes();
        if let Some(pane) = self.recent.previous(|p| listed.contains(&p)) {
            self.go_to_pane(pane);
        }
    }

    /// `recent_panes`: the list of panes, the recently used first.
    pub(super) fn open_recent_popup(&mut self) {
        self.palette = None;
        self.history_popup = Some(HistoryPopup::new(
            PopupKind::Panes,
            Vec::new(),
            String::new(),
            None,
        ));
        self.refresh_history_popup();
    }

    pub(super) fn recent_popup_rows(&self) -> Vec<PopupRow> {
        let Some(running) = &self.running else {
            return Vec::new();
        };
        let several = running.mux.workspaces().len() > 1;
        self.recent
            .order(&self.listed_panes())
            .into_iter()
            .filter_map(|id| {
                let p = running.panes.get(&id)?;
                let (workspace, index) = running.mux.pane_place(id)?;
                Some(pane_row(&PaneInfo {
                    id,
                    tab: index + 1,
                    workspace: several.then_some(workspace),
                    // Like the tab title (`C:\...\powershell.exe` = `powershell`).
                    title: fterm_mux::mux::tab_title(
                        None,
                        p.app_title.as_deref(),
                        display_name(p.session.program()),
                    ),
                    cwd: p.shell.cwd.as_deref(),
                    agent: p.agent.as_ref().map(|a| a.kind.name()),
                    companion: running.mux.owner_of(id).is_some(),
                }))
            })
            .collect()
    }

    /// Enter in the pane list.
    pub(super) fn accept_recent_popup(&mut self, key: &str) {
        if let Ok(id) = key.parse() {
            self.go_to_pane(PaneId(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_of_the_pane_list() {
        let info = PaneInfo {
            id: PaneId(7),
            tab: 2,
            workspace: None,
            title: "claude",
            cwd: Some("C:/work/fterm"),
            agent: Some("waiting"),
            companion: false,
        };
        let row = pane_row(&info);
        assert_eq!(row.text, "2: claude");
        assert_eq!(row.hint, "waiting · fterm");
        let windows = pane_row(&PaneInfo {
            cwd: Some(r"C:\work\fterm\"),
            agent: None,
            ..info
        });
        assert_eq!(windows.hint, "fterm", "the last folder of a Windows path");
        assert_eq!(row.key, "7");
        let row = pane_row(&PaneInfo {
            workspace: Some("agents"),
            agent: None,
            cwd: None,
            companion: true,
            ..info
        });
        assert_eq!(row.text, "agents 2: claude");
        assert_eq!(row.hint, "companion");
    }
}
