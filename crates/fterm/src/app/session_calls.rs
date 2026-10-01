//! Restore the session (Phase 9b): save the window every 30 seconds and at the end, and bring it back.

use std::time::{Duration, Instant};

use fterm_config::load::{Rerun, Restore};
use fterm_term::input::{lines_text, total_lines};

use crate::ai_chat::InputBox;
use winit::dpi::{PhysicalPosition, PhysicalSize};

use super::*;
use crate::history_popup::{HistoryPopup, PopupKind, PopupRow};
use crate::session_state::{
    Entry, EntryKind, SavedDock, SavedPane, SavedSession, SavedTab, SavedWindow, VERSION,
    adopt_dead, close_live, entry_text, intro_bytes, last_lines, list, live_path, load,
    newest_closed, rerun_command, restore_layout, save, save_layout, save_named, sessions_dir,
    through_hook,
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
        let lines = self.config.config.restore_history;
        let pane_of = |id: PaneId| {
            running
                .panes
                .get(&id)
                .map(|p| SavedPane {
                    profile: p.profile.clone(),
                    cwd: p.shell.cwd.clone(),
                    program: display_name(p.session.program()).to_owned(),
                    ran: p.shell.running_command().map(str::to_owned),
                    text: if lines == 0 {
                        Vec::new()
                    } else {
                        p.session.with_term(|term| {
                            let total = total_lines(term);
                            last_lines(
                                &lines_text(term, total.saturating_sub(lines + 50), total),
                                lines,
                            )
                        })
                    },
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
            name: None,
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

    /// Saves this window to its live file (when there is something to save).
    pub(super) fn save_session(&mut self) {
        self.session_saved_at = Some(Instant::now());
        if self.config.config.restore == Restore::Never {
            return;
        }
        let (Some(dir), Some(session)) = (sessions_dir(), self.capture_session()) else {
            return;
        };
        let path = live_path(&dir, std::process::id());
        if let Err(err) = save(&path, &session) {
            tracing::warn!(path = %path.display(), "cannot save the session: {err}");
        }
    }

    /// When fterm closes: what is open goes to the list of closed sessions. When the last tab was
    /// closed by hand, there is nothing to bring back.
    pub(super) fn save_or_forget_session(&mut self) {
        let Some(dir) = sessions_dir() else {
            return;
        };
        let has_tabs = self
            .running
            .as_ref()
            .is_some_and(|r| !r.mux.tabs().is_empty());
        if has_tabs {
            self.save_session();
            close_live(&dir, std::process::id());
        } else {
            let _ = std::fs::remove_file(live_path(&dir, std::process::id()));
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

    /// At start: bring back the last closed session, or ask first (only when no other fterm window runs).
    pub(super) fn offer_restore(&mut self) {
        self.session_saved_at = Some(Instant::now());
        let Some(dir) = sessions_dir() else {
            return;
        };
        let alive: Vec<u32> =
            fterm_api::discovery::instances(&fterm_api::discovery::instances_dir())
                .into_iter()
                .map(|i| i.pid)
                .chain([std::process::id()])
                .collect();
        // Windows that crashed: their sessions are closed ones now.
        adopt_dead(&dir, &alive);
        if self.config.config.restore == Restore::Never || alive.len() > 1 {
            return;
        }
        let Some(entry) = newest_closed(&dir) else {
            return;
        };
        match self.config.config.restore {
            Restore::Always => self.restore_from(entry),
            _ => {
                self.restore_offer = Some(entry);
                if let Some(running) = &self.running {
                    running.window.request_redraw();
                }
            }
        }
    }

    pub(super) fn restore_question_lines(&self) -> Option<Vec<String>> {
        let entry = self.restore_offer.as_ref()?;
        Some(vec![
            "Restore the last session?".to_owned(),
            entry.session.describe(now_ms()),
            String::new(),
            "Enter = restore, Esc = no (Ctrl+Shift+S: all sessions)".to_owned(),
        ])
    }

    pub(super) fn restore_key(&mut self, event: &KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => {
                if let Some(entry) = self.restore_offer.take() {
                    self.restore_from(entry);
                }
            }
            Key::Named(NamedKey::Escape) => self.restore_offer = None,
            _ => return,
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// The palette command: bring back the newest closed session now.
    pub(super) fn restore_last_session(&mut self) {
        match sessions_dir().and_then(|dir| newest_closed(&dir)) {
            Some(entry) => self.restore_from(entry),
            None => self.notify(None, "No saved session", "", Level::Info, Source::App),
        }
    }

    /// Restores a session of the list. A closed one leaves the list (it is open again);
    /// a named one stays.
    fn restore_from(&mut self, entry: Entry) {
        let session = match through_hook(&self.config, &entry.session) {
            Ok(Some(session)) => session,
            // The config said no: the session stays in the list.
            Ok(None) => {
                return self.notify(
                    None,
                    "Not restored",
                    "on_restore in your config said no.",
                    Level::Info,
                    Source::App,
                );
            }
            // A broken function must not lose the session.
            Err(err) => {
                self.notify(None, "on_restore failed", &err, Level::Error, Source::App);
                entry.session
            }
        };
        if entry.kind == EntryKind::Closed {
            let _ = std::fs::remove_file(&entry.path);
        }
        self.restore_session(session);
    }

    /// Enter in the sessions list: the row key is the file of the session.
    pub(super) fn restore_entry(&mut self, path: &str) {
        let path = std::path::PathBuf::from(path);
        let kind = if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("named-"))
        {
            EntryKind::Named
        } else {
            EntryKind::Closed
        };
        match load(&path) {
            Some(session) => self.restore_from(Entry {
                path,
                kind,
                session,
            }),
            None => self.notify(None, "The session is gone", "", Level::Warning, Source::App),
        }
    }

    /// The rows of the sessions list.
    pub(super) fn session_rows(&self) -> Vec<PopupRow> {
        let Some(dir) = sessions_dir() else {
            return Vec::new();
        };
        let now = now_ms();
        list(&dir)
            .into_iter()
            .map(|entry| {
                let (text, hint) = entry_text(&entry, now);
                PopupRow {
                    text,
                    hint,
                    bad: false,
                    key: entry.path.display().to_string(),
                }
            })
            .collect()
    }

    /// `sessions` (Ctrl+Shift+S): the list of saved sessions.
    pub(super) fn open_sessions_popup(&mut self) {
        let rows = self.session_rows();
        if rows.is_empty() {
            return self.notify(
                None,
                "No saved sessions",
                "A session is saved when fterm closes, or with \"Save session as…\".",
                Level::Info,
                Source::App,
            );
        }
        self.palette = None;
        self.history_popup = Some(HistoryPopup::new(
            PopupKind::Sessions,
            rows,
            String::new(),
            None,
        ));
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// `save_session_as`: a box that asks for the name.
    pub(super) fn start_name_prompt(&mut self) {
        self.name_prompt = Some(InputBox::default());
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    pub(super) fn name_prompt_lines(&self) -> Option<Vec<String>> {
        let input = self.name_prompt.as_ref()?;
        Some(vec![
            "Save this session as:".to_owned(),
            format!("{}▏", input.text),
            String::new(),
            "Enter = save, Esc = cancel. A saved session stays in the list (Ctrl+Shift+S)."
                .to_owned(),
        ])
    }

    pub(super) fn name_prompt_key(&mut self, event: &KeyEvent) {
        let ctrl = self.mods.control_key();
        let Some(input) = &mut self.name_prompt else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.name_prompt = None,
            Key::Named(NamedKey::Enter) => {
                let name = input.take();
                self.name_prompt = None;
                if !name.trim().is_empty() {
                    self.save_named_session(name.trim());
                }
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

    fn save_named_session(&mut self, name: &str) {
        let (Some(dir), Some(session)) = (sessions_dir(), self.capture_session()) else {
            return;
        };
        match save_named(&dir, name, &session) {
            Ok(_) => self.notify(None, "Session saved", name, Level::Success, Source::App),
            Err(err) => self.notify(
                None,
                "Cannot save the session",
                &err.to_string(),
                Level::Error,
                Source::App,
            ),
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
        let (programs, agents) = (
            self.config.config.restore_programs,
            self.config.config.restore_agents,
        );
        let mut reruns = 0;
        let ago = match crate::panels::short_ago(Duration::from_millis(
            now_ms().saturating_sub(saved.saved),
        )) {
            now if now == "now" => "just now".to_owned(),
            age => format!("{age} ago"),
        };
        for tab in &saved.tabs {
            let layout = restore_layout(&tab.layout, &mut |pane| {
                self.spawn_cwd = pane.cwd.clone();
                self.spawn_intro = intro_bytes(&pane.text, &ago);
                match self.spawn_pane(size, pane.profile.as_deref()) {
                    Ok(id) => {
                        let rerun = pane
                            .ran
                            .as_deref()
                            .and_then(|ran| rerun_command(ran, programs, agents));
                        if let Some(p) = self.running.as_mut().and_then(|r| r.panes.get_mut(&id)) {
                            p.intro_lines = pane.text.len();
                            if rerun.is_some() {
                                p.rerun = rerun;
                                reruns += 1;
                            }
                        }
                        Some(id)
                    }
                    Err(err) => {
                        tracing::warn!("cannot restore a pane: {err:#}");
                        None
                    }
                }
            });
            self.spawn_cwd = None;
            self.spawn_intro.clear();
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
        tracing::info!(tabs = opened, reruns, "session restored");
        if reruns > 0 && programs != Rerun::Run && agents != Rerun::Run {
            let body = if reruns == 1 {
                "A pane ran a program: its command is in the prompt again. Press Enter to run it."
                    .to_owned()
            } else {
                format!(
                    "{reruns} panes ran programs: their commands are in the prompt again. Press Enter to run them."
                )
            };
            self.notify(
                None,
                "Programs can run again",
                &body,
                Level::Info,
                Source::App,
            );
        }
        self.resize_all_panes();
        self.tab_changed();
    }
}
