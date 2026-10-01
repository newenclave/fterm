//! The app: it gets window events from winit, sends keys to the active pane, and draws the window.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fterm_config::keys::{Action, BuiltinAction, SpawnWhere};
use fterm_config::load::{ApiCall, LoadedConfig, SAMPLE_CONFIG, config_path, load_file};
use fterm_config::profiles::{Profile, detect_profiles, launch_command, path_extension, which};
use fterm_mux::{Closed, Direction, Edge, Mux, PaneId, Rect, TabId};
use fterm_render::Renderer;
use fterm_render::builtin::BrailleStyle;
use fterm_render::tabbar::{Hit, TabBarInput, bar_height, hit, layout_tabs};
use fterm_term::alacritty_terminal::grid::Scroll;
use fterm_term::alacritty_terminal::index::{Point, Side};
use fterm_term::alacritty_terminal::selection::{Selection, SelectionType};
use fterm_term::alacritty_terminal::term::TermMode;
use fterm_term::colors::{ColorOverrides, Palette};
use fterm_term::copy_mode::{self, CopyAction, CopyResult};
use fterm_term::links::url_at;
use fterm_term::process::{display_name, running_children};
use fterm_term::session::{Session, SessionOptions, TermEvent};
use fterm_term::shell::{ShellEvent, ShellState, install_scripts, is_powershell, powershell_args};
use fterm_term::size::GridSize;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

use crate::clipboard::{Clipboard, paste_bytes};
use crate::gpu::Gpu;
use crate::input::{KeyInput, copy_mode_action, encode_key, key_chord};
use crate::mouse::{
    ClickCounter, GridGeometry, ReportButton, ReportKind, ReportMods, Wheel, autoscroll_lines,
    encode_mouse,
};
use crate::palette::{PaletteItem, PaletteState, VISIBLE_ROWS};

/// How often auto-scroll moves while the user drags a selection out of the pane.
const AUTOSCROLL_TICK: Duration = Duration::from_millis(16);
/// How long "Copied N lines" stays in the window title.
const TITLE_MESSAGE: Duration = Duration::from_millis(1500);
/// Wait this long after the last change of the config file, then load it.
const CONFIG_DEBOUNCE: Duration = Duration::from_millis(150);

/// Events from other threads to the window thread.
#[derive(Debug)]
pub enum UserEvent {
    Term(PaneId, TermEvent),
    /// The config file changed on disk.
    ConfigChanged,
}

/// One terminal pane: a session and the title that its app set.
struct Pane {
    session: Session,
    app_title: Option<String>,
    /// The folder and the commands, from shell integration.
    shell: ShellState,
}

/// Everything that exists only while the window is open.
struct Running {
    window: Arc<Window>,
    gpu: Gpu,
    renderer: Renderer,
    mux: Mux,
    panes: HashMap<PaneId, Pane>,
}

impl Running {
    fn active_pane(&self) -> Option<&Pane> {
        self.panes.get(&self.mux.active_pane()?)
    }

    fn session(&self) -> Option<&Session> {
        self.active_pane().map(|pane| &pane.session)
    }

    fn bar_height(&self) -> f32 {
        bar_height(self.renderer.cell())
    }

    /// The space for the panes of a tab (the window below the tab bar).
    fn tab_area(&self) -> Rect {
        let size = self.window.inner_size();
        let top = self.bar_height();
        Rect::new(
            0.0,
            top,
            size.width as f32,
            (size.height as f32 - top).max(0.0),
        )
    }

    /// The place of the active pane in the window.
    fn pane_area(&self) -> Rect {
        let area = self.tab_area();
        let active = self.mux.active_pane();
        self.mux
            .pane_rects(area)
            .into_iter()
            .find(|(pane, _)| Some(*pane) == active)
            .map_or(area, |(_, rect)| rect)
    }

    /// How many cells fit in a pane of this size.
    fn grid_for(&self, rect: Rect) -> GridSize {
        let cell = self.renderer.cell();
        GridSize::from_pixels(
            rect.width as u32,
            rect.height as u32,
            cell.width,
            cell.height,
            self.renderer.padding(),
        )
    }

    /// Every pane of every tab gets the size of its own rect.
    fn resize_all_panes(&self) {
        let area = self.tab_area();
        let cell = cell_px(&self.renderer);
        for tab in self.mux.tabs() {
            let rects = match tab.zoomed {
                Some(pane) => vec![(pane, area)],
                None => tab.layout.rects(area),
            };
            for (id, rect) in rects {
                let size = self.grid_for(rect);
                if let Some(pane) = self.panes.get(&id)
                    && pane.session.grid_size() != size
                {
                    pane.session.resize(size, cell);
                }
            }
        }
    }

    fn tab_titles(&self) -> Vec<String> {
        self.mux
            .tabs()
            .iter()
            .map(|tab| {
                let pane = self.panes.get(&tab.active_pane);
                let app = pane.and_then(|p| p.app_title.as_deref());
                let program = pane.map_or("", |p| p.session.program());
                fterm_mux::mux::tab_title(tab.custom_title.as_deref(), app, program).to_owned()
            })
            .collect()
    }
}

/// The state of the mouse.
#[derive(Default)]
struct MouseState {
    /// Last mouse position in the window, in pixels.
    position: (f64, f64),
    /// The left button is down and the user is selecting.
    selecting: bool,
    /// The mouse moved to another cell after the press (so it is a drag, not a click).
    dragged: bool,
    clicks: ClickCounter,
    wheel: Wheel,
    /// The button that the app knows is down (when the app gets the mouse).
    reported_button: Option<ReportButton>,
    /// The last cell sent to the app, so we send motion only when the cell changes.
    reported_cell: Option<(usize, usize)>,
    /// What is under the mouse in the tab bar.
    tab_hover: Hit,
    /// Clicks on tabs (a double click renames).
    tab_clicks: ClickCounter,
    /// The divider that the user drags (its path in the tree).
    dragging_divider: Option<Vec<bool>>,
}

/// What the close question is about.
#[derive(Clone, Copy)]
enum CloseTarget {
    Tab(TabId),
    Pane(PaneId),
}

/// One line of the command palette to draw: (label, key, is it selected).
type PaletteRow = (String, String, bool);

/// "Close the tab? A program is running."
struct CloseQuestion {
    target: CloseTarget,
    lines: Vec<String>,
}

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    running: Option<Running>,
    mods: ModifiersState,
    focused: bool,
    mouse: MouseState,
    clipboard: Clipboard,
    /// When to put the tab title back after a short message.
    title_message_until: Option<Instant>,
    /// The tab that is being renamed, and the text typed so far.
    renaming: Option<(TabId, String)>,
    close_question: Option<CloseQuestion>,
    config: LoadedConfig,
    config_path: std::path::PathBuf,
    /// Profiles from the config, or the ones that fterm found.
    profiles: Vec<Profile>,
    /// Watches the config file. Kept here so it does not stop.
    _watcher: Option<notify::RecommendedWatcher>,
    /// A message box (for example, an error in the config). Any key closes it.
    message: Option<Vec<String>>,
    /// Editors save in several steps: we load the config a moment after the last change.
    reload_at: Option<Instant>,
    /// The PowerShell shell integration script (written at start).
    shell_script: Option<std::path::PathBuf>,
    /// The command palette, when it is open.
    palette: Option<PaletteState>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let config_path = config_path();
        let (config, message) = if config_path.exists() {
            match load_file(&config_path) {
                Ok(config) => {
                    tracing::info!(path = %config_path.display(), "config loaded");
                    (config, None)
                }
                Err(err) => (LoadedConfig::defaults(), Some(error_lines(&err))),
            }
        } else {
            (LoadedConfig::defaults(), None)
        };
        let profiles = profiles_for(&config);
        Self {
            proxy,
            running: None,
            mods: ModifiersState::empty(),
            focused: true,
            mouse: MouseState::default(),
            clipboard: Clipboard::new(),
            title_message_until: None,
            renaming: None,
            close_question: None,
            config,
            config_path,
            profiles,
            _watcher: None,
            message,
            reload_at: None,
            shell_script: shell_script_path(),
            palette: None,
        }
    }

    /// Watches the folder of the config file (editors often write a new file, so we watch the folder).
    fn watch_config(&mut self) {
        let Some(dir) = self.config_path.parent().map(std::path::Path::to_path_buf) else {
            return;
        };
        if !dir.exists() {
            return;
        }
        let file = self.config_path.file_name().map(|f| f.to_owned());
        let proxy = self.proxy.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else {
                return;
            };
            let ours = event
                .paths
                .iter()
                .any(|path| path.file_name() == file.as_deref());
            if ours && (event.kind.is_modify() || event.kind.is_create()) {
                let _ = proxy.send_event(UserEvent::ConfigChanged);
            }
        });
        match watcher {
            Ok(mut watcher) => {
                use notify::Watcher;
                if let Err(err) = watcher.watch(&dir, notify::RecursiveMode::NonRecursive) {
                    tracing::warn!("cannot watch the config: {err}");
                }
                self._watcher = Some(watcher);
            }
            Err(err) => tracing::warn!("cannot watch the config: {err}"),
        }
    }

    /// Reads the config file again. An error keeps the old config and shows a message.
    fn reload_config(&mut self) {
        match load_file(&self.config_path) {
            Ok(config) => {
                tracing::info!("config reloaded");
                self.config = config;
                self.profiles = profiles_for(&self.config);
                self.message = None;
                self.apply_config();
                self.title_message("Config reloaded");
            }
            Err(err) => {
                tracing::warn!("config error: {err}");
                self.message = Some(error_lines(&err));
            }
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Gives the config to the renderer: font, padding, colors, Braille.
    fn apply_config(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        let config = &self.config.config;
        let scale = running.window.scale_factor() as f32;
        let font_px = config.font_size * scale;
        let padding = config.padding * scale;
        if (running.renderer.font_size() - font_px).abs() > 0.01
            || (running.renderer.padding() - padding).abs() > 0.01
        {
            if let Err(err) = running
                .renderer
                .set_font_size(running.gpu.device(), font_px, padding)
            {
                tracing::error!("cannot change the font: {err:#}");
            }
        }
        running.renderer.set_palette(palette_for(&self.config));
        let braille = match config.braille_style {
            fterm_config::load::BrailleStyle::Pixels => BrailleStyle::Pixels,
            fterm_config::load::BrailleStyle::Dots => BrailleStyle::Dots,
        };
        running
            .renderer
            .set_braille_style(running.gpu.device(), braille);
        running.resize_all_panes();
        running.window.request_redraw();
    }

    /// The profile by name, else the default profile, else the first one.
    fn profile(&self, name: Option<&str>) -> Option<Profile> {
        let wanted = name.or(self.config.config.default_profile.as_deref());
        wanted
            .and_then(|n| {
                self.profiles
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(n))
            })
            .or_else(|| self.profiles.first())
            .cloned()
    }

    fn start(&self, event_loop: &ActiveEventLoop) -> anyhow::Result<Running> {
        let attributes = Window::default_attributes()
            .with_title("fterm")
            .with_inner_size(LogicalSize::new(1024.0, 640.0));
        let window = Arc::new(event_loop.create_window(attributes)?);
        // IME: input methods for Chinese, Japanese, Korean, and others.
        window.set_ime_allowed(true);
        let gpu = pollster::block_on(Gpu::new(window.clone(), event_loop.owned_display_handle()))?;
        let scale = window.scale_factor() as f32;
        let config = &self.config.config;
        let mut renderer = Renderer::new(
            gpu.device(),
            gpu.format(),
            config.font_size * scale,
            config.padding * scale,
        )?;
        renderer.set_palette(palette_for(&self.config));
        if config.braille_style == fterm_config::load::BrailleStyle::Dots {
            renderer.set_braille_style(gpu.device(), BrailleStyle::Dots);
        }
        Ok(Running {
            window,
            gpu,
            renderer,
            mux: Mux::default(),
            panes: HashMap::new(),
        })
    }

    /// Starts a profile (or the default profile) for a new pane with this grid size.
    fn spawn_pane(&mut self, size: GridSize, profile: Option<&str>) -> anyhow::Result<PaneId> {
        // A new pane starts in the folder of the active pane, unless the profile has its own folder.
        let active_cwd = self
            .running
            .as_ref()
            .and_then(Running::active_pane)
            .and_then(|pane| pane.shell.cwd.clone())
            .map(std::path::PathBuf::from)
            .filter(|dir| dir.is_dir());
        let options = match self.profile(profile) {
            Some(profile) => {
                let (program, mut args) = launch_command(&profile, cfg!(windows), path_extension);
                if self.config.config.shell_integration
                    && is_powershell(&program)
                    && let Some(script) = &self.shell_script
                {
                    args = powershell_args(&args, script);
                }
                SessionOptions {
                    program: Some(program),
                    args,
                    cwd: profile
                        .cwd
                        .clone()
                        .filter(|dir| dir.is_dir())
                        .or(active_cwd),
                    env: profile.env.clone(),
                    scrollback: self.config.config.scrollback,
                }
            }
            None => SessionOptions {
                scrollback: self.config.config.scrollback,
                ..SessionOptions::default()
            },
        };
        let running = self.running.as_mut().expect("the window is open");
        let id = running.mux.new_pane_id();
        let proxy = self.proxy.clone();
        let session = Session::spawn(options, size, cell_px(&running.renderer), move |event| {
            // The window may be closed already. Then nobody needs the event.
            let _ = proxy.send_event(UserEvent::Term(id, event));
        })?;
        tracing::info!(
            pane = id.0,
            columns = size.columns,
            rows = size.rows,
            "new pane"
        );
        running.panes.insert(
            id,
            Pane {
                session,
                app_title: None,
                shell: ShellState::default(),
            },
        );
        Ok(id)
    }

    /// Starts a profile in a new tab, after the active tab.
    fn new_tab(&mut self, profile: Option<&str>) -> anyhow::Result<PaneId> {
        let running = self.running.as_ref().expect("the window is open");
        let size = running.grid_for(running.tab_area());
        let id = self.spawn_pane(size, profile)?;
        self.running.as_mut().unwrap().mux.new_tab(id);
        self.tab_changed();
        Ok(id)
    }

    /// Splits the active pane. The new pane gets the focus.
    fn split(&mut self, direction: Direction, profile: Option<&str>) -> anyhow::Result<()> {
        let running = self.running.as_ref().expect("the window is open");
        let half = running.pane_area();
        let half = match direction {
            Direction::Right => Rect::new(half.x, half.y, half.width / 2.0, half.height),
            Direction::Down => Rect::new(half.x, half.y, half.width, half.height / 2.0),
        };
        let size = running.grid_for(half);
        let id = self.spawn_pane(size, profile)?;
        let running = self.running.as_mut().unwrap();
        running.mux.split_active(id, direction);
        running.resize_all_panes();
        self.tab_changed();
        Ok(())
    }

    /// Closes one pane now (no question). Returns false when it was the last pane of the last tab.
    fn close_pane_now(&mut self, pane: PaneId) -> bool {
        let Some(running) = &mut self.running else {
            return false;
        };
        running.panes.remove(&pane);
        let alive = running.mux.close_pane(pane) != Closed::LastTab;
        if alive {
            running.resize_all_panes();
            self.tab_changed();
        }
        alive
    }

    /// Closes a tab now (no question). Returns false when it was the last tab.
    fn close_tab_now(&mut self, tab: TabId) -> bool {
        let Some(running) = &mut self.running else {
            return false;
        };
        for pane in running.mux.close_tab(tab) {
            // Dropping the session stops its pty, so the shell ends.
            running.panes.remove(&pane);
        }
        let alive = !running.mux.tabs().is_empty();
        if alive {
            self.tab_changed();
        }
        alive
    }

    /// Closes a tab or a pane, but asks first when a program runs in it.
    fn close(&mut self, event_loop: &ActiveEventLoop, target: CloseTarget) {
        let Some(running) = &self.running else {
            return;
        };
        let panes = match target {
            CloseTarget::Tab(tab) => {
                let Some(tab_info) = running.mux.tabs().iter().find(|t| t.id == tab) else {
                    return;
                };
                tab_info.layout.panes()
            }
            CloseTarget::Pane(pane) => vec![pane],
        };
        let mut programs: Vec<String> = Vec::new();
        for pane in panes {
            if let Some(pid) = running.panes.get(&pane).and_then(|p| p.session.pid()) {
                for name in running_children(pid) {
                    let name = display_name(&name).to_owned();
                    if !programs.contains(&name) {
                        programs.push(name);
                    }
                }
            }
        }
        if programs.is_empty() {
            if !self.close_now(target) {
                event_loop.exit();
            }
            return;
        }
        let what = match target {
            CloseTarget::Tab(_) => "Close this tab?",
            CloseTarget::Pane(_) => "Close this pane?",
        };
        self.close_question = Some(CloseQuestion {
            target,
            lines: vec![
                what.to_owned(),
                format!("Running: {}", programs.join(", ")),
                String::new(),
                "Enter = close, Esc = cancel".to_owned(),
            ],
        });
        running.window.request_redraw();
    }

    fn close_now(&mut self, target: CloseTarget) -> bool {
        match target {
            CloseTarget::Tab(tab) => self.close_tab_now(tab),
            CloseTarget::Pane(pane) => self.close_pane_now(pane),
        }
    }

    /// The active tab or the tab list changed: new window title, new sizes, and a redraw.
    fn tab_changed(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        // Panes may have an old size (for example, the window changed while their tab was hidden).
        running.resize_all_panes();
        self.update_window_title();
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    fn update_window_title(&self) {
        let Some(running) = &self.running else {
            return;
        };
        if self.title_message_until.is_some() {
            return;
        }
        let titles = running.tab_titles();
        let title = titles
            .get(running.mux.active_index())
            .map_or("fterm", String::as_str);
        running.window.set_title(title);
    }

    /// Shows a short message in the window title.
    fn title_message(&mut self, message: &str) {
        if let Some(running) = &self.running {
            let titles = running.tab_titles();
            let title = titles
                .get(running.mux.active_index())
                .map_or("fterm", String::as_str);
            running.window.set_title(&format!("{title} — {message}"));
            self.title_message_until = Some(Instant::now() + TITLE_MESSAGE);
        }
    }

    fn copy_text(&mut self, text: String) {
        let lines = text.lines().count().max(1);
        self.clipboard.copy(&text);
        let message = if lines == 1 {
            "Copied 1 line".to_owned()
        } else {
            format!("Copied {lines} lines")
        };
        self.title_message(&message);
    }

    fn paste(&mut self) {
        let Some(text) = self.clipboard.paste() else {
            return;
        };
        let Some(session) = self.running.as_ref().and_then(Running::session) else {
            return;
        };
        let bracketed = session.with_term(|term| term.mode().contains(TermMode::BRACKETED_PASTE));
        session.with_term_mut(|term, _| term.scroll_display(Scroll::Bottom));
        session.write(paste_bytes(&text, bracketed));
    }

    /// Runs an action from a key, the palette, or a Lua function.
    fn run_action(&mut self, event_loop: &ActiveEventLoop, action: Action) {
        match action {
            Action::Builtin(builtin) => self.run_builtin(event_loop, builtin),
            Action::Spawn { profile, place } => self.spawn(profile.as_deref(), place),
            Action::Lua(index) => match self.config.call(index) {
                Ok(calls) => {
                    for call in calls {
                        self.run_api_call(event_loop, call);
                    }
                }
                Err(err) => {
                    tracing::warn!("Lua error: {err}");
                    self.message = Some(error_lines(&err));
                    if let Some(running) = &self.running {
                        running.window.request_redraw();
                    }
                }
            },
        }
    }

    fn run_api_call(&mut self, event_loop: &ActiveEventLoop, call: ApiCall) {
        match call {
            ApiCall::Spawn { profile, place } => self.spawn(profile.as_deref(), place),
            ApiCall::SendText(text) => {
                if let Some(session) = self.running.as_ref().and_then(Running::session) {
                    session.with_term_mut(|term, _| term.scroll_display(Scroll::Bottom));
                    session.write(text.into_bytes());
                }
            }
            ApiCall::Notify(text) => self.title_message(&text),
            ApiCall::Copy(text) => self.copy_text(text),
            ApiCall::Action(builtin) => self.run_builtin(event_loop, builtin),
        }
    }

    fn spawn(&mut self, profile: Option<&str>, place: SpawnWhere) {
        let result = match place {
            SpawnWhere::Tab => self.new_tab(profile).map(|_| ()),
            SpawnWhere::SplitRight => self.split(Direction::Right, profile),
            SpawnWhere::SplitDown => self.split(Direction::Down, profile),
        };
        if let Err(err) = result {
            tracing::error!("cannot start: {err:#}");
            self.message = Some(error_lines(&format!("Cannot start: {err:#}")));
        }
    }

    fn run_builtin(&mut self, event_loop: &ActiveEventLoop, action: BuiltinAction) {
        use BuiltinAction as A;
        let Some(running) = &mut self.running else {
            return;
        };
        let scroll = |running: &Running, scroll: Scroll| {
            if let Some(session) = running.session() {
                session.with_term_mut(|term, _| term.scroll_display(scroll));
                running.window.request_redraw();
            }
        };
        match action {
            A::NewTab => return self.spawn(None, SpawnWhere::Tab),
            A::SplitRight => return self.spawn(None, SpawnWhere::SplitRight),
            A::SplitDown => return self.spawn(None, SpawnWhere::SplitDown),
            A::NextTab => running.mux.cycle(1),
            A::PrevTab => running.mux.cycle(-1),
            A::SelectTab(i) => running.mux.select(i),
            A::LastTab => running.mux.select_last(),
            A::MoveTabLeft => running.mux.move_active(-1),
            A::MoveTabRight => running.mux.move_active(1),
            A::RenameTab => {
                if let Some(tab) = running.mux.active_tab().map(|t| t.id) {
                    self.start_rename(tab);
                }
                return;
            }
            A::FocusLeft | A::FocusRight | A::FocusUp | A::FocusDown => {
                let area = running.tab_area();
                running.mux.focus_direction(edge_of(action), area);
            }
            A::ResizeLeft | A::ResizeRight | A::ResizeUp | A::ResizeDown => {
                let edge = edge_of(action);
                let area = running.tab_area();
                let cell = running.renderer.cell();
                let step = match edge {
                    Edge::Left | Edge::Right => cell.width,
                    Edge::Up | Edge::Down => cell.height,
                };
                if let Some(pane) = running.mux.active_pane()
                    && let Some(layout) = running.mux.active_layout_mut()
                {
                    layout.move_divider(pane, edge, step, area);
                }
            }
            A::Zoom => running.mux.toggle_zoom(),
            A::ClosePane => {
                if let Some(pane) = running.mux.active_pane() {
                    self.close(event_loop, CloseTarget::Pane(pane));
                }
                return;
            }
            A::Copy => {
                let text = running
                    .session()
                    .and_then(|s| s.with_term(|term| term.selection_to_string()));
                if let Some(text) = text.filter(|t| !t.is_empty()) {
                    self.copy_text(text);
                }
                return;
            }
            A::Paste => return self.paste(),
            A::CopyMode => {
                if let Some(session) = running.session() {
                    let active = session.with_term(copy_mode::is_active);
                    session.with_term_mut(|term, selection| {
                        if active {
                            copy_mode::apply(term, selection, CopyAction::Exit);
                        } else {
                            copy_mode::enter(term, selection);
                        }
                    });
                    running.window.request_redraw();
                }
                return;
            }
            A::ScrollPageUp => return scroll(running, Scroll::PageUp),
            A::ScrollPageDown => return scroll(running, Scroll::PageDown),
            A::ScrollTop => return scroll(running, Scroll::Top),
            A::ScrollBottom => return scroll(running, Scroll::Bottom),
            A::CommandPalette => {
                running.window.request_redraw();
                self.palette = Some(PaletteState::new(self.palette_items()));
                return;
            }
            A::ReloadConfig => return self.reload_config(),
            A::OpenConfig => return self.open_config(),
        }
        self.tab_changed();
    }

    /// All lines of the command palette: profiles, actions, and the commands from the config.
    fn palette_items(&self) -> Vec<PaletteItem> {
        let keys = &self.config.config.keys;
        let key = |action: &Action| keys.key_for(action).unwrap_or_default();
        let mut items = Vec::new();
        for profile in &self.profiles {
            for (label, place) in [
                ("New tab", SpawnWhere::Tab),
                ("Split right", SpawnWhere::SplitRight),
                ("Split down", SpawnWhere::SplitDown),
            ] {
                let action = Action::Spawn {
                    profile: Some(profile.name.clone()),
                    place,
                };
                items.push(PaletteItem {
                    label: format!("{label}: {}", profile.name),
                    key: key(&action),
                    action,
                });
            }
        }
        for command in &self.config.config.commands {
            items.push(PaletteItem {
                label: command.name.clone(),
                key: key(&command.action),
                action: command.action.clone(),
            });
        }
        for builtin in BuiltinAction::ALL {
            if builtin == BuiltinAction::CommandPalette {
                continue;
            }
            let action = Action::Builtin(builtin);
            items.push(PaletteItem {
                label: builtin.label(),
                key: key(&action),
                action,
            });
        }
        items
    }

    /// Keys while the command palette is open. All keys go to the palette.
    fn palette_key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        let Some(palette) = &mut self.palette else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.palette = None,
            Key::Named(NamedKey::Enter) => {
                let action = palette.selected_action().cloned();
                self.palette = None;
                if let Some(action) = action {
                    self.run_action(event_loop, action);
                }
            }
            Key::Named(NamedKey::ArrowUp) => palette.move_selection(-1),
            Key::Named(NamedKey::ArrowDown) => palette.move_selection(1),
            Key::Named(NamedKey::PageUp) => palette.move_selection(-(VISIBLE_ROWS as i32)),
            Key::Named(NamedKey::PageDown) => palette.move_selection(VISIBLE_ROWS as i32),
            Key::Named(NamedKey::Backspace) => palette.backspace(),
            _ => {
                if let Some(text) = event.text.as_deref()
                    && !self.mods.control_key()
                    && !self.mods.alt_key()
                {
                    palette.type_text(text);
                }
            }
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Opens the config file in the default editor. It makes a sample file first if there is none.
    fn open_config(&mut self) {
        let path = self.config_path.clone();
        if !path.exists() {
            let made = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, SAMPLE_CONFIG));
            if let Err(err) = made {
                self.message = Some(error_lines(&format!(
                    "Cannot make {}: {err}",
                    path.display()
                )));
                return;
            }
            // Now there is a folder to watch.
            if self._watcher.is_none() {
                self.watch_config();
            }
        }
        if let Err(err) = open::that_detached(&path) {
            self.message = Some(error_lines(&format!(
                "Cannot open {}: {err}",
                path.display()
            )));
        }
    }

    fn start_rename(&mut self, tab: TabId) {
        let Some(running) = &self.running else {
            return;
        };
        let index = running.mux.tabs().iter().position(|t| t.id == tab);
        let current = index
            .and_then(|i| running.tab_titles().get(i).cloned())
            .unwrap_or_default();
        self.renaming = Some((tab, current));
        running.window.request_redraw();
    }

    /// Keys while a tab is renamed. All keys go to the name.
    fn rename_key(&mut self, event: &KeyEvent) {
        let Some((tab, text)) = &mut self.renaming else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => {
                let (tab, text) = (*tab, text.clone());
                self.renaming = None;
                if let Some(running) = &mut self.running {
                    running.mux.rename(tab, &text);
                }
                self.update_window_title();
            }
            Key::Named(NamedKey::Escape) => self.renaming = None,
            Key::Named(NamedKey::Backspace) => {
                text.pop();
            }
            _ => {
                if let Some(typed) = event.text.as_deref()
                    && !self.mods.control_key()
                {
                    text.extend(typed.chars().filter(|c| !c.is_control()));
                }
            }
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Keys while the close question is open.
    fn close_question_key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => {
                if let Some(question) = self.close_question.take()
                    && !self.close_now(question.target)
                {
                    event_loop.exit();
                }
            }
            Key::Named(NamedKey::Escape) => self.close_question = None,
            _ => return,
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Copy mode keys and the Ctrl+C rule. Returns true when the key is used.
    fn handle_copy_keys(&mut self, event: &KeyEvent, action: Option<&Action>) -> bool {
        let Some(running) = &self.running else {
            return false;
        };
        let Some(session) = running.session() else {
            return false;
        };
        let key = KeyInput {
            logical: &event.logical_key,
            physical: event.physical_key,
            text: event.text.as_deref(),
            mods: self.mods,
        };

        // In copy mode, all keys belong to copy mode (the copy mode key itself closes it).
        if session.with_term(copy_mode::is_active) {
            if action == Some(&Action::Builtin(BuiltinAction::CopyMode)) {
                return false;
            }
            if let Some(copy_action) = copy_mode_action(&key) {
                let result = session.with_term_mut(|term, selection| {
                    copy_mode::apply(term, selection, copy_action)
                });
                running.window.request_redraw();
                if let CopyResult::Copied(text) = result {
                    self.copy_text(text);
                }
            }
            return true;
        }

        // Ctrl+C copies when there is a selection, else it goes to the app as ^C.
        if self.mods.control_key()
            && !self.mods.shift_key()
            && !self.mods.alt_key()
            && event.physical_key == PhysicalKey::Code(KeyCode::KeyC)
        {
            let selected = session.with_term(|term| term.selection_to_string());
            if let Some(text) = selected.filter(|t| !t.is_empty()) {
                self.copy_text(text);
                return true;
            }
        }
        false
    }

    fn geometry(running: &Running) -> GridGeometry {
        let cell = running.renderer.cell();
        GridGeometry {
            cell_width: cell.width,
            cell_height: cell.height,
            padding: running.renderer.padding(),
            size: running
                .session()
                .map_or(GridSize::new(1, 1), Session::grid_size),
        }
    }

    /// The mouse position inside the active pane.
    fn mouse_in_pane(&self, running: &Running) -> (f64, f64) {
        let area = running.pane_area();
        let (x, y) = self.mouse.position;
        (x - f64::from(area.x), y - f64::from(area.y))
    }

    /// The cell under the mouse now, in history coordinates.
    fn mouse_cell(&self, running: &Running) -> (Point, Side) {
        let offset = running
            .session()
            .map_or(0, |s| s.with_term(|term| term.grid().display_offset()));
        let (x, y) = self.mouse_in_pane(running);
        Self::geometry(running).cell_at(x, y, offset)
    }

    /// Moves the end of the selection to the mouse.
    fn extend_selection_to_mouse(&self, running: &Running) {
        let (point, side) = self.mouse_cell(running);
        let Some(session) = running.session() else {
            return;
        };
        session.with_term_mut(|term, sticky| {
            if let Some(mut selection) = term.selection.clone() {
                selection.update(point, side);
                sticky.set(term, Some(selection));
            }
        });
        running.window.request_redraw();
    }

    fn mouse_press(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let Some(session) = running.session() else {
            return;
        };
        let (point, side) = self.mouse_cell(running);
        let has_selection = session.with_term(|term| term.selection.is_some());
        self.mouse.selecting = true;
        self.mouse.dragged = false;

        // Shift + click: move the end of the selection (also after scrolling far away).
        if self.mods.shift_key() && has_selection {
            self.mouse.dragged = true;
            self.extend_selection_to_mouse(running);
            return;
        }

        let ty = match self.mouse.clicks.click(Instant::now(), point) {
            2 => SelectionType::Semantic,
            3 => SelectionType::Lines,
            _ if self.mods.alt_key() => SelectionType::Block,
            _ => SelectionType::Simple,
        };
        if ty != SelectionType::Simple && ty != SelectionType::Block {
            self.mouse.dragged = true;
        }
        session.with_term_mut(|term, sticky| {
            sticky.set(term, Some(Selection::new(ty, point, side)));
        });
        running.window.request_redraw();
    }

    fn mouse_release(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        self.mouse.selecting = false;
        // A click without a drag removes the selection.
        if !self.mouse.dragged
            && let Some(session) = running.session()
        {
            session.with_term_mut(|term, sticky| sticky.set(term, None));
            running.window.request_redraw();
        }
    }

    /// True when the app wants the mouse and Shift is not held (Shift always selects).
    fn app_wants_mouse(&self) -> bool {
        let Some(session) = self.running.as_ref().and_then(Running::session) else {
            return false;
        };
        !self.mods.shift_key()
            && session.with_term(|term| term.mode().intersects(TermMode::MOUSE_MODE))
    }

    /// Sends a mouse event to the app at the current mouse cell.
    fn report_mouse(&mut self, kind: ReportKind) {
        let Some(running) = &self.running else {
            return;
        };
        let Some(session) = running.session() else {
            return;
        };
        let (x, y) = self.mouse_in_pane(running);
        let (point, _) = Self::geometry(running).cell_at(x, y, 0);
        let cell = (point.column.0, point.line.0.max(0) as usize);
        let sgr = session.with_term(|term| term.mode().contains(TermMode::SGR_MOUSE));
        let mods = ReportMods {
            shift: self.mods.shift_key(),
            alt: self.mods.alt_key(),
            ctrl: self.mods.control_key(),
        };
        if let Some(bytes) = encode_mouse(kind, cell.0, cell.1, mods, sgr) {
            session.write(bytes);
        }
        self.mouse.reported_cell = Some(cell);
    }

    /// The tab bar part under the mouse.
    fn tab_bar_hit(&self) -> Hit {
        let Some(running) = &self.running else {
            return Hit::None;
        };
        let width = running.window.inner_size().width as f32;
        let layout = layout_tabs(running.mux.tabs().len(), width, running.renderer.cell());
        let (x, y) = self.mouse.position;
        hit(&layout, x as f32, y as f32)
    }

    fn tab_bar_button(&mut self, event_loop: &ActiveEventLoop, button: MouseButton, hit: Hit) {
        let Some(running) = &mut self.running else {
            return;
        };
        let tab_id = match hit {
            Hit::Tab(i) | Hit::Close(i) => running.mux.tabs().get(i).map(|t| t.id),
            _ => None,
        };
        match (button, hit) {
            (MouseButton::Left, Hit::Tab(i)) => {
                running.mux.select(i);
                let point = Point::new(fterm_term::alacritty_terminal::index::Line(0), i.into());
                let double = self.mouse.tab_clicks.click(Instant::now(), point) == 2;
                if double && let Some(tab) = tab_id {
                    self.start_rename(tab);
                }
                self.tab_changed();
            }
            (MouseButton::Left, Hit::Close(_))
            | (MouseButton::Middle, Hit::Tab(_) | Hit::Close(_)) => {
                if let Some(tab) = tab_id {
                    self.close(event_loop, CloseTarget::Tab(tab));
                }
            }
            (MouseButton::Left, Hit::NewTab) => self.spawn(None, SpawnWhere::Tab),
            _ => {}
        }
    }

    /// The divider near the mouse (3 px around the line), with its direction.
    fn divider_under_mouse(&self) -> Option<(Vec<bool>, Direction)> {
        let running = self.running.as_ref()?;
        let tab = running.mux.active_tab()?;
        if tab.zoomed.is_some() {
            return None;
        }
        let (x, y) = (self.mouse.position.0 as f32, self.mouse.position.1 as f32);
        tab.layout
            .dividers(running.tab_area())
            .into_iter()
            .find(|d| {
                let r = d.rect;
                x >= r.x - 3.0
                    && x <= r.x + r.width + 3.0
                    && y >= r.y - 3.0
                    && y <= r.y + r.height + 3.0
            })
            .map(|d| (d.path, d.direction))
    }

    /// The pane under the mouse in the active tab.
    fn pane_under_mouse(&self) -> Option<PaneId> {
        let running = self.running.as_ref()?;
        let (x, y) = (self.mouse.position.0 as f32, self.mouse.position.1 as f32);
        running
            .mux
            .pane_rects(running.tab_area())
            .into_iter()
            .find(|(_, rect)| rect.contains(x, y))
            .map(|(pane, _)| pane)
    }

    fn mouse_button(
        &mut self,
        event_loop: &ActiveEventLoop,
        state: ElementState,
        button: MouseButton,
    ) {
        if self.close_question.is_some() || self.palette.is_some() {
            if self.palette.is_some() && state == ElementState::Pressed {
                // A click outside of the list closes the palette.
                self.palette = None;
                if let Some(running) = &self.running {
                    running.window.request_redraw();
                }
            }
            return;
        }
        // The tab bar.
        if !self.mouse.selecting {
            let hit = self.tab_bar_hit();
            if hit != Hit::None {
                if state == ElementState::Pressed {
                    self.tab_bar_button(event_loop, button, hit);
                }
                return;
            }
        }
        // Drag a divider between panes.
        if button == MouseButton::Left {
            match state {
                ElementState::Pressed => {
                    if let Some((path, _)) = self.divider_under_mouse() {
                        self.mouse.dragging_divider = Some(path);
                        return;
                    }
                }
                ElementState::Released => {
                    if self.mouse.dragging_divider.take().is_some() {
                        return;
                    }
                }
            }
        }
        // A press in another pane gives it the focus first.
        if state == ElementState::Pressed
            && let Some(pane) = self.pane_under_mouse()
            && let Some(running) = &mut self.running
            && running.mux.active_pane() != Some(pane)
        {
            running.mux.focus(pane);
            self.tab_changed();
        }
        let report_button = match button {
            MouseButton::Left => Some(ReportButton::Left),
            MouseButton::Middle => Some(ReportButton::Middle),
            MouseButton::Right => Some(ReportButton::Right),
            _ => None,
        };
        if let Some(b) = report_button
            && self.app_wants_mouse()
        {
            match state {
                ElementState::Pressed => {
                    self.mouse.reported_button = Some(b);
                    self.report_mouse(ReportKind::Press(b));
                }
                ElementState::Released => {
                    self.mouse.reported_button = None;
                    self.report_mouse(ReportKind::Release(b));
                }
            }
            return;
        }
        match (button, state) {
            (MouseButton::Left, ElementState::Pressed) => {
                if self.mods.control_key() && self.open_link_under_mouse() {
                    return;
                }
                self.mouse_press();
            }
            (MouseButton::Left, ElementState::Released) => self.mouse_release(),
            (MouseButton::Right, ElementState::Pressed) => self.paste(),
            _ => {}
        }
    }

    /// Ctrl + click on a URL opens it. Returns true when there was a URL.
    fn open_link_under_mouse(&self) -> bool {
        let Some(running) = &self.running else {
            return false;
        };
        let Some(session) = running.session() else {
            return false;
        };
        let (point, _) = self.mouse_cell(running);
        let Some(url) = session.with_term(|term| url_at(term, point)) else {
            return false;
        };
        tracing::info!(%url, "open link");
        if let Err(err) = open::that_detached(&url) {
            tracing::warn!("cannot open {url}: {err}");
        }
        true
    }

    fn mouse_moved(&mut self, x: f64, y: f64) {
        let old = self.mouse.position;
        self.mouse.position = (x, y);

        // Hover in the tab bar.
        let hover = if self.mouse.selecting {
            Hit::None
        } else {
            self.tab_bar_hit()
        };
        if hover != self.mouse.tab_hover {
            self.mouse.tab_hover = hover;
            if let Some(running) = &self.running {
                running.window.request_redraw();
            }
        }

        // Drag a divider: its ratio follows the mouse.
        if let Some(path) = self.mouse.dragging_divider.clone() {
            if let Some(running) = &mut self.running {
                let area = running.tab_area();
                let (fx, fy) = (x as f32, y as f32);
                if let Some(layout) = running.mux.active_layout_mut()
                    && let Some(ratio) = layout.ratio_at(&path, area, fx, fy)
                {
                    layout.set_ratio(&path, ratio);
                }
                running.resize_all_panes();
                running.window.request_redraw();
            }
            return;
        }
        // The resize arrow over dividers.
        if let Some(running) = &self.running {
            let icon = match self.divider_under_mouse() {
                Some((_, Direction::Right)) => CursorIcon::ColResize,
                Some((_, Direction::Down)) => CursorIcon::RowResize,
                None => CursorIcon::Default,
            };
            running.window.set_cursor(icon);
        }

        if self.app_wants_mouse() {
            let Some(running) = &self.running else {
                return;
            };
            let Some(session) = running.session() else {
                return;
            };
            let (motion, drag) = session.with_term(|term| {
                let mode = term.mode();
                (
                    mode.contains(TermMode::MOUSE_MOTION),
                    mode.contains(TermMode::MOUSE_DRAG),
                )
            });
            let button = self.mouse.reported_button;
            let (px, py) = self.mouse_in_pane(running);
            let (point, _) = Self::geometry(running).cell_at(px, py, 0);
            let cell = (point.column.0, point.line.0.max(0) as usize);
            let in_pane = py >= 0.0;
            if in_pane
                && (motion || (drag && button.is_some()))
                && self.mouse.reported_cell != Some(cell)
            {
                self.report_mouse(ReportKind::Motion(button));
            }
            return;
        }
        let Some(running) = &self.running else {
            return;
        };
        if !self.mouse.selecting {
            return;
        }
        let geometry = Self::geometry(running);
        let old_cell = geometry.cell_at(old.0, old.1, 0).0;
        let new_cell = geometry.cell_at(x, y, 0).0;
        if old_cell != new_cell {
            self.mouse.dragged = true;
        }
        self.extend_selection_to_mouse(running);
    }

    fn mouse_wheel(&mut self, delta: MouseScrollDelta) {
        if let Some(palette) = &mut self.palette {
            let step = match delta {
                MouseScrollDelta::LineDelta(_, y) => -y.signum() as i32,
                MouseScrollDelta::PixelDelta(p) => -(p.y.signum() as i32),
            };
            palette.move_selection(step);
            if let Some(running) = &self.running {
                running.window.request_redraw();
            }
            return;
        }
        let Some(running) = &mut self.running else {
            return;
        };
        let lines = self
            .mouse
            .wheel
            .lines(delta, running.renderer.cell().height);
        if lines == 0 {
            return;
        }
        // The wheel on the tab bar switches tabs.
        if self.tab_bar_hit() != Hit::None {
            if let Some(running) = &mut self.running {
                running.mux.cycle(if lines > 0 { -1 } else { 1 });
            }
            self.tab_changed();
            return;
        }
        if self.app_wants_mouse() {
            let kind = if lines > 0 {
                ReportKind::WheelUp
            } else {
                ReportKind::WheelDown
            };
            for _ in 0..lines.unsigned_abs().min(10) {
                self.report_mouse(kind);
            }
            return;
        }
        let Some(running) = &self.running else {
            return;
        };
        let Some(session) = running.session() else {
            return;
        };
        let alt_screen = session.with_term(|term| term.mode().contains(TermMode::ALT_SCREEN));
        if alt_screen {
            // Full-screen apps (vim, less) have no history: the wheel sends arrow keys.
            let arrow: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
            session.write(arrow.repeat(lines.unsigned_abs() as usize));
            return;
        }
        session.with_term_mut(|term, _| term.scroll_display(Scroll::Delta(lines)));
        // While selecting, the end of the selection follows the mouse in the new view.
        if self.mouse.selecting {
            self.extend_selection_to_mouse(running);
        }
        running.window.request_redraw();
    }

    /// Scrolls while the user drags the selection above or below the pane.
    fn autoscroll(&mut self) -> bool {
        let Some(running) = &self.running else {
            return false;
        };
        if !self.mouse.selecting {
            return false;
        }
        let Some(session) = running.session() else {
            return false;
        };
        let area = running.pane_area();
        let (_, y) = self.mouse_in_pane(running);
        let lines = autoscroll_lines(y, f64::from(area.height));
        if lines == 0 {
            return false;
        }
        session.with_term_mut(|term, _| term.scroll_display(Scroll::Delta(lines)));
        self.mouse.dragged = true;
        self.extend_selection_to_mouse(running);
        true
    }

    /// IME text is ready: send it like typed text.
    fn ime_commit(&self, text: &str) {
        let Some(session) = self.running.as_ref().and_then(Running::session) else {
            return;
        };
        session.with_term_mut(|term, sticky| {
            term.scroll_display(Scroll::Bottom);
            if sticky.is_active() {
                sticky.set(term, None);
            }
        });
        session.write(text.as_bytes().to_vec());
    }

    /// Tells the IME where the cursor is, so its window opens next to the cursor.
    fn update_ime_area(running: &Running) {
        let Some(session) = running.session() else {
            return;
        };
        let cell = running.renderer.cell();
        let padding = running.renderer.padding();
        let area = running.pane_area();
        let (line, column) = session.with_term(|term| {
            let point = term.grid().cursor.point;
            (
                point.line.0 + term.grid().display_offset() as i32,
                point.column.0,
            )
        });
        let x = area.x + padding + column as f32 * cell.width;
        let y = area.y + padding + line.max(0) as f32 * cell.height;
        running.window.set_ime_cursor_area(
            PhysicalPosition::new(f64::from(x), f64::from(y)),
            PhysicalSize::new(f64::from(cell.width), f64::from(cell.height)),
        );
    }

    /// Every pane of every tab gets the size of its own rect.
    fn resize_all_panes(&self) {
        if let Some(running) = &self.running {
            running.resize_all_panes();
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let focused = self.focused;
        let titles = match &self.running {
            Some(running) => running.tab_titles(),
            None => return,
        };
        let renaming = self
            .renaming
            .as_ref()
            .map(|(tab, text)| (*tab, text.clone()));
        let hover = self.mouse.tab_hover;
        let question = self
            .close_question
            .as_ref()
            .map(|q| q.lines.clone())
            .or_else(|| self.message.clone());
        let palette_rows: Option<(String, Vec<PaletteRow>)> = self.palette.as_ref().map(|p| {
            let rows = p
                .visible()
                .into_iter()
                .map(|(item, selected)| (item.label.clone(), item.key.clone(), selected))
                .collect();
            (p.query().to_owned(), rows)
        });
        let running = self.running.as_mut().unwrap();
        let Running {
            gpu,
            renderer,
            mux,
            panes,
            window,
        } = running;
        let size = window.inner_size();
        let cell = renderer.cell();
        let top = bar_height(cell);
        let width = size.width as f32;
        let area = Rect::new(0.0, top, width, (size.height as f32 - top).max(0.0));
        let pane_rects = mux.pane_rects(area);
        let active_pane = mux.active_pane();
        let zoomed = mux.active_tab().is_some_and(|t| t.zoomed.is_some());
        let dividers: Vec<Rect> = match mux.active_tab() {
            Some(tab) if !zoomed => tab
                .layout
                .dividers(area)
                .into_iter()
                .map(|d| d.rect)
                .collect(),
            _ => Vec::new(),
        };
        let active_frame = (pane_rects.len() > 1)
            .then(|| {
                pane_rects
                    .iter()
                    .find(|(p, _)| Some(*p) == active_pane)
                    .map(|(_, r)| *r)
            })
            .flatten();
        let layout = layout_tabs(mux.tabs().len(), width, cell);
        let active = mux.active_index();
        let editing = renaming.as_ref().and_then(|(tab, text)| {
            let index = mux.tabs().iter().position(|t| t.id == *tab)?;
            Some((index, text.as_str()))
        });
        let view = Rect::new(0.0, 0.0, width, size.height as f32);

        let result = gpu.frame(|device, queue, target, size| {
            renderer.render(device, queue, target, size, |parts| {
                parts.tab_bar(&TabBarInput {
                    layout: &layout,
                    titles: &titles,
                    active,
                    hover,
                    editing,
                    cell,
                    width,
                })?;
                for (id, rect) in &pane_rects {
                    if let Some(pane) = panes.get(id) {
                        let is_active = Some(*id) == active_pane;
                        pane.session
                            .with_term(|term| parts.pane(term, *rect, focused && is_active))?;
                    }
                }
                parts.pane_chrome(&dividers, active_frame);
                if let Some((query, rows)) = &palette_rows {
                    parts.palette(&fterm_render::overlay::PaletteView { query, rows }, view)?;
                }
                if let Some(lines) = &question {
                    parts.message_box(lines, view)?;
                }
                Ok(())
            });
        });
        if let Err(err) = result {
            tracing::error!("cannot draw: {err:#}");
            event_loop.exit();
            return;
        }
        Self::update_ime_area(self.running.as_ref().unwrap());
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        self.watch_config();
        match self.start(event_loop) {
            Ok(running) => self.running = Some(running),
            Err(err) => {
                tracing::error!("cannot start: {err:#}");
                event_loop.exit();
                return;
            }
        }
        // Dev only: FTERM_TABS=N opens N tabs at start (for test scripts).
        let tabs = if cfg!(debug_assertions) {
            std::env::var("FTERM_TABS")
                .ok()
                .and_then(|n| n.parse().ok())
                .unwrap_or(1)
        } else {
            1
        };
        for _ in 0..tabs.clamp(1, 20) {
            if let Err(err) = self.new_tab(None) {
                tracing::error!("cannot start the shell: {err:#}");
                event_loop.exit();
                return;
            }
        }
        if let Some(running) = &mut self.running {
            running.mux.select(0);
        }
        // Dev only: FTERM_SPLITS="right,down" splits the first tab at start (for test scripts).
        if cfg!(debug_assertions)
            && let Ok(splits) = std::env::var("FTERM_SPLITS")
        {
            for split in splits.split(',') {
                let direction = match split.trim() {
                    "right" => Direction::Right,
                    "down" => Direction::Down,
                    _ => continue,
                };
                if let Err(err) = self.split(direction, None) {
                    tracing::error!("cannot split: {err:#}");
                }
            }
        }
        if let Some(running) = &mut self.running {
            // Dev only: FTERM_RUN="command" types this command into the first tab after start.
            // Test scripts use it, so they do not need to send keys to the window.
            if cfg!(debug_assertions)
                && let Ok(command) = std::env::var("FTERM_RUN")
                && let Some(session) = running.session()
            {
                tracing::info!(%command, "FTERM_RUN");
                session.write(format!("{command}\r").into_bytes());
            }
        }
        self.tab_changed();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        let Some(running) = &mut self.running else {
            return;
        };
        let (pane, event) = match event {
            UserEvent::Term(pane, event) => (pane, event),
            UserEvent::ConfigChanged => {
                self.reload_at = Some(Instant::now() + CONFIG_DEBOUNCE);
                return;
            }
        };
        match event {
            TermEvent::Redraw => {
                if running.mux.active_pane() == Some(pane) {
                    running.window.request_redraw();
                }
            }
            TermEvent::Title(title) => {
                if let Some(p) = running.panes.get_mut(&pane) {
                    p.app_title = (!title.is_empty()).then_some(title);
                }
                running.window.request_redraw();
                self.update_window_title();
            }
            TermEvent::Osc(osc) => {
                tracing::debug!(pane = pane.0, ?osc, "osc event");
                let Some(p) = running.panes.get_mut(&pane) else {
                    return;
                };
                if let Some(ShellEvent::CommandDone { exit, took }) =
                    p.shell.apply(&osc, Instant::now())
                {
                    tracing::debug!(pane = pane.0, ?exit, ?took, "command done");
                }
            }
            TermEvent::Exit => {
                tracing::info!(pane = pane.0, "the shell ended");
                running.panes.remove(&pane);
                match running.mux.close_pane(pane) {
                    Closed::LastTab => {
                        tracing::info!("the last tab closed, closing the window");
                        event_loop.exit();
                    }
                    Closed::Nothing => {}
                    Closed::Pane | Closed::Tab => {
                        running.resize_all_panes();
                        self.tab_changed();
                    }
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let mut wake_at = None;
        match self.reload_at {
            Some(at) if now >= at => {
                self.reload_at = None;
                self.reload_config();
            }
            Some(at) => wake_at = Some(at),
            None => {}
        }
        if self.autoscroll() {
            let tick = now + AUTOSCROLL_TICK;
            wake_at = Some(wake_at.map_or(tick, |t: Instant| t.min(tick)));
        }
        if let Some(until) = self.title_message_until {
            if now >= until {
                self.title_message_until = None;
                self.update_window_title();
            } else {
                wake_at = Some(wake_at.map_or(until, |t: Instant| t.min(until)));
            }
        }
        event_loop.set_control_flow(match wake_at {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.running.is_none() {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                let running = self.running.as_mut().unwrap();
                running.gpu.resize(size);
                self.resize_all_panes();
                self.running.as_ref().unwrap().window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let running = self.running.as_mut().unwrap();
                tracing::debug!(scale_factor, "scale factor changed");
                let scale = scale_factor as f32;
                let config = &self.config.config;
                if let Err(err) = running.renderer.set_font_size(
                    running.gpu.device(),
                    config.font_size * scale,
                    config.padding * scale,
                ) {
                    tracing::error!("cannot change the font size: {err:#}");
                }
                // The new window size comes in the next `Resized` event.
                self.resize_all_panes();
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                let running = self.running.as_ref().unwrap();
                if let Some(session) = running.session() {
                    let report =
                        session.with_term(|term| term.mode().contains(TermMode::FOCUS_IN_OUT));
                    if report {
                        let bytes: &[u8] = if focused { b"\x1b[I" } else { b"\x1b[O" };
                        session.write(bytes.to_vec());
                    }
                }
                running.window.request_redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => self.ime_commit(&text),
            WindowEvent::ModifiersChanged(mods) => self.mods = mods.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.close_question.is_some() {
                    self.close_question_key(event_loop, &event);
                    return;
                }
                if self.palette.is_some() {
                    self.palette_key(event_loop, &event);
                    return;
                }
                if self.renaming.is_some() {
                    self.rename_key(&event);
                    return;
                }
                // Any key closes a message box.
                if self.message.take().is_some() {
                    self.running.as_ref().unwrap().window.request_redraw();
                    return;
                }
                let key = KeyInput {
                    logical: &event.logical_key,
                    physical: event.physical_key,
                    text: event.text.as_deref(),
                    mods: self.mods,
                };
                let action =
                    key_chord(&key).and_then(|chord| self.config.config.keys.get(&chord).cloned());
                if self.handle_copy_keys(&event, action.as_ref()) {
                    return;
                }
                if let Some(action) = action {
                    self.run_action(event_loop, action);
                    return;
                }
                let Some(session) = self.running.as_ref().and_then(Running::session) else {
                    return;
                };
                let app_cursor =
                    session.with_term(|term| term.mode().contains(TermMode::APP_CURSOR));
                if let Some(bytes) = encode_key(&key, app_cursor) {
                    // Typing goes back to the bottom and removes the selection.
                    session.with_term_mut(|term, sticky| {
                        term.scroll_display(Scroll::Bottom);
                        if sticky.is_active() {
                            sticky.set(term, None);
                        }
                    });
                    session.write(bytes);
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.mouse_moved(position.x, position.y),
            WindowEvent::CursorLeft { .. } => {
                if self.mouse.tab_hover != Hit::None {
                    self.mouse.tab_hover = Hit::None;
                    self.running.as_ref().unwrap().window.request_redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse_button(event_loop, state, button)
            }
            WindowEvent::MouseWheel { delta, .. } => self.mouse_wheel(delta),
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }
}

/// Writes the shell integration scripts and returns the PowerShell one.
fn shell_script_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os(if cfg!(windows) {
        "LOCALAPPDATA"
    } else {
        "XDG_DATA_HOME"
    })
    .map(std::path::PathBuf::from)
    .or_else(|| fterm_config::profiles::home_dir().map(|h| h.join(".local").join("share")))
    .unwrap_or_else(std::env::temp_dir);
    match install_scripts(&base.join("fterm").join("shell")) {
        Ok(path) => Some(path),
        Err(err) => {
            tracing::warn!("cannot write the shell integration scripts: {err}");
            None
        }
    }
}

/// The profiles from the config, or the ones found on this computer.
fn profiles_for(config: &LoadedConfig) -> Vec<Profile> {
    if config.config.profiles.is_empty() {
        detect_profiles(cfg!(windows), which, std::path::Path::exists)
    } else {
        config.config.profiles.clone()
    }
}

fn palette_for(config: &LoadedConfig) -> Palette {
    let c = &config.config.colors;
    let rgb = |c: Option<fterm_config::colors::Rgb>| {
        c.map(|c| fterm_term::alacritty_terminal::vte::ansi::Rgb {
            r: c.r,
            g: c.g,
            b: c.b,
        })
    };
    Palette::with_colors(&ColorOverrides {
        background: rgb(c.background),
        foreground: rgb(c.foreground),
        cursor: rgb(c.cursor),
        selection: rgb(c.selection),
        ansi: c.ansi.map(rgb),
        bright: c.bright.map(rgb),
    })
}

fn edge_of(action: BuiltinAction) -> Edge {
    use BuiltinAction as A;
    match action {
        A::FocusLeft | A::ResizeLeft => Edge::Left,
        A::FocusRight | A::ResizeRight => Edge::Right,
        A::FocusUp | A::ResizeUp => Edge::Up,
        _ => Edge::Down,
    }
}

/// An error as lines for the message box (long lines are cut into parts).
fn error_lines(error: &str) -> Vec<String> {
    const WIDTH: usize = 90;
    let mut lines = vec!["Config error:".to_owned(), String::new()];
    for line in error.lines() {
        let chars: Vec<char> = line.chars().collect();
        for part in chars.chunks(WIDTH) {
            lines.push(part.iter().collect());
        }
    }
    lines.push(String::new());
    lines.push(
        "The old config is still used. Fix the file and save it. Any key closes this.".to_owned(),
    );
    lines
}

/// Cell size in whole pixels, for the pty (some apps ask for it).
fn cell_px(renderer: &Renderer) -> (u16, u16) {
    let cell = renderer.cell();
    (cell.width as u16, cell.height as u16)
}
