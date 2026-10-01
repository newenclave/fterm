//! Restore the session (Phase 9b): save the window every 30 seconds and at the end, and bring it back.

use std::time::{Duration, Instant};

use fterm_config::load::Restore;
use winit::dpi::{PhysicalPosition, PhysicalSize};

use super::*;
use crate::session_state::{
    SavedDock, SavedPane, SavedSession, SavedTab, SavedWindow, VERSION, default_path, load,
    restore_layout, save, save_layout,
};

/// How often the session is saved (so a crash or a reboot loses little).
pub(super) const AUTOSAVE: Duration = Duration::from_secs(30);

impl App {
    /// What the window has now.
    fn capture_session(&self) -> Option<SavedSession> {
        let running = self.running.as_ref()?;
        if running.mux.tabs().is_empty() {
            return None;
        }
        let pane_of = |id: PaneId| {
            running
                .panes
                .get(&id)
                .map(|p| SavedPane {
                    profile: p.profile.clone(),
                    cwd: p.shell.cwd.clone(),
                    program: display_name(p.session.program()).to_owned(),
                })
                .unwrap_or_default()
        };
        let tabs = running
            .mux
            .tabs()
            .iter()
            .map(|tab| SavedTab {
                title: tab.custom_title.clone(),
                active: tab
                    .layout
                    .panes()
                    .iter()
                    .position(|p| *p == tab.active_pane)
                    .unwrap_or(0),
                layout: save_layout(&tab.layout, &pane_of),
            })
            .collect();
        let size = running.window.inner_size();
        let window = running.window.outer_position().ok().map(|pos| SavedWindow {
            width: size.width,
            height: size.height,
            x: pos.x,
            y: pos.y,
            maximized: running.window.is_maximized(),
        });
        let panel = match running.dock.active {
            PanelKind::Events => "events",
            PanelKind::Agents => "agents",
            PanelKind::Ai => "ai",
        };
        Some(SavedSession {
            version: VERSION,
            saved: now_ms(),
            window,
            active_tab: running.mux.active_index(),
            tabs,
            dock: SavedDock {
                open: running.dock.open,
                panel: Some(panel.to_owned()),
                ratio: Some(running.dock.ratio),
            },
        })
    }

    /// Saves the session now (when there is something to save).
    pub(super) fn save_session(&mut self) {
        self.session_saved_at = Some(Instant::now());
        if self.config.config.restore == Restore::Never {
            return;
        }
        let (Some(path), Some(session)) = (default_path(), self.capture_session()) else {
            return;
        };
        if let Err(err) = save(&path, &session) {
            tracing::warn!(path = %path.display(), "cannot save the session: {err}");
        }
    }

    /// When fterm closes: save what is open. When the last tab was closed, there is nothing to bring back.
    pub(super) fn save_or_forget_session(&mut self) {
        let has_tabs = self
            .running
            .as_ref()
            .is_some_and(|r| !r.mux.tabs().is_empty());
        if has_tabs {
            self.save_session();
        } else if let Some(path) = default_path() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// The next autosave, for the event loop to wake up.
    pub(super) fn autosave(&mut self, now: Instant) -> Instant {
        let at = self.session_saved_at.unwrap_or(now) + AUTOSAVE;
        if now >= at {
            self.save_session();
            return now + AUTOSAVE;
        }
        at
    }

    /// At start: bring back the last session, or ask first (only when no other fterm window runs).
    pub(super) fn offer_restore(&mut self) {
        self.session_saved_at = Some(Instant::now());
        if self.config.config.restore == Restore::Never {
            return;
        }
        let others = fterm_api::discovery::instances(&fterm_api::discovery::instances_dir())
            .into_iter()
            .any(|i| i.pid != std::process::id());
        if others {
            return;
        }
        let Some(saved) = default_path().and_then(|path| load(&path)) else {
            return;
        };
        match self.config.config.restore {
            Restore::Always => self.restore_session(saved),
            _ => {
                self.restore_offer = Some(saved);
                if let Some(running) = &self.running {
                    running.window.request_redraw();
                }
            }
        }
    }

    pub(super) fn restore_question_lines(&self) -> Option<Vec<String>> {
        let saved = self.restore_offer.as_ref()?;
        Some(vec![
            "Restore the last session?".to_owned(),
            saved.describe(now_ms()),
            String::new(),
            "Enter = restore, Esc = no".to_owned(),
        ])
    }

    pub(super) fn restore_key(&mut self, event: &KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => {
                if let Some(saved) = self.restore_offer.take() {
                    self.restore_session(saved);
                }
            }
            Key::Named(NamedKey::Escape) => self.restore_offer = None,
            _ => return,
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// The palette command: bring back the last saved session now.
    pub(super) fn restore_last_session(&mut self) {
        match default_path().and_then(|path| load(&path)) {
            Some(saved) => self.restore_session(saved),
            None => self.notify(None, "No saved session", "", Level::Info, Source::App),
        }
    }

    fn restore_session(&mut self, saved: SavedSession) {
        let Some(running) = &self.running else {
            return;
        };
        // The first tab of a new window: when nothing happened in it yet, the restored tabs take its place.
        let fresh = match running.mux.tabs() {
            [tab] if tab.layout.panes().len() == 1 => running
                .panes
                .get(&tab.active_pane)
                .filter(|p| p.last_command.is_none() && !p.shell.is_running())
                .map(|_| tab.id),
            _ => None,
        };
        let size = running.grid_for(running.tab_area());
        let first = running.mux.tabs().len();
        let mut opened = 0;
        for tab in &saved.tabs {
            let layout = restore_layout(&tab.layout, &mut |pane| {
                self.spawn_cwd = pane.cwd.clone();
                match self.spawn_pane(size, pane.profile.as_deref()) {
                    Ok(id) => Some(id),
                    Err(err) => {
                        tracing::warn!("cannot restore a pane: {err:#}");
                        None
                    }
                }
            });
            self.spawn_cwd = None;
            let Some(layout) = layout else {
                continue;
            };
            let panes = layout.panes();
            let active = panes[tab.active.min(panes.len() - 1)];
            if let Some(running) = &mut self.running {
                running.mux.add_tab(layout, active, tab.title.clone());
                opened += 1;
            }
        }
        if opened == 0 {
            return self.notify(None, "Nothing to restore", "", Level::Info, Source::App);
        }
        let mut first = first;
        if let Some(tab) = fresh {
            self.close_tab_now(tab);
            first -= 1;
        }
        let Some(running) = &mut self.running else {
            return;
        };
        running.mux.select(first + saved.active_tab.min(opened - 1));
        running.dock.open = saved.dock.open;
        if let Some(kind) = saved.dock.panel.as_deref().and_then(PanelKind::from_name) {
            running.dock.active = kind;
        }
        if let Some(ratio) = saved.dock.ratio {
            running.dock.ratio = ratio.clamp(0.1, 0.9);
        }
        if let Some(w) = saved.window {
            if w.maximized {
                running.window.set_maximized(true);
            } else {
                let _ = running
                    .window
                    .request_inner_size(PhysicalSize::new(w.width.max(320), w.height.max(200)));
                // Only a place that is on a monitor now (monitors can change).
                let on_screen = running.window.available_monitors().any(|m| {
                    let (p, s) = (m.position(), m.size());
                    w.x >= p.x
                        && w.y >= p.y
                        && w.x < p.x + s.width as i32
                        && w.y < p.y + s.height as i32
                });
                if on_screen {
                    running
                        .window
                        .set_outer_position(PhysicalPosition::new(w.x, w.y));
                }
            }
        }
        tracing::info!(tabs = opened, "session restored");
        self.resize_all_panes();
        self.tab_changed();
    }
}
