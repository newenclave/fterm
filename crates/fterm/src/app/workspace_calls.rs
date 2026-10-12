//! Workspaces: the list (go to one, make one, move a tab there), the next and the previous one, a new name,
//! and the name at the left end of the tab bar.

use super::*;
use crate::history_popup::{HistoryPopup, PopupKind, PopupRow};

/// The rows of the list of workspaces: the name, how many tabs, and which one is in use.
pub(super) fn workspace_rows(workspaces: &[(String, usize)], current: &str) -> Vec<PopupRow> {
    workspaces
        .iter()
        .map(|(name, tabs)| {
            let count = trn!("count.tabs", *tabs);
            PopupRow {
                text: name.clone(),
                hint: if name == current {
                    tr!("popup.workspace_in_use", tabs = count)
                } else {
                    count
                },
                key: name.clone(),
                ..Default::default()
            }
        })
        .collect()
}

impl App {
    /// `choose_workspace` (or `move_tab_to_workspace` when `move_tab`): the list of workspaces.
    pub(super) fn open_workspace_popup(&mut self, move_tab: bool) {
        self.palette = None;
        let kind = if move_tab {
            PopupKind::MoveTab
        } else {
            PopupKind::Workspaces
        };
        self.history_popup = Some(HistoryPopup::new(kind, Vec::new(), String::new(), None));
        self.refresh_history_popup();
    }

    pub(super) fn workspace_popup_rows(&self) -> Vec<PopupRow> {
        let Some(running) = &self.running else {
            return Vec::new();
        };
        workspace_rows(&running.mux.workspaces(), running.mux.workspace())
    }

    /// Enter in the list: the chosen workspace, or the typed name when no row is chosen (a new workspace).
    pub(super) fn accept_workspace_popup(&mut self, kind: PopupKind, name: String) {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return;
        }
        let Some(running) = &mut self.running else {
            return;
        };
        match kind {
            PopupKind::MoveTab => {
                if let Some(tab) = running.mux.active_tab().map(|t| t.id) {
                    running.mux.move_tab_to(tab, &name);
                }
            }
            _ => {
                if !running.mux.switch_workspace(&name) {
                    // A new workspace starts with a new tab.
                    self.spawn_workspace = Some(name);
                    self.spawn(None, SpawnWhere::Tab);
                    return;
                }
            }
        }
        self.workspace_changed();
    }

    /// `next_workspace` / `prev_workspace`.
    pub(super) fn step_workspace(&mut self, step: i32) {
        if let Some(running) = &mut self.running {
            running.mux.cycle_workspace(step);
        }
        self.workspace_changed();
    }

    fn workspace_changed(&mut self) {
        self.hints = None;
        if let Some(running) = &self.running {
            running.resize_all_panes();
        }
        self.tab_changed();
    }

    /// `rename_workspace`: a box with the name in use.
    pub(super) fn start_workspace_prompt(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let mut input = crate::ai_chat::InputBox::default();
        input.set(running.mux.workspace());
        self.workspace_prompt = Some(input);
        running.window.request_redraw();
    }

    pub(super) fn workspace_prompt_lines(&self) -> Option<Vec<String>> {
        let input = self.workspace_prompt.as_ref()?;
        Some(vec![
            tr!("box.rename_workspace"),
            format!("{}▏", input.text),
            String::new(),
            tr!("box.rename_workspace_keys"),
        ])
    }

    pub(super) fn workspace_prompt_key(&mut self, event: &KeyEvent) {
        let ctrl = self.mods.control_key();
        let Some(input) = &mut self.workspace_prompt else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.workspace_prompt = None,
            Key::Named(NamedKey::Enter) => {
                let name = input.take();
                self.workspace_prompt = None;
                if let Some(running) = &mut self.running {
                    let old = running.mux.workspace().to_owned();
                    if name.trim() != old && !running.mux.rename_workspace(&old, &name) {
                        self.notify(
                            None,
                            &tr!("toast.workspace_name_taken"),
                            name.trim(),
                            Level::Info,
                            Source::App,
                        );
                    }
                }
                self.tab_changed();
            }
            Key::Named(NamedKey::Backspace) => input.backspace(),
            _ => {
                if let Some(text) = event.text.as_deref()
                    && !ctrl
                    && !self.mods.alt_key()
                {
                    let text: String = text.chars().filter(|c| !c.is_control()).collect();
                    input.insert(&text);
                }
            }
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// The name at the left end of the tab bar (only with more than one workspace), and a dot when an agent
    /// in another workspace waits or failed.
    pub(super) fn workspace_chip(
        &self,
    ) -> Option<(
        String,
        Option<fterm_term::alacritty_terminal::vte::ansi::Rgb>,
    )> {
        let running = self.running.as_ref()?;
        if running.mux.workspaces().len() < 2 {
            return None;
        }
        let current = running.mux.workspace();
        let others = running
            .mux
            .all_tabs()
            .filter(|t| running.mux.tab_workspace(t.id) != Some(current))
            .flat_map(|t| t.layout.panes())
            .filter_map(|p| running.panes.get(&p)?.agent.as_ref().map(|a| a.kind));
        let mut dot = None;
        for kind in others {
            match kind {
                AgentKind::Waiting => {
                    dot = Some(self.ui.agent_waiting);
                    break;
                }
                AgentKind::Error => dot = Some(self.ui.agent_error),
                _ => {}
            }
        }
        Some((current.to_owned(), dot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_of_the_workspace_list() {
        let ws = vec![("main".to_owned(), 2), ("agents".to_owned(), 1)];
        let rows = workspace_rows(&ws, "agents");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].text.as_str(), rows[0].hint.as_str()),
            ("main", "2 tabs")
        );
        assert_eq!(rows[1].hint, "1 tab · in use");
        assert_eq!(rows[1].key, "agents");
    }
}
