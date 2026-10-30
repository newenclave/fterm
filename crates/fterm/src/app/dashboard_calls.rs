//! The dashboard (Ctrl+Shift+D): tiles of panes, of commands, and of buttons over the window, in a scene pane that
//! fterm draws itself (see `dashboard.rs`). Keys and clicks on it go to the tiles, not to the scene.

use std::collections::{HashMap, HashSet};

use fterm_config::load::{DashTile, Press};
use winit::keyboard::{Key, NamedKey};

use super::*;
use crate::dashboard::{self, Activity, Colors, PaneFacts, Places, Tile, TileState};
use crate::runner::RunResult;

/// How often a shown dashboard is drawn again.
const REFRESH: Duration = Duration::from_secs(1);
/// The longest a command of a tile or a button may run.
const TIMEOUT: Duration = Duration::from_secs(60);
/// How long a press of a button with `confirm` waits for the second press.
const CONFIRM: Duration = Duration::from_secs(4);
/// The most lines of a pane a tile keeps.
const PANE_LINES: usize = 20;

/// What a drawn tile stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Pane(PaneId),
    /// A command tile (its command).
    Command(String),
    /// A button tile (its index in `dashboard.tiles`).
    Buttons(usize),
    /// A tile from the API (its id).
    Api(String),
}

/// A result from a thread of the dashboard.
#[derive(Debug)]
pub enum DashDone {
    /// A command tile ran (its command).
    Command(String, RunResult),
    /// A button with `run` ran (its tile, its label).
    Button(usize, String, RunResult),
}

/// The state of a command tile.
#[derive(Debug, Default)]
struct CommandState {
    result: Option<RunResult>,
    /// When it started the last time.
    started: Option<Instant>,
    running: bool,
}

/// All that the dashboard keeps between frames.
#[derive(Debug, Default)]
pub(super) struct DashState {
    entries: Vec<Entry>,
    /// The tiles as they were drawn last (button keys count on them).
    tiles: Vec<Tile>,
    places: Places,
    /// Columns of the grid that was drawn (for the up and down arrows).
    columns: usize,
    selected: usize,
    /// The size of the scene when it was drawn.
    size: Option<GridSize>,
    next_draw: Option<Instant>,
    /// Output of each pane in the last minutes (the charts).
    pub(super) activity: HashMap<PaneId, Activity>,
    commands: HashMap<String, CommandState>,
    /// The last result of a button tile, as a line (by the index of the tile).
    button_results: HashMap<usize, String>,
    /// A button with `confirm` that was pressed once: (tile, button, until when).
    confirm: Option<(usize, usize, Instant)>,
    /// Tiles from the API (`dashboard_tile`), in the order they came.
    pub(super) api_tiles: Vec<(String, Tile)>,
}

/// `4m`, `12s`, `2h`.
fn age(since: Instant, now: Instant) -> String {
    let s = now.saturating_duration_since(since).as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
    }
}

/// The status and the state of a command tile.
fn command_status(state: Option<&CommandState>, now: Instant) -> (String, TileState, Vec<String>) {
    let Some(state) = state else {
        return (String::new(), TileState::Idle, Vec::new());
    };
    if state.running && state.result.is_none() {
        return (tr!("dashboard.running"), TileState::Working, Vec::new());
    }
    let Some(result) = &state.result else {
        return (String::new(), TileState::Idle, Vec::new());
    };
    let when = state.started.map(|t| age(t, now)).unwrap_or_default();
    let mut lines = result.lines();
    if let Some(error) = &result.error {
        lines.push(error.clone());
    }
    let status = match result.exit {
        Some(code) => format!("exit {code} · {when}"),
        None => when,
    };
    let tile_state = if result.ok() {
        TileState::Done
    } else {
        TileState::Error
    };
    (status, tile_state, lines)
}

fn scene_rgb(c: fterm_term::alacritty_terminal::vte::ansi::Rgb) -> fterm_scene::Rgb {
    fterm_scene::Rgb::new(c.r, c.g, c.b)
}

impl App {
    /// The dashboard pane, when it is shown and has the keys.
    pub(super) fn dashboard_shown(&self) -> Option<PaneId> {
        let running = self.running.as_ref()?;
        match running.mux.dashboard() {
            Some((pane, true)) if running.mux.input_pane() == Some(pane) => Some(pane),
            _ => None,
        }
    }

    /// `toggle_dashboard`: shows it (made the first time) or hides it.
    pub(super) fn toggle_dashboard(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        match running.mux.toggle_dashboard() {
            Some(_) => {}
            None => {
                let grid = running.grid_for(running.tab_area());
                let id = self.spawn_scene(grid);
                let running = self.running.as_mut().expect("the window is open");
                if let Some(p) = running.panes.get_mut(&id) {
                    p.app_title = Some("dashboard".to_owned());
                }
                running.mux.set_dashboard(id);
                tracing::info!(pane = id.0, "the dashboard opened");
            }
        }
        if let Some(running) = &self.running {
            running.resize_all_panes();
            running.window.request_redraw();
        }
        if self.dashboard_shown().is_some() {
            self.dash.selected = 0;
            self.dash.next_draw = None;
            self.dashboard_tick(Instant::now());
        }
        self.update_ime();
    }

    fn hide_dashboard(&mut self) {
        if self.dashboard_shown().is_some() {
            self.toggle_dashboard();
        }
    }

    /// The shell that runs the commands of the dashboard: the program of the default profile (not a WSL one), else
    /// the system shell.
    fn background_shell(&self) -> String {
        self.profile(None)
            .filter(|p| p.wsl.is_none() && !p.command.trim().is_empty())
            .map(|p| p.command)
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "powershell.exe".to_owned()
                } else {
                    std::env::var("SHELL").unwrap_or_else(|_| "sh".to_owned())
                }
            })
    }

    /// Runs `command` on a thread; the result comes back as `UserEvent::Dashboard`.
    fn run_in_background(
        &self,
        command: &str,
        done: impl FnOnce(RunResult) -> DashDone + Send + 'static,
    ) {
        let shell = self.background_shell();
        let args = overlay_calls::shell_run_args(&shell, command);
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result = crate::runner::run(&shell, &args, TIMEOUT);
            let _ = proxy.send_event(UserEvent::Dashboard(done(result)));
        });
    }

    fn start_command(&mut self, command: &str, now: Instant) {
        let state = self.dash.commands.entry(command.to_owned()).or_default();
        if state.running {
            return;
        }
        state.running = true;
        state.started = Some(now);
        let owned = command.to_owned();
        self.run_in_background(command, move |r| DashDone::Command(owned, r));
    }

    /// The loop waits: start the commands that are due and draw again when it is time. Gives when to wake up.
    pub(super) fn dashboard_tick(&mut self, now: Instant) -> Option<Instant> {
        let pane = self.dashboard_shown()?;
        let mut wake: Option<Instant> = None;
        let mut soon = |t: Instant| wake = Some(wake.map_or(t, |w| w.min(t)));
        let due: Vec<(String, u64)> = self
            .config
            .config
            .dashboard
            .tiles
            .iter()
            .filter_map(|t| match t {
                DashTile::Command { command, every, .. } => Some((command.clone(), *every)),
                _ => None,
            })
            .collect();
        for (command, every) in due {
            let next = self
                .dash
                .commands
                .get(&command)
                .and_then(|s| s.started)
                .map(|t| t + Duration::from_secs(every));
            match next {
                Some(at) if at > now => soon(at),
                _ => self.start_command(&command, now),
            }
        }
        let size = self.running.as_ref()?.panes.get(&pane)?.session.grid_size();
        if self.dash.next_draw.is_none_or(|t| now >= t) || self.dash.size != Some(size) {
            self.draw_dashboard(now);
        }
        if let Some(t) = self.dash.next_draw {
            soon(t);
        }
        wake
    }

    /// The tiles in the order of the config, and what each stands for.
    fn dashboard_tiles(&self, now: Instant) -> (Vec<Entry>, Vec<Tile>) {
        let mut entries = Vec::new();
        let mut tiles = Vec::new();
        let Some(running) = &self.running else {
            return (entries, tiles);
        };
        let several = running.mux.workspaces().len() > 1;
        let panes = self.listed_panes();
        let mut taken = HashSet::new();
        for (index, config_tile) in self.config.config.dashboard.tiles.iter().enumerate() {
            match config_tile {
                DashTile::Panes(filter) => {
                    for &id in &panes {
                        let Some(p) = running.panes.get(&id) else {
                            continue;
                        };
                        if p.scene.is_some() || p.picture_pane.is_some() || taken.contains(&id) {
                            continue;
                        }
                        let program = display_name(p.session.program());
                        let title =
                            fterm_mux::mux::tab_title(None, p.app_title.as_deref(), program);
                        let (workspace, tab) = running.mux.pane_place(id).unwrap_or(("", 0));
                        let failed = p
                            .last_command
                            .as_ref()
                            .is_some_and(|c| c.exit.is_some_and(|e| e != 0));
                        let facts = PaneFacts {
                            agent: p.agent.is_some(),
                            running: p.shell.is_running(),
                            failed,
                            program,
                            title,
                            workspace,
                            profile: p.profile.as_deref().unwrap_or(""),
                        };
                        if !dashboard::pane_matches(filter, &facts) {
                            continue;
                        }
                        taken.insert(id);
                        entries.push(Entry::Pane(id));
                        tiles.push(self.pane_tile(id, tab + 1, several.then_some(workspace), now));
                    }
                }
                DashTile::Command { title, command, .. } => {
                    let (status, state, lines) =
                        command_status(self.dash.commands.get(command), now);
                    entries.push(Entry::Command(command.clone()));
                    tiles.push(Tile {
                        title: title.clone(),
                        status,
                        state,
                        lines,
                        ..Default::default()
                    });
                }
                DashTile::Buttons { title, buttons } => {
                    let status = match self.dash.confirm {
                        Some((t, b, until)) if t == index && now < until => {
                            let n = self.button_number(index, b);
                            tr!(
                                "dashboard.confirm",
                                key = n.and_then(dashboard::button_key).unwrap_or(' ').to_string()
                            )
                        }
                        _ => String::new(),
                    };
                    entries.push(Entry::Buttons(index));
                    tiles.push(Tile {
                        title: title.clone(),
                        status,
                        lines: self
                            .dash
                            .button_results
                            .get(&index)
                            .cloned()
                            .into_iter()
                            .collect(),
                        buttons: buttons.iter().map(|b| b.label.clone()).collect(),
                        ..Default::default()
                    });
                }
            }
        }
        for (id, tile) in &self.dash.api_tiles {
            entries.push(Entry::Api(id.clone()));
            tiles.push(tile.clone());
        }
        (entries, tiles)
    }

    /// The number of a button of the dashboard, across the tiles before it (its key is `button_key` of it).
    fn button_number(&self, tile: usize, button: usize) -> Option<usize> {
        let before: usize = self
            .config
            .config
            .dashboard
            .tiles
            .iter()
            .take(tile)
            .map(|t| match t {
                DashTile::Buttons { buttons, .. } => buttons.len(),
                _ => 0,
            })
            .sum();
        Some(before + button)
    }

    /// The tile of a pane: its tab and title, the agent state or the last command, the last lines, the activity.
    fn pane_tile(&self, id: PaneId, tab: usize, workspace: Option<&str>, now: Instant) -> Tile {
        let running = self.running.as_ref().expect("the window is open");
        let p = &running.panes[&id];
        let program = display_name(p.session.program());
        let title = fterm_mux::mux::tab_title(None, p.app_title.as_deref(), program);
        let place = match workspace {
            Some(ws) => format!("{ws} {tab}"),
            None => tab.to_string(),
        };
        let title = tr!("dashboard.pane_title", title = title, tab = place);
        let (status, state) = if let Some(agent) = &p.agent {
            (
                format!("{} {}", agent.kind.name(), age(agent.since, now)),
                TileState::from_name(agent.kind.name()),
            )
        } else if p.shell.is_running() {
            (tr!("dashboard.running"), TileState::Working)
        } else {
            match p.last_command.as_ref().and_then(|c| c.exit) {
                Some(0) => ("exit 0".to_owned(), TileState::Idle),
                Some(code) => (format!("exit {code}"), TileState::Error),
                None => (String::new(), TileState::Idle),
            }
        };
        let mut lines: Vec<String> = p.session.screen_text().lines().map(str::to_owned).collect();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        let start = lines.len().saturating_sub(PANE_LINES);
        Tile {
            title,
            status,
            state,
            lines: lines.split_off(start),
            values: self
                .dash
                .activity
                .get(&id)
                .map(|a| a.values(now))
                .unwrap_or_default(),
            buttons: Vec::new(),
        }
    }

    fn draw_dashboard(&mut self, now: Instant) {
        let Some(pane) = self.dashboard_shown() else {
            return;
        };
        let (entries, tiles) = self.dashboard_tiles(now);
        let ui = self.ui;
        let colors = Colors {
            text: scene_rgb(ui.text),
            dim: scene_rgb(ui.text_dim),
            states: [
                scene_rgb(ui.text_ghost),
                scene_rgb(ui.agent_working),
                scene_rgb(ui.agent_waiting),
                scene_rgb(ui.agent_done),
                scene_rgb(ui.agent_error),
            ],
        };
        let Some(running) = &self.running else {
            return;
        };
        let Some(p) = running.panes.get(&pane) else {
            return;
        };
        let size = p.session.grid_size();
        let (cols, rows) = (size.columns, size.rows);
        self.dash.selected = self.dash.selected.min(tiles.len().saturating_sub(1));
        let header = if tiles.is_empty() {
            tr!("dashboard.empty")
        } else {
            tr!("dashboard.header")
        };
        let columns = self.config.config.dashboard.columns;
        let (ops, places) = dashboard::draw(
            &tiles,
            cols,
            rows,
            columns,
            Some(self.dash.selected),
            &header,
            &colors,
        );
        if let Some(scene) = &p.scene {
            let mut scene = scene.lock().unwrap();
            if let Err(err) = scene.draw(&ops) {
                tracing::warn!("the dashboard: {err}");
            }
            p.session.feed(&fterm_scene::render(scene.canvas()));
        }
        running.window.request_redraw();
        self.dash.columns = dashboard::grid(tiles.len(), cols, rows.saturating_sub(1), columns).0;
        self.dash.entries = entries;
        self.dash.tiles = tiles;
        self.dash.places = places;
        self.dash.size = Some(size);
        self.dash.next_draw = Some(now + REFRESH);
    }

    /// Draw again at once (after a key, a result).
    fn redraw_dashboard(&mut self) {
        self.dash.next_draw = None;
        self.dashboard_tick(Instant::now());
    }

    /// A key while the dashboard is shown.
    pub(super) fn dashboard_key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        let n = self.dash.tiles.len();
        let cols = self.dash.columns.max(1);
        let sel = self.dash.selected;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => return self.hide_dashboard(),
            Key::Named(NamedKey::Enter) => return self.open_tile(event_loop, sel),
            Key::Named(NamedKey::ArrowLeft) => self.dash.selected = sel.saturating_sub(1),
            Key::Named(NamedKey::ArrowRight) => {
                self.dash.selected = (sel + 1).min(n.saturating_sub(1))
            }
            Key::Named(NamedKey::ArrowUp) => self.dash.selected = sel.saturating_sub(cols),
            Key::Named(NamedKey::ArrowDown) => {
                if sel + cols < n {
                    self.dash.selected = sel + cols;
                }
            }
            Key::Named(NamedKey::F5) => {
                // Run all command tiles again now.
                for s in self.dash.commands.values_mut() {
                    s.started = None;
                }
            }
            _ => {
                let Some(c) = event.text.as_deref().and_then(|t| t.chars().next()) else {
                    return;
                };
                if let Some(d) = c.to_digit(10).filter(|d| *d >= 1) {
                    return self.open_tile(event_loop, d as usize - 1);
                }
                if let Some((tile, button)) =
                    dashboard::button_of_key(&self.dash.tiles, c.to_ascii_lowercase())
                    && let Some(Entry::Buttons(index)) = self.dash.entries.get(tile).cloned()
                {
                    return self.press_button(event_loop, index, button);
                }
                return;
            }
        }
        self.redraw_dashboard();
    }

    /// A click on the dashboard (`true` = it was there).
    pub(super) fn dashboard_click(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let Some(pane) = self.dashboard_shown() else {
            return false;
        };
        if self.pane_under_mouse() != Some(pane) {
            return false;
        }
        let Some(running) = &self.running else {
            return false;
        };
        let (point, _) = self.mouse_cell(running);
        let (col, row) = (point.column.0, point.line.0.max(0) as usize);
        let button = self
            .dash
            .places
            .buttons
            .iter()
            .find(|(_, _, r)| r.contains(col, row))
            .map(|(t, b, _)| (*t, *b));
        if let Some((tile, b)) = button {
            if let Some(Entry::Buttons(index)) = self.dash.entries.get(tile).cloned() {
                self.press_button(event_loop, index, b);
            }
            return true;
        }
        if let Some(tile) = self
            .dash
            .places
            .tiles
            .iter()
            .position(|r| r.contains(col, row))
        {
            if self.dash.selected == tile {
                self.open_tile(event_loop, tile);
            } else {
                self.dash.selected = tile;
                self.redraw_dashboard();
            }
        }
        true
    }

    /// Enter (or the key) on a tile: a pane goes there, a command runs again, a button tile presses its first one.
    fn open_tile(&mut self, event_loop: &ActiveEventLoop, tile: usize) {
        let Some(entry) = self.dash.entries.get(tile).cloned() else {
            return;
        };
        self.dash.selected = tile;
        match entry {
            Entry::Pane(pane) => {
                self.hide_dashboard();
                self.go_to_pane(pane);
            }
            Entry::Command(command) => {
                if let Some(s) = self.dash.commands.get_mut(&command) {
                    s.started = None;
                }
                self.redraw_dashboard();
            }
            Entry::Buttons(index) => self.press_button(event_loop, index, 0),
            Entry::Api(_) => self.redraw_dashboard(),
        }
    }

    /// A button: with `confirm`, the first press asks for a second one.
    fn press_button(&mut self, event_loop: &ActiveEventLoop, tile: usize, button: usize) {
        let Some(DashTile::Buttons { buttons, .. }) = self.config.config.dashboard.tiles.get(tile)
        else {
            return;
        };
        let Some(b) = buttons.get(button).cloned() else {
            return;
        };
        let now = Instant::now();
        if b.confirm
            && !matches!(self.dash.confirm, Some((t, n, until)) if t == tile && n == button && now < until)
        {
            self.dash.confirm = Some((tile, button, now + CONFIRM));
            self.dash.next_draw = None;
            self.redraw_dashboard();
            return;
        }
        self.dash.confirm = None;
        match b.press {
            Press::Run {
                command,
                overlay: true,
            } => {
                self.hide_dashboard();
                if let Err(err) = self.open_overlay(Some(command), None, None) {
                    self.notify(
                        None,
                        &tr!("toast.cannot_start"),
                        &format!("{err:#}"),
                        Level::Error,
                        Source::App,
                    );
                }
            }
            Press::Run {
                command,
                overlay: false,
            } => {
                self.dash.button_results.insert(
                    tile,
                    tr!("dashboard.button_running", label = b.label.clone()),
                );
                let label = b.label.clone();
                self.run_in_background(&command, move |r| DashDone::Button(tile, label, r));
                self.redraw_dashboard();
            }
            Press::Action(action) => {
                self.hide_dashboard();
                self.run_action(event_loop, action);
            }
        }
    }

    /// A result from a thread of the dashboard.
    pub(super) fn dashboard_done(&mut self, done: DashDone) {
        match done {
            DashDone::Command(command, result) => {
                let state = self.dash.commands.entry(command).or_default();
                state.running = false;
                state.result = Some(result);
            }
            DashDone::Button(tile, label, result) => {
                let last = result
                    .error
                    .clone()
                    .or_else(|| result.lines().last().cloned())
                    .unwrap_or_default();
                let code = result
                    .exit
                    .map_or_else(|| "-".to_owned(), |c| c.to_string());
                self.dash
                    .button_results
                    .insert(tile, format!("{label}: exit {code} · {last}"));
                let (title, level) = if result.ok() {
                    (tr!("toast.button_done", label = label), Level::Success)
                } else {
                    (tr!("toast.button_failed", label = label), Level::Error)
                };
                self.notify(None, &title, &last, level, Source::App);
            }
        }
        if self.dashboard_shown().is_some() {
            self.redraw_dashboard();
        }
    }
}
