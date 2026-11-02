//! The dashboard (Ctrl+Shift+D): tiles of panes, of commands, and of buttons over the window, in a scene pane that
//! fterm draws itself (see `dashboard.rs`). Keys and clicks on it go to the tiles, not to the scene.

use std::collections::{HashMap, HashSet};

use fterm_api::RpcError;
use fterm_config::keys::Action;
use fterm_config::load::{DashTile, ItemAction, ItemDo, ItemKey, Press};
use serde_json::{Value, json};
use winit::keyboard::{Key, NamedKey};

use super::*;
use crate::dashboard::{self, Activity, Colors, Item, PaneFacts, Places, Tile, TileState};
use crate::runner::RunResult;

/// How often a shown dashboard is drawn again.
const REFRESH: Duration = Duration::from_secs(1);
/// The longest a command of a tile or a button may run.
const TIMEOUT: Duration = Duration::from_secs(60);
/// How long a press of a button with `confirm` waits for the second press.
const CONFIRM: Duration = Duration::from_secs(4);
/// The most lines of a pane a tile keeps.
const PANE_LINES: usize = 20;
/// The most tiles from the API, and the most lines and values in one of them.
pub(super) const MAX_API_TILES: usize = 32;
const MAX_API_LINES: usize = 50;
const MAX_API_VALUES: usize = 500;

/// A tile from the params of `dashboard_tile`: `None` = remove it. Only text and numbers: no commands or buttons.
pub(super) fn api_tile(params: &Value) -> Result<Option<(String, Tile)>, RpcError> {
    let id = params
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| RpcError::invalid_params("`id` is missing"))?
        .to_owned();
    if params.get("remove").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let text = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let lines: Vec<String> = params
        .get("lines")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let values: Vec<f64> = params
        .get("values")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    let tile = Tile {
        title: if params.get("title").is_some() {
            text("title")
        } else {
            id.clone()
        },
        status: text("status"),
        state: TileState::from_name(&text("state")),
        lines: lines[lines.len().saturating_sub(MAX_API_LINES)..].to_vec(),
        values: values[values.len().saturating_sub(MAX_API_VALUES)..].to_vec(),
        ..Default::default()
    };
    Ok(Some((id, tile)))
}

/// Puts (or with `None` removes) the API tile `id`: the same id changes it in place; at most `MAX_API_TILES` (the
/// oldest goes).
pub(super) fn put_api_tile(tiles: &mut Vec<(String, Tile)>, id: String, tile: Option<Tile>) {
    let at = tiles.iter().position(|(i, _)| *i == id);
    match (at, tile) {
        (Some(i), Some(tile)) => tiles[i].1 = tile,
        (Some(i), None) => {
            tiles.remove(i);
        }
        (None, Some(tile)) => {
            if tiles.len() >= MAX_API_TILES {
                tiles.remove(0);
            }
            tiles.push((id, tile));
        }
        (None, None) => {}
    }
}

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
    /// Programs of an item action or of `fterm.run` ran (the command of their list tile, if any; the label).
    Steps(Option<String>, String, RunResult),
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
    /// The list tile that has the keys (its command), and the chosen item in it.
    inside: Option<String>,
    cursor: usize,
    /// An item action with `confirm` that was pressed once: (the command of the tile, the action, until when).
    item_confirm: Option<(String, usize, Instant)>,
    /// The list tile whose Lua action runs now (its `fterm.run` makes the list run again).
    origin: Option<String>,
}

/// The items of a command tile: its output lines.
fn items_of(state: Option<&CommandState>) -> Vec<Item> {
    state
        .and_then(|s| s.result.as_ref())
        .filter(|r| r.error.is_none())
        .map(|r| {
            r.lines()
                .iter()
                .filter(|l| !l.trim().is_empty())
                .map(|l| Item::parse(l))
                .collect()
        })
        .unwrap_or_default()
}

/// The keys shown under a list: its actions, and Enter = open the link when no action takes Enter.
fn item_keys(actions: &[ItemAction]) -> Vec<(String, String)> {
    let mut keys: Vec<(String, String)> = actions
        .iter()
        .map(|a| {
            let key = match a.key {
                ItemKey::Enter => "enter".to_owned(),
                ItemKey::Char(c) => c.to_string(),
            };
            (key, a.label.clone())
        })
        .collect();
    if !actions.iter().any(|a| a.key == ItemKey::Enter) {
        keys.insert(0, ("enter".to_owned(), tr!("dashboard.open_link")));
    }
    keys.push(("esc".to_owned(), tr!("dashboard.back")));
    keys
}

/// The path of `ftermctl` next to fterm (it runs the steps of an overlay).
fn ftermctl_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "ftermctl.exe"
    } else {
        "ftermctl"
    };
    let path = exe.with_file_name(name);
    path.is_file().then_some(path)
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
        let argv = self
            .config
            .config
            .dashboard
            .tiles
            .iter()
            .find_map(|t| match t {
                DashTile::Command {
                    command: c,
                    argv: Some(argv),
                    ..
                } if c == command => Some(argv.clone()),
                _ => None,
            });
        match argv {
            // A list: the program itself, no shell.
            Some(argv) => {
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let result = crate::runner::run(&argv[0], &argv[1..], TIMEOUT);
                    let _ =
                        proxy.send_event(UserEvent::Dashboard(DashDone::Command(owned, result)));
                });
            }
            None => self.run_in_background(command, move |r| DashDone::Command(owned, r)),
        }
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
                DashTile::Command {
                    title,
                    command,
                    show,
                    actions,
                    waiting,
                    ..
                } => {
                    let state_of = self.dash.commands.get(command);
                    let (mut status, mut state, mut lines) = command_status(state_of, now);
                    let items = items_of(state_of);
                    let list = !actions.is_empty() || *waiting || show.is_some();
                    let mut title = title.clone();
                    if list
                        && state_of
                            .and_then(|s| s.result.as_ref())
                            .is_some_and(|r| r.error.is_none())
                    {
                        lines = items.iter().map(|i| i.shown(*show).to_owned()).collect();
                        title = format!("{title} ({})", items.len());
                        if let Some(age) = status.split(" · ").nth(1) {
                            status = age.to_owned();
                        }
                        if *waiting {
                            state = if items.is_empty() {
                                TileState::Done
                            } else {
                                TileState::Waiting
                            };
                        }
                    }
                    let inside = self.dash.inside.as_deref() == Some(command.as_str());
                    if inside
                        && let Some((c, a, until)) = &self.dash.item_confirm
                        && c == command
                        && now < *until
                        && let Some(action) = actions.get(*a)
                    {
                        let key = match action.key {
                            ItemKey::Enter => "enter".to_owned(),
                            ItemKey::Char(c) => c.to_string(),
                        };
                        status = tr!("dashboard.confirm", key = key);
                    }
                    entries.push(Entry::Command(command.clone()));
                    tiles.push(Tile {
                        title,
                        status,
                        state,
                        lines,
                        from_top: list,
                        cursor: inside.then(|| self.dash.cursor.min(items.len().saturating_sub(1))),
                        keys: if inside {
                            item_keys(actions)
                        } else {
                            Vec::new()
                        },
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
            ..Default::default()
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
        if let Some(command) = self.dash.inside.clone() {
            return self.list_key(event_loop, event, &command);
        }
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
        let key = self
            .dash
            .places
            .keys
            .iter()
            .find(|(_, _, r)| r.contains(col, row))
            .map(|(t, k, _)| (*t, *k));
        if let Some((tile, k)) = key
            && let Some(Entry::Command(command)) = self.dash.entries.get(tile).cloned()
            && let Some((name, _)) = self
                .dash
                .tiles
                .get(tile)
                .and_then(|t| t.keys.get(k))
                .cloned()
        {
            match name.as_str() {
                "esc" => {
                    self.dash.inside = None;
                    self.redraw_dashboard();
                }
                "enter" => self.item_key(event_loop, &command, ItemKey::Enter),
                other => {
                    if let Some(c) = other.chars().next() {
                        self.item_key(event_loop, &command, ItemKey::Char(c));
                    }
                }
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
                // Into the list: arrows choose an item, its keys act on it.
                self.dash.inside = Some(command);
                self.dash.cursor = 0;
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

    /// The actions of the list tile of `command` (from the config).
    fn list_actions(&self, command: &str) -> Option<(Option<usize>, Vec<ItemAction>)> {
        self.config
            .config
            .dashboard
            .tiles
            .iter()
            .find_map(|t| match t {
                DashTile::Command {
                    command: c,
                    show,
                    actions,
                    ..
                } if c == command => Some((*show, actions.clone())),
                _ => None,
            })
    }

    /// A key inside a list tile.
    fn list_key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent, command: &str) {
        let n = items_of(self.dash.commands.get(command)).len();
        let last = n.saturating_sub(1);
        let page = 10;
        let c = &mut self.dash.cursor;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.dash.inside = None;
                self.dash.item_confirm = None;
            }
            Key::Named(NamedKey::ArrowUp) => *c = c.saturating_sub(1),
            Key::Named(NamedKey::ArrowDown) => *c = (*c + 1).min(last),
            Key::Named(NamedKey::PageUp) => *c = c.saturating_sub(page),
            Key::Named(NamedKey::PageDown) => *c = (*c + page).min(last),
            Key::Named(NamedKey::Home) => *c = 0,
            Key::Named(NamedKey::End) => *c = last,
            Key::Named(NamedKey::F5) => {
                if let Some(s) = self.dash.commands.get_mut(command) {
                    s.started = None;
                }
            }
            Key::Named(NamedKey::Enter) => {
                return self.item_key(event_loop, command, ItemKey::Enter);
            }
            _ => {
                if let Some(ch) = event.text.as_deref().and_then(|t| t.chars().next())
                    && ch.is_ascii_alphanumeric()
                {
                    return self.item_key(
                        event_loop,
                        command,
                        ItemKey::Char(ch.to_ascii_lowercase()),
                    );
                }
                return;
            }
        }
        self.redraw_dashboard();
    }

    /// The key of an item action on the chosen item of the list of `command`.
    fn item_key(&mut self, event_loop: &ActiveEventLoop, command: &str, key: ItemKey) {
        let items = items_of(self.dash.commands.get(command));
        let Some(item) = items
            .get(self.dash.cursor.min(items.len().saturating_sub(1)))
            .cloned()
        else {
            return;
        };
        let Some((_, actions)) = self.list_actions(command) else {
            return;
        };
        let Some(index) = actions.iter().position(|a| a.key == key) else {
            // No action on Enter: open the first link of the line.
            if key == ItemKey::Enter
                && let Some(url) = dashboard::first_url(&item.line)
            {
                let url = url.to_owned();
                self.open_checked(&url);
            }
            return;
        };
        let action = actions[index].clone();
        let now = Instant::now();
        if action.confirm
            && !matches!(&self.dash.item_confirm, Some((c, a, until)) if c == command && *a == index && now < *until)
        {
            self.dash.item_confirm = Some((command.to_owned(), index, now + CONFIRM));
            return self.redraw_dashboard();
        }
        self.dash.item_confirm = None;
        match &action.does {
            ItemDo::Open(template) => {
                let url = dashboard::fill(template, &item);
                self.open_checked(&url);
            }
            ItemDo::Run { steps, overlay } => {
                let steps: Vec<Vec<String>> = steps
                    .iter()
                    .map(|s| dashboard::fill_args(s, &item))
                    .collect();
                self.run_steps(
                    steps,
                    *overlay,
                    Some(command.to_owned()),
                    action.label.clone(),
                );
            }
            ItemDo::Action(Action::Lua(i)) => {
                match self.config.call_with_item(*i, &item.line, &item.fields) {
                    Ok(calls) => {
                        self.dash.origin = Some(command.to_owned());
                        for call in calls {
                            self.run_api_call(event_loop, call);
                        }
                        self.dash.origin = None;
                    }
                    Err(err) => self.notify(
                        None,
                        &tr!("toast.lua_error"),
                        &err,
                        Level::Error,
                        Source::App,
                    ),
                }
            }
            ItemDo::Action(other) => {
                self.hide_dashboard();
                self.run_action(event_loop, other.clone());
            }
        }
        if self.dashboard_shown().is_some() {
            self.redraw_dashboard();
        }
    }

    /// Opens a link from a list: http(s) at once, other kinds after a question (the same as a click on a link).
    fn open_checked(&mut self, url: &str) {
        self.open_link(fterm_term::links::Link {
            start: Default::default(),
            end: Default::default(),
            url: url.to_owned(),
            text: url.to_owned(),
            osc8: false,
        });
    }

    /// `fterm.run` (from a Lua function): programs one after another.
    pub(super) fn lua_run(&mut self, steps: Vec<Vec<String>>, overlay: bool) {
        let label = steps
            .first()
            .and_then(|s| s.first())
            .cloned()
            .unwrap_or_default();
        let origin = self.dash.origin.clone();
        self.run_steps(steps, overlay, origin, label);
    }

    /// Programs with their arguments, one after another and with no shell: in the background (a toast at the end),
    /// or in an overlay that stays until Enter (through `ftermctl steps`).
    pub(super) fn run_steps(
        &mut self,
        steps: Vec<Vec<String>>,
        overlay: bool,
        origin: Option<String>,
        label: String,
    ) {
        if steps.iter().any(|s| s.is_empty()) || steps.is_empty() {
            return;
        }
        if overlay {
            let (program, args) = match ftermctl_path() {
                Some(ctl) => (
                    ctl.display().to_string(),
                    vec![
                        "steps".to_owned(),
                        serde_json::to_string(&steps).unwrap_or_default(),
                    ],
                ),
                // No ftermctl next to fterm: the first program alone (the overlay goes when it ends).
                None => (steps[0][0].clone(), steps[0][1..].to_vec()),
            };
            self.hide_dashboard();
            self.spawn_argv = Some((program, args));
            let opened = self.open_overlay(None, None, None);
            self.spawn_argv = None;
            if let Err(err) = opened {
                self.notify(
                    None,
                    &tr!("toast.cannot_start"),
                    &format!("{err:#}"),
                    Level::Error,
                    Source::App,
                );
            }
            return;
        }
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let mut result = RunResult::default();
            for step in &steps {
                result = crate::runner::run(&step[0], &step[1..], TIMEOUT);
                if !result.ok() {
                    break;
                }
            }
            let _ = proxy.send_event(UserEvent::Dashboard(DashDone::Steps(origin, label, result)));
        });
    }

    /// `dashboard`: shows or hides it (`show`; no `show` = the other state).
    pub(super) fn api_dashboard(&mut self, params: &Value) -> Result<Value, RpcError> {
        let want = params.get("show").and_then(Value::as_bool);
        if want != Some(self.dashboard_shown().is_some()) {
            self.toggle_dashboard();
        }
        Ok(json!({ "shown": self.dashboard_shown().is_some() }))
    }

    /// `dashboard_tile`: a tile of a script or an agent (`id`, `title`, `status`, `state`, `lines`, `values`,
    /// `remove`).
    pub(super) fn api_dashboard_tile(&mut self, params: &Value) -> Result<Value, RpcError> {
        let (id, tile) = match api_tile(params)? {
            Some((id, tile)) => (id, Some(tile)),
            None => (
                params["id"].as_str().unwrap_or_default().trim().to_owned(),
                None,
            ),
        };
        put_api_tile(&mut self.dash.api_tiles, id, tile);
        if self.dashboard_shown().is_some() {
            self.redraw_dashboard();
        }
        Ok(json!({ "tiles": self.dash.api_tiles.len() }))
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
            DashDone::Steps(origin, label, result) => {
                let last = result
                    .error
                    .clone()
                    .or_else(|| result.lines().last().cloned())
                    .unwrap_or_default();
                let (title, level) = if result.ok() {
                    (tr!("toast.button_done", label = label), Level::Success)
                } else {
                    (tr!("toast.button_failed", label = label), Level::Error)
                };
                self.notify(None, &title, &last, level, Source::App);
                // The list changed (an approved pull request goes away): run its command again now.
                if let Some(command) = origin
                    && let Some(s) = self.dash.commands.get_mut(&command)
                {
                    s.started = None;
                }
            }
        }
        if self.dashboard_shown().is_some() {
            self.redraw_dashboard();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_from_the_api() {
        let params = json!({
            "id": "build", "title": "Build", "state": "working", "status": "2/5",
            "lines": ["one", 7, "two"], "values": [1, 2.5, "x"]
        });
        let (id, tile) = api_tile(&params).unwrap().unwrap();
        assert_eq!(id, "build");
        assert_eq!(tile.title, "Build");
        assert_eq!(tile.state, TileState::Working);
        assert_eq!(tile.status, "2/5");
        assert_eq!(tile.lines, ["one", "two"], "only text lines");
        assert_eq!(tile.values, [1.0, 2.5]);
        assert!(tile.buttons.is_empty(), "no buttons from outside");
        assert_eq!(
            api_tile(&json!({ "id": "build", "remove": true })).unwrap(),
            None
        );
        assert!(api_tile(&json!({ "title": "x" })).is_err(), "no id");
        // Not too much: a tile is small.
        let many = json!({ "id": "x", "lines": vec!["l"; 500], "values": vec![1; 5000] });
        let (_, tile) = api_tile(&many).unwrap().unwrap();
        assert!(tile.lines.len() <= 50 && tile.values.len() <= 500);
    }

    #[test]
    fn api_tiles_change_in_place_and_are_limited() {
        let mut tiles = Vec::new();
        for i in 0..40 {
            put_api_tile(&mut tiles, format!("t{i}"), Some(Tile::default()));
        }
        assert_eq!(tiles.len(), MAX_API_TILES);
        put_api_tile(
            &mut tiles,
            "t39".into(),
            Some(Tile {
                title: "new".into(),
                ..Default::default()
            }),
        );
        assert_eq!(tiles.len(), MAX_API_TILES);
        assert_eq!(
            tiles.last().unwrap().1.title,
            "new",
            "the same id changes in place"
        );
        put_api_tile(&mut tiles, "t39".into(), None);
        assert!(!tiles.iter().any(|(id, _)| id == "t39"));
    }
}
