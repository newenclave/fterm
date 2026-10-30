//! Companions: panes that belong to a pane (a log, a watcher, a second shell), shown as a strip of it.

use super::*;

/// The side of the pane where its companions are.
pub(super) fn companion_side(side: fterm_config::load::CompanionSide) -> Direction {
    match side {
        fterm_config::load::CompanionSide::Bottom => Direction::Down,
        fterm_config::load::CompanionSide::Right => Direction::Right,
    }
}

impl App {
    /// Opens a companion of `owner` (`None` = the active pane, or its owner when it is a companion): `command` run by
    /// the shell, or the shell itself. It is shown and gets the keys.
    pub(super) fn open_companion(
        &mut self,
        command: Option<String>,
        owner: Option<PaneId>,
    ) -> anyhow::Result<PaneId> {
        let running = self.running.as_mut().expect("the window is open");
        let owner = match owner {
            Some(owner) => owner,
            None => {
                let active = running
                    .mux
                    .active_pane()
                    .ok_or_else(|| anyhow::anyhow!("no pane"))?;
                running.mux.owner_of(active).unwrap_or(active)
            }
        };
        // The owner is in front, so the new companion is in its tab.
        running.mux.reveal(owner);
        let area = running.tab_area();
        let rect = running
            .mux
            .pane_rects(area)
            .into_iter()
            .find(|(p, _)| *p == owner)
            .map_or(area, |(_, r)| r);
        let side = companion_side(self.config.config.companion_side);
        let (_, strip) =
            fterm_mux::mux::companion_rects(rect, side, self.config.config.companion_size);
        let grid = running.grid_for(strip);
        self.spawn_command = command;
        let id = self.spawn_pane(grid, None);
        self.spawn_command = None;
        let id = id?;
        let running = self.running.as_mut().expect("the window is open");
        if !running.mux.add_companion(owner, id) {
            running.panes.remove(&id);
            anyhow::bail!("pane {} is not in a tab", owner.0);
        }
        tracing::info!(pane = id.0, owner = owner.0, "a companion opened");
        self.tab_changed();
        self.update_ime();
        Ok(id)
    }

    /// Shows or hides the companion of the active pane; with none, asks for the command of a new one.
    pub(super) fn toggle_companion(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        if running.mux.toggle_companion().is_none() {
            return self.start_overlay_prompt(overlay_calls::PromptFor::Companion);
        }
        self.tab_changed();
        self.update_ime();
    }

    /// Shows the next companion of the active pane.
    pub(super) fn next_companion(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        if running.mux.next_companion().is_some() {
            self.tab_changed();
            self.update_ime();
        }
    }
}
