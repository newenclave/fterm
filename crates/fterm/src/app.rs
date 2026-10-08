//! The app: it gets window events from winit, sends keys to the active pane, and draws the window.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fterm_config::keys::{Action, BuiltinAction, SpawnWhere};
use fterm_config::load::{
    AgentIn, AgentOut, ApiCall, CloseIn, DockPlace, HistoryIn, LoadedConfig, NotifyIn, NotifyOut,
    OsNotify, SAMPLE_CONFIG, ToastPosition, config_path, load_file,
};
use fterm_config::profiles::{Profile, detect_profiles, launch_command, path_extension, which};
use fterm_history::{CommandFilter, CommandRecord, History, Limits, now_ms, should_save};
use fterm_mux::{Closed, Direction, Edge, Mux, PaneId, Rect, TabId};
use fterm_render::Renderer;
use fterm_render::builtin::BrailleStyle;
use fterm_render::dock::{
    DockHit, DockLayout, DockRow, DockSide, DockView, dock_hit, layout_dock, split_area,
};
use fterm_render::tabbar::{Hit, TabBarInput, bar_height, corner_rect, hit, layout_tabs};
use fterm_render::theme::UiColors;
use fterm_render::toasts::{
    Corner, ToastLevel, ToastView, avoid_cursor, close_rect, layout_toasts,
};
use fterm_term::alacritty_terminal::grid::{Dimensions, Scroll};
use fterm_term::alacritty_terminal::index::{Point, Side};
use fterm_term::alacritty_terminal::selection::{Selection, SelectionType};
use fterm_term::alacritty_terminal::term::TermMode;
use fterm_term::colors::Palette;
use fterm_term::copy_mode::{self, CopyAction, CopyResult};
use fterm_term::input::typed_input;
use fterm_term::links::url_at;
use fterm_term::osc::OscEvent;
use fterm_term::process::{display_name, is_shell, running_children};
use fterm_term::session::{Session, SessionOptions, TermEvent};
use fterm_term::shell::{
    ShellEvent, ShellState, bash_args, install_scripts, is_bash, is_powershell, is_zsh,
    powershell_args, wsl_args, wsl_cwd, zsh_env,
};
use fterm_term::size::GridSize;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

use crate::agent::{AgentKind, AgentState, badge_color, notification_for, tab_badge};
use crate::clipboard::{Clipboard, paste_bytes};
use crate::close::{CloseDecision, WindowState, decide, decide_again};
use crate::gpu::Gpu;
use crate::history_popup::{
    HistoryPopup, PopupKind, PopupRow, cd_command, command_rows, dir_rows, replace_input,
};
use crate::input::{KeyInput, copy_mode_action, encode_key, key_chord, physical_letter};
use crate::mouse::{
    ClickCounter, GridGeometry, ReportButton, ReportKind, ReportMods, Wheel, autoscroll_lines,
    encode_mouse,
};
use crate::notify::{Center, Level, Source, human_duration};
use crate::palette::{PaletteItem, PaletteState, VISIBLE_ROWS};
use crate::panels::{
    AgentEntry, Dock, EventFilter, PanelKind, agent_rows, event_rows, level_color, toast_level,
};

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
    /// A theme file changed on disk.
    ThemeFilesChanged,
    /// A request from an API client (`ftermctl`, `ftermctl mcp`, a script).
    Api(crate::api::ApiRequest),
    /// An API client closed its connection.
    ApiGone(fterm_api::server::ClientId),
    /// A piece of an AI answer (request id, event).
    Ai(u64, fterm_ai::Event),
    /// A piece of a "text to command" answer.
    AiCommand(u64, fterm_ai::Event),
}

mod ai_calls;
mod api_calls;
mod review_calls;
mod session_calls;

/// The last command of a pane (for the API).
struct LastCommand {
    command: Option<String>,
    exit: Option<i32>,
    took_ms: u64,
}

/// One terminal pane: a session and the title that its app set.
struct Pane {
    session: Session,
    app_title: Option<String>,
    /// The folder and the commands, from shell integration.
    shell: ShellState,
    /// What the agent in this pane (for example Claude Code) does now.
    agent: Option<AgentState>,
    /// The agent of this pane sent its state with hooks: then fterm does not guess it.
    agent_hooks: bool,
    /// The last command that ended (from shell integration).
    last_command: Option<LastCommand>,
    /// API clients may read and type into it (with the user's yes). `toggle_remote_control` changes it.
    remote: bool,
    /// The API client that opened this pane (it may use it without a question).
    opened_by: Option<fterm_api::server::ClientId>,
    /// The profile that started it (for restoring the session).
    profile: Option<String>,
    /// A restored pane: the command that ran in it, for its first prompt (true = run it).
    rerun: Option<(String, bool)>,
    /// A restored pane: the lines of its old text, to scroll them into view at its first prompt.
    intro_lines: usize,
    /// A Braille scene pane (Phase 11): its canvas. The pane's grid shows it; there is no program.
    scene: Option<std::sync::Mutex<fterm_scene::Scene>>,
    /// A Review tab: the review that the grid shows; keys go to it.
    review: Option<review_calls::ReviewPane>,
    /// The colors of its programs may fit the theme (`harmonize = false` in its profile: no).
    harmonize: bool,
    /// Its programs may change the palette (from its profile; `None` = the value of the config).
    palette_changes: Option<bool>,
}

/// Everything that exists only while the window is open.
struct Running {
    window: Arc<Window>,
    gpu: Gpu,
    renderer: Renderer,
    mux: Mux,
    panes: HashMap<PaneId, Pane>,
    /// The dock with the service panels.
    dock: Dock,
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

    /// The space for the panes of a tab: the window below the tab bar, without the dock.
    fn tab_area(&self) -> Rect {
        let area = self.below_bar();
        if self.dock.open {
            split_area(area, self.dock.side, self.dock.ratio, self.renderer.cell()).0
        } else {
            area
        }
    }

    /// The place of the dock, when it is open.
    fn dock_rect(&self) -> Option<Rect> {
        self.dock.open.then(|| {
            split_area(
                self.below_bar(),
                self.dock.side,
                self.dock.ratio,
                self.renderer.cell(),
            )
            .1
        })
    }

    /// The window below the tab bar.
    fn below_bar(&self) -> Rect {
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
        // A minimized window has no size: the programs keep theirs (else they draw for one cell).
        let minimized = self.window.is_minimized().unwrap_or(false);
        if !crate::gpu::panes_follow(self.window.inner_size(), minimized) {
            return;
        }
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
                    if let Some(scene) = &pane.scene {
                        // The picture is drawn again for the new size (zoom, a split, the window).
                        let mut scene = scene.lock().unwrap();
                        scene.set_aspect(dot_aspect(self.renderer.cell()));
                        scene.resize(size.columns, size.rows);
                        pane.session.feed(&fterm_scene::render(scene.canvas()));
                    }
                }
            }
        }
    }

    /// The agent badge of each tab: the most important agent state of its panes.
    fn tab_badges(&self) -> Vec<Option<AgentKind>> {
        self.mux
            .tabs()
            .iter()
            .map(|tab| {
                let panes = tab.layout.panes();
                tab_badge(
                    panes
                        .iter()
                        .filter_map(|id| self.panes.get(id)?.agent.as_ref()),
                )
            })
            .collect()
    }

    /// Panes on the screen now (in the active tab).
    fn visible_panes(&self) -> Vec<PaneId> {
        self.mux
            .pane_rects(self.tab_area())
            .into_iter()
            .map(|(id, _)| id)
            .collect()
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
    /// The user drags the dock edge.
    dragging_dock: bool,
}

/// What the close question is about.
#[derive(Clone, Copy)]
enum CloseTarget {
    Tab(TabId),
    Pane(PaneId),
    /// The whole window (its ×, Alt+F4, or the last tab).
    Window,
}

/// One line of the command palette to draw: (label, key, is it selected).
type PaletteRow = (String, String, bool);

/// The hint cache: (typed text, folder, history changes, the hint).
type HintCache = (String, Option<String>, u64, Option<String>);

/// A list to draw like the palette: the command palette, or a history popup.
struct ListView {
    query: String,
    rows: Vec<PaletteRow>,
    title: String,
    footer: &'static str,
    bad: Vec<bool>,
}

/// "Add the fterm hooks to Claude Code?": the file and its new text.
struct HooksQuestion {
    path: std::path::PathBuf,
    text: String,
    lines: Vec<String>,
}

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
    hooks_question: Option<HooksQuestion>,
    config: LoadedConfig,
    config_path: std::path::PathBuf,
    /// Profiles from the config, or the ones that fterm found.
    profiles: Vec<Profile>,
    /// Watches the config file. Kept here so it does not stop.
    _watcher: Option<notify::RecommendedWatcher>,
    /// The theme in use, and its UI colors.
    theme: fterm_config::theme::Theme,
    ui: UiColors,
    /// A theme chosen while fterm runs (the palette or the API).
    theme_override: Option<crate::themes::Override>,
    /// The system is in dark mode (for `theme = { light = ..., dark = ... }`).
    system_dark: bool,
    /// `toggle_original_colors`: the colors that programs chose, as they are.
    original_colors: bool,
    /// `harmonize_more` / `harmonize_less`: a strength to try, until the config changes.
    harmonize_live: Option<f32>,
    /// The notification that shows the value being tried (it changes, not a new one per step).
    harmonize_note: Option<u64>,
    /// A message box (for example, an error in the config). Any key closes it.
    message: Option<Vec<String>>,
    /// Editors save in several steps: we load the config a moment after the last change.
    reload_at: Option<Instant>,
    /// When to read the theme files again (after they changed on disk).
    theme_reload_at: Option<Instant>,
    /// The PowerShell shell integration script (written at start).
    shell_script: Option<std::path::PathBuf>,
    /// The command palette, when it is open.
    palette: Option<PaletteState>,
    /// All notifications and the toasts on the screen.
    center: Center,
    /// The folder and command history (`None` when it is off, or its folder cannot be made).
    history: Option<History>,
    /// The history popup (Alt+F8 / Alt+F12), when it is open.
    history_popup: Option<HistoryPopup>,
    /// The next new pane starts in this folder (a choice in the folder popup).
    /// The old text of a restored pane, for the next `spawn_pane` (Phase 9b).
    spawn_intro: Vec<u8>,
    spawn_cwd: Option<String>,
    /// The last hint: (typed text, folder, history changes) -> the rest of the command.
    hint_cache: RefCell<Option<HintCache>>,
    /// Grows when a command is saved, so the hint cache knows it is old.
    history_changes: u64,
    /// When `on_close_window` last stopped the close. A second × soon after it asks instead.
    close_stopped_at: Option<Instant>,
    /// The API server of this window, its instance file, and who its clients are.
    api_server: Option<fterm_api::server::Server>,
    api_registration: Option<fterm_api::discovery::Registration>,
    api_clients: HashMap<fterm_api::server::ClientId, api_calls::ApiClient>,
    /// Client names that the user allowed "always" (in this session).
    api_always: std::collections::HashSet<String>,
    /// Access questions (one per client); the first one is on the screen.
    api_questions: std::collections::VecDeque<api_calls::ApiAsk>,
    /// `wait_for` calls that wait.
    waits: Vec<api_calls::Wait>,
    /// Screenshots that wait for the next frame.
    shots: Vec<api_calls::PendingShot>,
    /// `ai_ask` calls that wait for the end of the answer.
    ai_waits: Vec<api_calls::AiWait>,
    /// `review` calls that wait for the user.
    reviews: Vec<review_calls::PendingReview>,
    /// Messages between agents, by pane.
    inbox: crate::inbox::Inbox,
    /// The AI chat, the flag that stops its running answer, and the API key prompt.
    ai: crate::ai_chat::Session,
    ai_stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    key_prompt: Option<crate::ai_chat::InputBox>,
    /// "Text to command" that waits for its answer.
    pending_command: Option<ai_calls::PendingCommand>,
    /// The last session (and its file), while fterm asks "Restore the last session?".
    restore_offer: Option<crate::session_state::Entry>,
    /// The name box of "Save session as…".
    name_prompt: Option<crate::ai_chat::InputBox>,
    /// When the session was saved last.
    session_saved_at: Option<Instant>,
    command_next_id: u64,
    /// The Events panel was on the screen (and fterm in front) in the last frame.
    /// When it goes away, its events count as read.
    events_seen: bool,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let arg = fterm_config::load::config_arg(&args).unwrap_or_else(|err| {
            tracing::warn!("{err}");
            None
        });
        let config_path = config_path(arg.as_deref());
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
        // `data_dir`: the history, the sessions, and the shell scripts in one folder (a portable fterm).
        let data_dir = config.config.data_dir.as_deref().map(|dir| {
            crate::paths::resolve(
                &config_path,
                dir,
                fterm_config::profiles::home_dir().as_deref(),
            )
        });
        if let Some(dir) = &data_dir {
            tracing::info!(folder = %dir.display(), "the data folder");
        }
        crate::paths::set_data_dir(data_dir);
        let profiles = profiles_for(&config);
        let mut app = Self {
            proxy,
            running: None,
            mods: ModifiersState::empty(),
            focused: true,
            mouse: MouseState::default(),
            clipboard: Clipboard::new(),
            title_message_until: None,
            renaming: None,
            close_question: None,
            hooks_question: None,
            config,
            config_path,
            profiles,
            _watcher: None,
            theme: fterm_config::theme::Theme::default(),
            ui: UiColors::default(),
            theme_override: None,
            system_dark: true,
            original_colors: false,
            harmonize_live: None,
            harmonize_note: None,
            message,
            reload_at: None,
            theme_reload_at: None,
            events_seen: false,
            history: None,
            history_popup: None,
            spawn_intro: Vec::new(),
            spawn_cwd: None,
            hint_cache: RefCell::new(None),
            history_changes: 0,
            close_stopped_at: None,
            api_server: None,
            api_registration: None,
            api_clients: HashMap::new(),
            api_always: std::collections::HashSet::new(),
            api_questions: std::collections::VecDeque::new(),
            waits: Vec::new(),
            shots: Vec::new(),
            ai_waits: Vec::new(),
            reviews: Vec::new(),
            inbox: crate::inbox::Inbox::default(),
            ai: crate::ai_chat::Session::default(),
            ai_stop: None,
            key_prompt: None,
            pending_command: None,
            restore_offer: None,
            name_prompt: None,
            session_saved_at: None,
            command_next_id: 0,
            center: Center::new(4, true),
            shell_script: shell_script_path(),
            palette: None,
        };
        app.apply_notification_config();
        Self::open_history(&mut app.history, &app.config.config.history);
        if let Some(lines) = app.message.take() {
            // A config error at start: a toast, not a box in the way.
            let text = lines.join("\n");
            app.notify(None, "Config error", &text, Level::Error, Source::App);
        }
        app
    }

    fn apply_notification_config(&mut self) {
        let n = &self.config.config.notifications;
        self.center.max_toasts = n.max_visible;
        self.center.toasts_on = n.toasts.is_some();
    }

    /// Makes a notification: the Lua filter, then the history and a toast, and maybe an OS notification.
    fn notify(
        &mut self,
        pane: Option<PaneId>,
        title: &str,
        body: &str,
        level: Level,
        source: Source,
    ) {
        self.api_event(
            "notification",
            serde_json::json!({
                "pane": pane.map(|p| p.0),
                "title": title,
                "body": body,
                "level": level.name(),
                "source": source.name(),
            }),
        );
        let input = NotifyIn {
            title: title.to_owned(),
            body: body.to_owned(),
            level: level.name().to_owned(),
            source: source.name().to_owned(),
            pane: pane.map(|p| p.0),
        };
        let (title, body, level, os, toast) = match self.config.filter_notification(&input) {
            Ok(NotifyOut::Drop) => return,
            Ok(NotifyOut::Keep {
                title,
                body,
                level: name,
                os,
                toast,
            }) => (
                title,
                body,
                Level::from_name(&name).unwrap_or(level),
                os,
                toast,
            ),
            Err(err) => {
                tracing::warn!("on_notification error: {err}");
                (input.title, input.body, level, None, None)
            }
        };
        let now = Instant::now();
        self.center.push(
            now,
            pane,
            &title,
            &body,
            level,
            source,
            toast.unwrap_or(true),
        );

        let n = &self.config.config.notifications;
        let level_ok = n.os_levels.is_empty() || n.os_levels.iter().any(|l| l == level.name());
        let send_os = os.unwrap_or(match n.os {
            OsNotify::Never => false,
            OsNotify::Always => level_ok,
            OsNotify::WhenUnfocused => level_ok && !self.focused,
        });
        if send_os {
            let summary = if title.is_empty() {
                "fterm".to_owned()
            } else {
                title.clone()
            };
            let text = body.clone();
            // The OS call can be slow, so it runs on its own thread.
            std::thread::spawn(move || {
                if let Err(err) = notify_rust::Notification::new()
                    .appname("fterm")
                    .summary(&summary)
                    .body(&text)
                    .show()
                {
                    tracing::warn!("cannot show an OS notification: {err}");
                }
            });
        }
        if let Some(running) = &self.running {
            if level == Level::Attention && n.flash && !self.focused {
                running
                    .window
                    .request_user_attention(Some(winit::window::UserAttentionType::Informational));
            }
            running.window.request_redraw();
        }
    }

    /// A command ended. If it ran long and you did not see it, fterm tells you.
    /// An agent in a pane says what it does now (`OSC 777;fterm-agent;<state>;<message>`).
    /// An agent with no hooks: its tool runs in the pane (the command from shell integration), and its
    /// window title shows what it does (a spinner = working, `✳` = done). With no shell integration,
    /// only the title says it. Hooks, when the agent has them, are more exact and win.
    fn detect_agent(&mut self, event_loop: &ActiveEventLoop, pane: PaneId) {
        use crate::agent::{agent_program, title_state, title_topic};
        let Some(p) = self.running.as_ref().and_then(|r| r.panes.get(&pane)) else {
            return;
        };
        if p.agent_hooks {
            return;
        }
        let command = p.shell.running_command();
        let tool = command.and_then(agent_program);
        let title = p.app_title.clone().unwrap_or_default();
        let shown = title_state(&title);
        let auto = p.agent.as_ref().is_some_and(|a| a.auto);
        // At the prompt of a shell with integration no tool runs, whatever the old title says.
        if tool.is_none() && (command.is_some() || p.shell.at_prompt() || shown.is_none()) {
            // Not an agent (any more).
            if auto {
                self.agent_state(event_loop, pane, "idle", "");
            }
            return;
        }
        let kind = shown.unwrap_or(AgentKind::Working);
        let topic = match title_topic(&title) {
            "" => tool.unwrap_or_default().to_owned(),
            topic => topic.to_owned(),
        };
        self.agent_state(event_loop, pane, kind.name(), &topic);
        if let Some(a) = self
            .running
            .as_mut()
            .and_then(|r| r.panes.get_mut(&pane))
            .and_then(|p| p.agent.as_mut())
        {
            a.auto = true;
        }
    }

    fn agent_state(
        &mut self,
        event_loop: &ActiveEventLoop,
        pane: PaneId,
        state: &str,
        message: &str,
    ) {
        let Some(kind) = AgentKind::parse(state) else {
            tracing::debug!(pane = pane.0, state, "unknown agent state");
            return;
        };
        let focused = self.focused;
        let Some(running) = self.running.as_mut() else {
            return;
        };
        let seen = focused && running.visible_panes().contains(&pane);
        let tab = running
            .mux
            .tabs()
            .iter()
            .position(|t| t.layout.contains(pane))
            .and_then(|i| running.tab_titles().get(i).cloned())
            .unwrap_or_default();
        let Some(p) = running.panes.get_mut(&pane) else {
            return;
        };
        let previous = p.agent.as_ref().map(|old| old.kind);
        if previous == kind && p.agent.as_ref().is_none_or(|old| old.message == message) {
            return;
        }
        p.agent = kind.map(|kind| AgentState {
            kind,
            message: message.to_owned(),
            since: match &p.agent {
                Some(old) if previous == Some(kind) => old.since,
                _ => Instant::now(),
            },
            seen,
            auto: false,
        });
        running.window.request_redraw();
        if previous == kind {
            // Only the message changed.
            return;
        }
        tracing::debug!(pane = pane.0, ?previous, ?kind, "agent state");
        self.update_window_title();
        let state = kind.map_or("idle", AgentKind::name);
        let data = serde_json::json!({
            "event": "agent_state",
            "pane": pane.0,
            "state": state,
            "message": message,
        });
        self.api_event("agent_state", data.clone());
        self.resolve_waits(pane, crate::waits::Happening::Agent(state), data);
        let input = AgentIn {
            pane: pane.0,
            state: kind.map_or("idle", AgentKind::name).to_owned(),
            previous: previous.map(|k| k.name().to_owned()),
            message: message.to_owned(),
            name: tab.clone(),
        };
        let out = match self.config.on_agent(&input) {
            Ok(out) => out,
            Err(err) => {
                tracing::warn!("on_agent: {err}");
                self.notify(None, "Lua error", &err, Level::Error, Source::App);
                AgentOut {
                    notify: true,
                    calls: Vec::new(),
                }
            }
        };
        for call in out.calls {
            self.run_api_call(event_loop, call);
        }
        if out.notify
            && !seen
            && let Some(kind) = kind
            && let Some((title, body, level)) = notification_for(kind, message, &tab)
        {
            self.notify(Some(pane), &title, &body, level, Source::Agent);
        }
    }

    fn command_done(&mut self, pane: PaneId, exit: Option<i32>, took: Duration) {
        let limit = self.config.config.notifications.long_command;
        if limit <= 0.0 || took.as_secs_f64() < limit {
            return;
        }
        let Some(running) = &self.running else {
            return;
        };
        let visible = running
            .mux
            .pane_rects(running.tab_area())
            .iter()
            .any(|(p, _)| *p == pane);
        if visible && self.focused {
            return;
        }
        let tab = running
            .mux
            .tabs()
            .iter()
            .position(|t| t.layout.contains(pane))
            .and_then(|i| running.tab_titles().get(i).cloned())
            .unwrap_or_default();
        let (title, level) = match exit {
            Some(0) | None => ("Command finished".to_owned(), Level::Success),
            Some(code) => (format!("Command failed (exit {code})"), Level::Error),
        };
        let body = format!("{tab} · took {}", human_duration(took));
        self.notify(Some(pane), &title, &body, level, Source::Command);
    }

    /// The toasts on the screen and their places (the same for drawing and for the mouse).
    /// Opens the history when it is on (and closes it when it is off). Changed limits open it again.
    fn open_history(history: &mut Option<History>, config: &fterm_config::load::HistoryConfig) {
        let limits = Limits {
            commands: config.commands,
            dirs: config.dirs,
        };
        if !config.enabled {
            *history = None;
            return;
        }
        if history.as_ref().is_some_and(|h| h.limits() == limits) {
            return;
        }
        let Some(folder) = crate::paths::folder(
            std::env::var_os("FTERM_HISTORY_DIR").map(std::path::PathBuf::from),
            crate::paths::data_dir().as_deref(),
            "history",
            fterm_history::default_folder(),
        ) else {
            tracing::warn!("no folder for the history");
            return;
        };
        match History::open(&folder, limits) {
            Ok((h, problems)) => {
                for problem in problems {
                    tracing::warn!("history: {problem}");
                }
                tracing::info!(folder = %folder.display(), "history loaded");
                *history = Some(h);
            }
            Err(err) => {
                tracing::warn!(folder = %folder.display(), "cannot open the history: {err}")
            }
        }
    }

    /// A command ended: save it in the history (if the config and `on_history` say yes).
    fn save_command(
        &mut self,
        pane: PaneId,
        command: Option<String>,
        cwd: Option<String>,
        exit: Option<i32>,
        took: Duration,
    ) {
        let Some(cmd) = command else {
            return;
        };
        let settings = &self.config.config.history;
        if self.history.is_none() || !should_save(&cmd, settings.ignore_space) {
            return;
        }
        let shell = self
            .running
            .as_ref()
            .and_then(|r| r.panes.get(&pane))
            .map(|p| display_name(p.session.program()).to_owned())
            .unwrap_or_default();
        let input = HistoryIn {
            cmd,
            cwd: cwd.clone(),
            exit,
            shell: shell.clone(),
        };
        let cmd = match self.config.on_history(&input) {
            Ok(Some(cmd)) if should_save(&cmd, false) => cmd,
            Ok(_) => return,
            Err(err) => {
                tracing::warn!("on_history: {err}");
                self.notify(None, "Lua error", &err, Level::Error, Source::App);
                return;
            }
        };
        let took_ms = took.as_millis() as u64;
        let record = CommandRecord {
            cmd,
            cwd,
            exit,
            start: now_ms().saturating_sub(took_ms),
            took_ms,
            shell: (!shell.is_empty()).then_some(shell),
        };
        if let Some(history) = &mut self.history
            && let Err(err) = history.add_command(record)
        {
            tracing::warn!("cannot save the command: {err}");
        }
        self.history_changes += 1;
    }

    /// The panel tabs: "Events 3", "Agents 2".
    fn dock_tabs(&self) -> Vec<String> {
        let unread = self.center.unread();
        let agents = self.running.as_ref().map_or(0, |r| {
            r.panes.values().filter(|p| p.agent.is_some()).count()
        });
        PanelKind::ALL
            .iter()
            .map(|kind| {
                let count = match kind {
                    PanelKind::Events => unread,
                    PanelKind::Agents => agents,
                    PanelKind::Ai => 0,
                };
                if count > 0 {
                    format!("{} {count}", kind.label())
                } else {
                    kind.label().to_owned()
                }
            })
            .collect()
    }

    /// The rows of the active panel, and the pane of each row.
    fn dock_rows(&self) -> Vec<(DockRow, Option<PaneId>)> {
        let Some(running) = &self.running else {
            return Vec::new();
        };
        let now = Instant::now();
        match running.dock.active {
            // The AI panel draws its chat, not rows.
            PanelKind::Ai => Vec::new(),
            PanelKind::Events => {
                event_rows(self.center.history(), running.dock.filter, now, &self.ui)
            }
            PanelKind::Agents => {
                let titles = running.tab_titles();
                let mut entries = Vec::new();
                for (tab, title) in running.mux.tabs().iter().zip(&titles) {
                    for pane in tab.layout.panes() {
                        if let Some(state) = running.panes.get(&pane).and_then(|p| p.agent.as_ref())
                        {
                            entries.push(AgentEntry {
                                pane,
                                name: title,
                                state,
                                messages: self.inbox.unread(pane),
                            });
                        }
                    }
                }
                agent_rows(&entries, now, &self.ui)
            }
        }
    }

    fn dock_layout(&self) -> Option<DockLayout> {
        let running = self.running.as_ref()?;
        let rect = running.dock_rect()?;
        Some(layout_dock(
            rect,
            running.dock.side,
            &self.dock_tabs(),
            running.renderer.cell(),
        ))
    }

    /// The panes moved (the dock opened, closed, or changed its size).
    fn dock_changed(&mut self) {
        if let Some(running) = &self.running {
            running.resize_all_panes();
            running.window.request_redraw();
        }
    }

    /// Enter or a click on a row: go to its pane, and give the keyboard back to the terminal.
    fn open_dock_row(&mut self, index: usize) {
        let pane = self.dock_rows().get(index).and_then(|(_, pane)| *pane);
        if let Some(pane) = pane
            && let Some(running) = &mut self.running
            && running.panes.contains_key(&pane)
        {
            running.dock.focused = false;
            self.go_to_pane(pane);
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// The event of row `index` in the Events panel.
    fn event_of_row(&self, index: usize) -> Option<u64> {
        let filter = self.running.as_ref()?.dock.filter;
        crate::panels::filtered_events(self.center.history(), filter)
            .get(index)
            .map(|n| n.id)
    }

    /// The lines of the open full text, and how many fit. `None` = no full text is open
    /// (or its event is gone from the history).
    fn reader_lines(&self) -> Option<(Vec<fterm_render::dock::ChatLine>, usize)> {
        let running = self.running.as_ref()?;
        if running.dock.active != PanelKind::Events {
            return None;
        }
        let id = running.dock.reading()?;
        let n = self.center.history().find(|n| n.id == id)?;
        let layout = self.dock_layout()?;
        let cell = running.renderer.cell();
        let width = ((layout.list.width / cell.width) as usize).saturating_sub(2);
        let place = n.pane.and_then(|pane| {
            let tab = running
                .mux
                .tabs()
                .iter()
                .position(|t| t.layout.contains(pane))?;
            Some(format!("tab {}, pane {}", tab + 1, pane.0))
        });
        let lines = crate::panels::event_lines(n, place.as_deref(), Instant::now(), width);
        Some((lines, fterm_render::dock::reader_rows(&layout, cell)))
    }

    /// A key while the full text of an event is open.
    fn reader_key(&mut self, event: &KeyEvent) {
        let Some((lines, visible)) = self.reader_lines() else {
            if let Some(running) = &mut self.running {
                running.dock.close_reader();
            }
            return;
        };
        let ctrl = self.mods.control_key();
        let Some(running) = &mut self.running else {
            return;
        };
        let dock = &mut running.dock;
        let (total, page) = (lines.len(), visible.max(1) as isize);
        match &event.logical_key {
            Key::Named(NamedKey::ArrowUp) => dock.scroll_reader(-1, total, visible),
            Key::Named(NamedKey::ArrowDown) => dock.scroll_reader(1, total, visible),
            Key::Named(NamedKey::PageUp) => dock.scroll_reader(-page, total, visible),
            Key::Named(NamedKey::PageDown | NamedKey::Space) => {
                dock.scroll_reader(page, total, visible)
            }
            Key::Named(NamedKey::Home) => dock.scroll_reader(isize::MIN / 2, total, visible),
            Key::Named(NamedKey::End) => dock.scroll_reader(isize::MAX / 2, total, visible),
            Key::Named(NamedKey::Escape | NamedKey::ArrowLeft | NamedKey::Backspace) => {
                dock.close_reader()
            }
            Key::Named(NamedKey::Tab) => dock.next_panel(),
            Key::Named(NamedKey::Enter) => {
                let index = dock.selected();
                dock.close_reader();
                return self.open_dock_row(index);
            }
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("c") => {
                let id = dock.reading();
                let text = self.center.history().find(|n| Some(n.id) == id).map(|n| {
                    if n.body.trim().is_empty() {
                        n.title.clone()
                    } else {
                        format!("{}\n\n{}", n.title, n.body)
                    }
                });
                if let Some(text) = text {
                    self.copy_text(text);
                }
                return;
            }
            _ => {}
        }
        running.window.request_redraw();
    }

    /// A key while the dock has the keyboard. The keys never go to the terminal.
    fn dock_key(&mut self, event: &KeyEvent) {
        if self
            .running
            .as_ref()
            .is_some_and(|r| r.dock.active == PanelKind::Events && r.dock.reading().is_some())
        {
            return self.reader_key(event);
        }
        if self
            .running
            .as_ref()
            .is_some_and(|r| r.dock.active == PanelKind::Ai)
        {
            return self.ai_key(event);
        }
        let rows = self.dock_rows().len();
        let visible = self.dock_layout().map_or(1, |l| l.visible_rows().max(1));
        let Some(running) = &mut self.running else {
            return;
        };
        let dock = &mut running.dock;
        let page = visible as isize;
        match &event.logical_key {
            Key::Named(NamedKey::ArrowUp) => dock.move_by(-1, rows, visible),
            Key::Named(NamedKey::ArrowDown) => dock.move_by(1, rows, visible),
            Key::Named(NamedKey::PageUp) => dock.move_by(-page, rows, visible),
            Key::Named(NamedKey::PageDown) => dock.move_by(page, rows, visible),
            Key::Named(NamedKey::Home) => dock.select(0, rows, visible),
            Key::Named(NamedKey::End) => dock.select(rows.saturating_sub(1), rows, visible),
            Key::Named(NamedKey::Space | NamedKey::ArrowRight)
                if dock.active == PanelKind::Events && rows > 0 =>
            {
                let index = dock.selected();
                if let Some(id) = self.event_of_row(index)
                    && let Some(running) = &mut self.running
                {
                    running.dock.open_reader(id);
                    running.window.request_redraw();
                }
                return;
            }
            Key::Named(NamedKey::Tab | NamedKey::ArrowLeft | NamedKey::ArrowRight) => {
                dock.next_panel()
            }
            Key::Named(NamedKey::Escape) => dock.focused = false,
            Key::Named(NamedKey::Enter) => {
                let index = dock.selected();
                return self.open_dock_row(index);
            }
            Key::Character(c) if dock.active == PanelKind::Events => match c.as_str() {
                "f" | "F" => {
                    dock.filter = dock.filter.next();
                    dock.select(0, rows, visible);
                }
                "m" | "M" => self.center.mark_all_read(),
                _ => {}
            },
            _ => {}
        }
        running.window.request_redraw();
    }

    /// A mouse button over the dock. Returns `true` when the dock took it.
    fn dock_mouse(&mut self, state: ElementState, button: MouseButton) -> bool {
        if self.mouse.dragging_dock {
            if state == ElementState::Released && button == MouseButton::Left {
                self.mouse.dragging_dock = false;
            }
            return true;
        }
        let Some(layout) = self.dock_layout() else {
            return false;
        };
        let rows = self.dock_rows().len();
        let (x, y) = (self.mouse.position.0 as f32, self.mouse.position.1 as f32);
        let Some(running) = &mut self.running else {
            return false;
        };
        let hit = dock_hit(&layout, running.dock.scroll(), rows, x, y);
        if hit == DockHit::Outside {
            // A press in the terminal gives the keyboard back to it.
            if state == ElementState::Pressed && running.dock.focused {
                running.dock.focused = false;
                running.window.request_redraw();
            }
            return false;
        }
        if state != ElementState::Pressed || button != MouseButton::Left {
            return true;
        }
        let visible = layout.visible_rows().max(1);
        match hit {
            DockHit::Tab(i) => {
                running.dock.close_reader();
                if let Some(kind) = PanelKind::ALL.get(i) {
                    running.dock.active = *kind;
                }
                running.dock.focused = true;
            }
            // The full text of an event covers the rows: a click there only takes the keyboard.
            DockHit::Row(_) if running.dock.reading().is_some() => running.dock.focused = true,
            DockHit::Row(i) => {
                running.dock.select(i, rows, visible);
                self.open_dock_row(i);
                return true;
            }
            DockHit::Grab => self.mouse.dragging_dock = true,
            DockHit::Inside => running.dock.focused = true,
            DockHit::Outside => {}
        }
        running.window.request_redraw();
        true
    }

    /// The mouse moved: drag the dock edge, or show the resize arrow over it.
    /// Returns `true` when the dock took the move.
    fn dock_mouse_moved(&mut self, x: f32, y: f32) -> bool {
        let layout = self.dock_layout();
        let Some(running) = &mut self.running else {
            return false;
        };
        if self.mouse.dragging_dock {
            let area = running.below_bar();
            let ratio = match running.dock.side {
                DockSide::Right => (area.x + area.width - x) / area.width,
                DockSide::Left => (x - area.x) / area.width,
                DockSide::Bottom => (area.y + area.height - y) / area.height,
            };
            running.dock.ratio = ratio.clamp(0.1, 0.9);
            self.dock_changed();
            return true;
        }
        let Some(layout) = layout else {
            return false;
        };
        if !layout.rect.contains(x, y) {
            return false;
        }
        let icon = if layout.grab.contains(x, y) {
            match layout.side {
                DockSide::Bottom => CursorIcon::RowResize,
                _ => CursorIcon::ColResize,
            }
        } else {
            CursorIcon::Default
        };
        running.window.set_cursor(icon);
        true
    }

    /// The text in the tab bar corner: the unread events, when the Events panel is not on the screen.
    fn tab_bar_corner(&self) -> Option<String> {
        let running = self.running.as_ref()?;
        let unread = self.center.unread();
        (unread > 0 && !running.dock.showing(PanelKind::Events)).then(|| unread.to_string())
    }

    /// The width that the tabs can use (the corner text is not for tabs).
    fn tabs_width(&self, width: f32, cell: fterm_render::font::CellMetrics) -> f32 {
        match self.tab_bar_corner() {
            Some(text) => width - corner_rect(&text, width, cell).width,
            None => width,
        }
    }

    fn toast_layout(&self) -> Vec<(u64, Rect)> {
        let Some(running) = &self.running else {
            return Vec::new();
        };
        let Some(position) = self.config.config.notifications.toasts else {
            return Vec::new();
        };
        let ids: Vec<u64> = self.center.toasts().iter().map(|t| t.id).collect();
        if ids.is_empty() {
            return Vec::new();
        }
        let cell = running.renderer.cell();
        let area = running.tab_area();
        let mut corner = match position {
            ToastPosition::BottomRight => Corner::BottomRight,
            ToastPosition::TopRight => Corner::TopRight,
            ToastPosition::BottomLeft => Corner::BottomLeft,
            ToastPosition::TopLeft => Corner::TopLeft,
            ToastPosition::Bottom => Corner::Bottom,
        };
        let rects = layout_toasts(ids.len(), area, corner, cell);
        // Do not cover the text cursor of the active pane.
        if let (Some(first), Some(last), Some(cursor)) =
            (rects.first(), rects.last(), self.cursor_rect())
        {
            let top = first.y.min(last.y);
            let bottom = (first.y + first.height).max(last.y + last.height);
            let stack = Rect::new(first.x, top, first.width, bottom - top);
            corner = avoid_cursor(corner, stack, cursor);
        }
        ids.into_iter()
            .zip(layout_toasts(
                self.center.toasts().len(),
                area,
                corner,
                cell,
            ))
            .collect()
    }

    /// The text cursor of the active pane, in window pixels.
    fn cursor_rect(&self) -> Option<Rect> {
        let running = self.running.as_ref()?;
        let session = running.session()?;
        let cell = running.renderer.cell();
        let area = running.pane_area();
        let (line, column) = session.with_term(|term| {
            let point = term.grid().cursor.point;
            (
                point.line.0 + term.grid().display_offset() as i32,
                point.column.0,
            )
        });
        let padding = running.renderer.padding();
        Some(Rect::new(
            area.x + padding + column as f32 * cell.width,
            area.y + padding + line.max(0) as f32 * cell.height,
            cell.width,
            cell.height,
        ))
    }

    /// Goes to the tab and the pane (for example, after a click on a toast).
    fn go_to_pane(&mut self, pane: PaneId) {
        let Some(running) = &mut self.running else {
            return;
        };
        if let Some(index) = running
            .mux
            .tabs()
            .iter()
            .position(|t| t.layout.contains(pane))
        {
            running.mux.select(index);
            running.mux.focus(pane);
            self.tab_changed();
        }
    }

    /// The toast under the mouse: (id, is it the close button).
    fn toast_under_mouse(&self) -> Option<(u64, bool)> {
        let (x, y) = (self.mouse.position.0 as f32, self.mouse.position.1 as f32);
        let cell = self.running.as_ref()?.renderer.cell();
        self.toast_layout()
            .into_iter()
            .rev()
            .find(|(_, rect)| rect.contains(x, y))
            .map(|(id, rect)| (id, close_rect(rect, cell).contains(x, y)))
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
        let theme_dirs: Vec<std::path::PathBuf> = self
            .theme_dirs()
            .into_iter()
            .filter(|d| d.is_dir())
            .collect();
        let watched_themes = theme_dirs.clone();
        let proxy = self.proxy.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else {
                return;
            };
            if !(event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove()) {
                return;
            }
            let ours = event
                .paths
                .iter()
                .any(|path| path.file_name() == file.as_deref());
            let theme = event.paths.iter().any(|path| {
                path.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("json"))
                    && path
                        .parent()
                        .is_some_and(|d| watched_themes.iter().any(|t| t == d))
            });
            if ours && !event.kind.is_remove() {
                let _ = proxy.send_event(UserEvent::ConfigChanged);
            } else if theme {
                let _ = proxy.send_event(UserEvent::ThemeFilesChanged);
            }
        });
        match watcher {
            Ok(mut watcher) => {
                use notify::Watcher;
                if let Err(err) = watcher.watch(&dir, notify::RecursiveMode::NonRecursive) {
                    tracing::warn!("cannot watch the config: {err}");
                }
                for themes in &theme_dirs {
                    if let Err(err) = watcher.watch(themes, notify::RecursiveMode::NonRecursive) {
                        tracing::warn!("cannot watch {}: {err}", themes.display());
                    }
                }
                self._watcher = Some(watcher);
            }
            Err(err) => tracing::warn!("cannot watch the config: {err}"),
        }
    }

    /// The terminal palette: the theme, with `colors` of the config on top.
    pub(crate) fn palette(&self) -> Palette {
        let mut palette = Palette::with_colors(&crate::themes::palette_colors(
            &self.theme,
            &self.config.config.colors,
        ));
        palette.set_harmonize(self.harmonize());
        palette
    }

    /// How the colors of programs fit the theme now: the theme, the config, and a value being tried.
    fn harmonize(&self) -> fterm_term::harmonize::Settings {
        let mut settings =
            crate::themes::harmonize_settings(&self.theme, &self.config.config.harmonize);
        if let Some(strength) = self.harmonize_live {
            settings.strength = strength;
        }
        settings
    }

    /// `harmonize_more` / `harmonize_less`: one step, at once, with a toast that says the value.
    fn step_harmonize(&mut self, dir: i32) {
        let strength = crate::themes::step_strength(self.harmonize().strength, dir);
        self.harmonize_live = Some(strength);
        self.original_colors = false;
        self.apply_theme();
        let body = format!(
            "harmonize = {{ strength = {strength:.1} }} in fterm.lua keeps it. Ctrl+Shift+] more, Ctrl+Shift+[ less."
        );
        let title = format!("Harmonize: {strength:.1}");
        // One toast that says the new value, not a new event for every step.
        let now = Instant::now();
        let shown = self
            .harmonize_note
            .is_some_and(|id| self.center.update(id, now, &title, &body));
        if !shown {
            self.notify(None, &title, &body, Level::Info, Source::App);
            // The newest one, if the Lua filter did not drop or change it.
            self.harmonize_note = self
                .center
                .history()
                .next()
                .filter(|n| n.title == title)
                .map(|n| n.id);
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// The folders with theme files.
    pub(crate) fn theme_dirs(&self) -> Vec<std::path::PathBuf> {
        crate::themes::theme_dirs(&self.config_path, crate::paths::data_dir().as_deref())
    }

    /// Finds the theme to use (the config, the system mode, a theme chosen while fterm runs).
    /// An error keeps the last good theme and shows a toast. Gives true when the theme is in use.
    pub(crate) fn load_theme(&mut self) -> bool {
        let dirs = self.theme_dirs();
        let picked = crate::themes::pick(
            &self.config.config.theme,
            self.theme_override.as_ref(),
            self.system_dark,
            |name| fterm_config::theme::find_theme(name, &dirs),
        );
        match picked {
            Ok(theme) => {
                self.ui = crate::themes::ui_colors(&theme);
                self.theme = theme;
                true
            }
            Err(err) => {
                tracing::warn!("theme: {err}");
                self.notify(None, "Theme error", &err, Level::Error, Source::App);
                false
            }
        }
    }

    /// `choose_theme`: the list of themes.
    fn open_theme_popup(&mut self) {
        self.palette = None;
        self.history_popup = Some(HistoryPopup::new(
            PopupKind::Themes,
            Vec::new(),
            String::new(),
            None,
        ));
        self.refresh_history_popup();
    }

    /// Uses a theme until the config changes or fterm closes. On an error the old theme stays.
    pub(crate) fn use_theme(&mut self, theme: crate::themes::Override) -> Result<String, String> {
        let old = self.theme_override.replace(theme);
        if self.load_theme() {
            self.apply_theme();
            Ok(self.theme.name.clone())
        } else {
            self.theme_override = old;
            Err("the theme was not changed".to_owned())
        }
    }

    /// Gives the theme to the renderer.
    pub(crate) fn apply_theme(&mut self) {
        self.redraw_reviews();
        let palette = self.palette();
        let ui = self.ui;
        if let Some(running) = &mut self.running {
            running.renderer.set_palette(palette);
            running.renderer.set_ui(ui);
            running.window.request_redraw();
        }
    }

    /// Reads the config file again. An error keeps the old config and shows a message.
    fn reload_config(&mut self) {
        match load_file(&self.config_path) {
            Ok(config) => {
                tracing::info!("config reloaded");
                let gpu_changed = config.config.gpu != self.config.config.gpu;
                self.config = config;
                self.profiles = profiles_for(&self.config);
                self.apply_notification_config();
                self.theme_override = None;
                self.harmonize_live = None;
                self.load_theme();
                self.apply_config();
                // Say which file it read: with FTERM_CONFIG it is not always the one you think.
                let path = self.config_path.display().to_string();
                self.notify(None, "Config reloaded", &path, Level::Info, Source::App);
                if gpu_changed {
                    // The GPU is made once, at start.
                    self.notify(
                        None,
                        "Restart fterm for the new gpu settings",
                        "gpu.backend and gpu.power are used when fterm starts.",
                        Level::Info,
                        Source::App,
                    );
                }
            }
            Err(err) => {
                tracing::warn!("config error: {err}");
                let text = format!("{err}\nThe old config is still used.");
                self.notify(None, "Config error", &text, Level::Error, Source::App);
            }
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Gives the config to the renderer: font, padding, colors, Braille.
    fn apply_config(&mut self) {
        let palette = self.palette();
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
        running.renderer.set_palette(palette);
        running.renderer.set_ui(self.ui);
        let braille = match config.braille_style {
            fterm_config::load::BrailleStyle::Pixels => BrailleStyle::Pixels,
            fterm_config::load::BrailleStyle::Dots => BrailleStyle::Dots,
        };
        running
            .renderer
            .set_braille_style(running.gpu.device(), braille);
        running.dock.side = dock_side(config.panels.dock);
        Self::open_history(&mut self.history, &config.history);
        running.dock.ratio = config.panels.size;
        running.resize_all_panes();
        running.window.request_redraw();
    }

    /// The profile by name, else the default profile, else the first one.
    /// Windows, or the WSL distro of the active pane (for the history of folders and commands).
    fn active_world(&self) -> crate::history_popup::World {
        let distro = self
            .running
            .as_ref()
            .and_then(Running::active_pane)
            .and_then(|pane| pane.profile.as_deref())
            .and_then(|name| self.profiles.iter().find(|p| p.name == name))
            .and_then(|p| p.wsl.clone());
        match distro {
            Some(distro) => crate::history_popup::World::Wsl(distro),
            None => crate::history_popup::World::Windows,
        }
    }

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
            .with_inner_size(LogicalSize::new(1024.0, 640.0))
            .with_window_icon(crate::window_icon::window_icon(32));
        // The big icon for the taskbar and Alt+Tab.
        #[cfg(windows)]
        let attributes = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_taskbar_icon(crate::window_icon::window_icon(256))
        };
        let window = Arc::new(event_loop.create_window(attributes)?);
        // IME: input methods for Chinese, Japanese, Korean, and others.
        window.set_ime_allowed(true);
        let gpu = pollster::block_on(Gpu::new(
            window.clone(),
            event_loop.owned_display_handle(),
            self.config.config.gpu,
        ))?;
        let scale = window.scale_factor() as f32;
        let config = &self.config.config;
        let mut renderer = Renderer::new(
            gpu.device(),
            gpu.format(),
            config.font_size * scale,
            config.padding * scale,
        )?;
        renderer.set_palette(self.palette());
        renderer.set_ui(self.ui);
        if config.braille_style == fterm_config::load::BrailleStyle::Dots {
            renderer.set_braille_style(gpu.device(), BrailleStyle::Dots);
        }
        Ok(Running {
            window,
            gpu,
            renderer,
            mux: Mux::default(),
            panes: HashMap::new(),
            dock: Dock::new(
                dock_side(config.panels.dock),
                config.panels.size,
                config.panels.open.as_deref().and_then(PanelKind::from_name),
            ),
        })
    }

    /// `install_claude_skill`: writes the fterm skill for Claude Code, but never over a file of the user.
    fn install_claude_skill(&mut self) {
        use fterm_api::guide::{SkillPlan, skill_md, skill_path, skill_plan};
        let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(std::path::PathBuf::from);
        let home = fterm_config::profiles::home_dir();
        let Some(path) = skill_path(config_dir.as_deref(), home.as_deref()) else {
            return self.notify(None, "No home folder", "", Level::Error, Source::App);
        };
        let shown = path.display().to_string();
        let existing = std::fs::read_to_string(&path).ok();
        match skill_plan(existing.as_deref()) {
            SkillPlan::UpToDate => self.notify(
                None,
                "The fterm skill is up to date",
                &shown,
                Level::Info,
                Source::App,
            ),
            SkillPlan::Foreign => {
                self.copy_text(skill_md());
                self.notify(
                    None,
                    "A different fterm skill is there",
                    &format!("{shown} is not from fterm, so it is not changed. The fterm skill is in the clipboard."),
                    Level::Warning,
                    Source::App,
                );
            }
            SkillPlan::Write => {
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&path, skill_md()));
                match written {
                    Ok(()) => self.notify(
                        None,
                        "The fterm skill for Claude Code is installed",
                        &format!("{shown}. Claude Code uses it in new sessions."),
                        Level::Success,
                        Source::App,
                    ),
                    Err(err) => self.notify(
                        None,
                        "Cannot write the skill",
                        &format!("{shown}: {err}"),
                        Level::Error,
                        Source::App,
                    ),
                }
            }
        }
    }

    /// A new Braille scene pane with this grid size (no program; the API draws into it).
    fn spawn_scene(&mut self, size: GridSize) -> PaneId {
        let running = self.running.as_mut().expect("the window is open");
        let id = running.mux.new_pane_id();
        let session = Session::scene(size);
        let mut scene = fterm_scene::Scene::new(size.columns, size.rows);
        scene.set_aspect(dot_aspect(running.renderer.cell()));
        session.feed(&fterm_scene::render(scene.canvas()));
        running.panes.insert(
            id,
            Pane {
                session,
                app_title: Some("scene".to_owned()),
                shell: ShellState::default(),
                agent: None,
                agent_hooks: false,
                last_command: None,
                remote: true,
                opened_by: None,
                profile: None,
                rerun: None,
                intro_lines: 0,
                scene: Some(std::sync::Mutex::new(scene)),
                review: None,
                harmonize: true,
                palette_changes: None,
            },
        );
        tracing::info!(
            pane = id.0,
            columns = size.columns,
            rows = size.rows,
            "new scene"
        );
        id
    }

    /// Starts a profile (or the default profile) for a new pane with this grid size.
    fn spawn_pane(&mut self, size: GridSize, profile: Option<&str>) -> anyhow::Result<PaneId> {
        // A new pane starts in the folder of the active pane (or the saved one), unless the profile
        // has its own folder. From a WSL pane it is the same distro: its folder is a Linux one.
        let (raw_cwd, active_wsl) = {
            let active = self.running.as_ref().and_then(Running::active_pane);
            let raw_cwd = self
                .spawn_cwd
                .clone()
                .or_else(|| active.and_then(|p| p.shell.cwd.clone()));
            let active_wsl = active.and_then(|p| p.profile.clone()).filter(|name| {
                self.profiles
                    .iter()
                    .any(|p| p.name == *name && p.wsl.is_some())
            });
            (raw_cwd, active_wsl)
        };
        let profile = profile.map(str::to_owned).or(active_wsl);
        let profile = profile.as_deref();
        let active_cwd = raw_cwd
            .clone()
            .map(std::path::PathBuf::from)
            .filter(|dir| dir.is_dir());
        let profile_name = self.profile(profile).map(|p| p.name.clone());
        let harmonize = self.profile(profile).is_none_or(|p| p.harmonize);
        let palette_changes = self.profile(profile).and_then(|p| p.palette_changes);
        let options = match self.profile(profile) {
            Some(profile) => {
                let (program, mut args) = launch_command(&profile, cfg!(windows), path_extension);
                if self.config.config.shell_integration
                    && is_powershell(&program)
                    && let Some(script) = &self.shell_script
                {
                    args = powershell_args(&args, script);
                }
                // bash and zsh (Linux, macOS, Git Bash) get the shell integration too.
                let mut env = profile.env.clone();
                if self.config.config.shell_integration
                    && profile.wsl.is_none()
                    && let Some(dir) = self.shell_script.as_deref().and_then(|s| s.parent())
                {
                    if is_bash(&program) && native_bash(&program) {
                        if let Some(new) = bash_args(&args, dir) {
                            args = new;
                        }
                    } else if is_zsh(&program) {
                        let user = env
                            .iter()
                            .find(|(k, _)| k == "ZDOTDIR")
                            .map(|(_, v)| v.clone())
                            .or_else(|| std::env::var("ZDOTDIR").ok());
                        env.retain(|(k, _)| k != "ZDOTDIR");
                        env.extend(zsh_env(user.as_deref(), dir));
                    }
                }
                // A WSL distro with the args that fterm made: bash with the shell integration.
                if let Some(distro) = &profile.wsl
                    && args == wsl_args(distro, "~", None)
                {
                    let script = self
                        .shell_script
                        .as_ref()
                        .filter(|_| self.config.config.shell_integration)
                        .map(|ps1| ps1.with_file_name("fterm-login.bash").display().to_string());
                    let cwd = wsl_cwd(distro, raw_cwd.as_deref());
                    args = wsl_args(distro, &cwd, script.as_deref());
                }
                SessionOptions {
                    program: Some(program),
                    args,
                    cwd: profile
                        .cwd
                        .clone()
                        .filter(|dir| dir.is_dir())
                        .or(active_cwd),
                    env,
                    scrollback: self.config.config.scrollback,
                    intro: Vec::new(),
                    record: None,
                }
            }
            None => SessionOptions {
                scrollback: self.config.config.scrollback,
                ..SessionOptions::default()
            },
        };
        let mut options = SessionOptions {
            intro: std::mem::take(&mut self.spawn_intro),
            ..options
        };
        let socket = self.api_socket();
        if let Some(dir) = self.spawn_cwd.take() {
            options.cwd = Some(std::path::PathBuf::from(dir)).filter(|dir| dir.is_dir());
        }
        let running = self.running.as_mut().expect("the window is open");
        let id = running.mux.new_pane_id();
        // FTERM_RECORD: a recording of the pane, for finding bugs of the output.
        if let Some(dir) = std::env::var_os("FTERM_RECORD").filter(|d| !d.is_empty()) {
            let file = crate::env::record_file(
                std::path::Path::new(&dir),
                std::process::id(),
                id.0,
                now_ms(),
            );
            options.record = Some(file);
        }
        // Hooks of tools in the pane (for example Claude Code) can use it.
        options
            .env
            .push(("FTERM_PANE_ID".to_owned(), id.0.to_string()));
        if let Some(socket) = socket {
            options.env.push(("FTERM_SOCKET".to_owned(), socket));
        }
        // WSL panes get the fterm vars too (Windows vars do not go into WSL by themselves).
        if cfg!(windows) {
            let wslenv = options
                .env
                .iter()
                .rev()
                .find(|(k, _)| k.eq_ignore_ascii_case("WSLENV"))
                .map(|(_, v)| v.clone())
                .or_else(|| std::env::var("WSLENV").ok());
            options.env.push((
                "WSLENV".to_owned(),
                crate::env::wslenv_with(wslenv.as_deref()),
            ));
        }
        // The folder of fterm.exe (with ftermctl.exe) at the end of PATH, so `ftermctl mcp` works in panes.
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.display().to_string()))
        {
            let base = options
                .env
                .iter()
                .rev()
                .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
                .map(|(_, v)| v.clone())
                .or_else(|| std::env::var("PATH").ok());
            let path = crate::env::path_with(base.as_deref(), &dir, cfg!(windows));
            options.env.push(("PATH".to_owned(), path));
        }
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
        if let Some(server) = &self.api_server {
            server.broadcast("pane_opened", serde_json::json!({ "pane": id.0 }));
        }
        running.panes.insert(
            id,
            Pane {
                session,
                app_title: None,
                shell: ShellState::default(),
                agent: None,
                agent_hooks: false,
                last_command: None,
                remote: true,
                opened_by: None,
                profile: profile_name,
                rerun: None,
                intro_lines: 0,
                scene: None,
                review: None,
                harmonize,
                palette_changes,
            },
        );
        Ok(id)
    }

    /// Starts a profile in a new tab, after the active tab.
    fn new_tab(&mut self, profile: Option<&str>) -> anyhow::Result<PaneId> {
        let running = self.running.as_ref().expect("the window is open");
        let size = running.grid_for(running.tab_area());
        let id = self.spawn_pane(size, profile)?;
        let color = self.profile(profile).and_then(|p| p.tab_color);
        let running = self.running.as_mut().unwrap();
        let tab = running.mux.new_tab(id);
        running.mux.set_color(tab, color);
        self.tab_changed();
        Ok(id)
    }

    /// Splits the active pane. The new pane gets the focus.
    fn split(&mut self, direction: Direction, profile: Option<&str>) -> anyhow::Result<()> {
        self.split_with(direction, |app, size| app.spawn_pane(size, profile))
    }

    /// Splits the active pane; `make` starts the new pane with its grid size.
    fn split_with(
        &mut self,
        direction: Direction,
        make: impl FnOnce(&mut Self, GridSize) -> anyhow::Result<PaneId>,
    ) -> anyhow::Result<()> {
        let running = self.running.as_ref().expect("the window is open");
        let half = running.pane_area();
        let half = match direction {
            Direction::Right => Rect::new(half.x, half.y, half.width / 2.0, half.height),
            Direction::Down => Rect::new(half.x, half.y, half.width, half.height / 2.0),
        };
        let size = running.grid_for(half);
        let id = make(self, size)?;
        let running = self.running.as_mut().unwrap();
        running.mux.split_active(id, direction);
        running.resize_all_panes();
        self.tab_changed();
        Ok(())
    }

    /// Closes one pane now (no question). Returns false when it was the last pane of the last tab.
    fn close_pane_now(&mut self, pane: PaneId) -> bool {
        self.api_pane_closed(pane);
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
    /// The programs that run in a pane: its own program when it is not a shell (for example `claude`),
    /// else the programs that run in its shell.
    fn pane_programs(running: &Running, pane: PaneId) -> Vec<String> {
        let Some(p) = running.panes.get(&pane) else {
            return Vec::new();
        };
        if p.scene.is_some() {
            // A scene runs nothing.
            return Vec::new();
        }
        if !is_shell(p.session.program()) {
            let file = p
                .session
                .program()
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default();
            return vec![display_name(file).to_owned()];
        }
        p.session
            .pid()
            .map(running_children)
            .unwrap_or_default()
            .iter()
            .map(|name| display_name(name).to_owned())
            .collect()
    }

    /// What runs in the whole window (for the close question and `on_close_window`).
    fn window_state(&self) -> WindowState {
        let Some(running) = &self.running else {
            return WindowState::default();
        };
        let titles = running.tab_titles();
        let mut state = WindowState {
            tabs: running.mux.tabs().len(),
            ..WindowState::default()
        };
        for (i, tab) in running.mux.tabs().iter().enumerate() {
            for pane in tab.layout.panes() {
                state.panes += 1;
                for program in Self::pane_programs(running, pane) {
                    if !state.running.contains(&(i + 1, program.clone())) {
                        state.running.push((i + 1, program));
                    }
                }
                if let Some(agent) = running.panes.get(&pane).and_then(|p| p.agent.as_ref()) {
                    let name = titles.get(i).cloned().unwrap_or_default();
                    state
                        .agents
                        .push((i + 1, name, agent.kind.name().to_owned()));
                }
            }
        }
        state
    }

    /// The window ×: close at once, ask first, or do nothing (by `confirm_close` and `on_close_window`).
    fn close_window(&mut self, event_loop: &ActiveEventLoop) {
        let state = self.window_state();
        let input = CloseIn {
            tabs: state.tabs,
            panes: state.panes,
            running: state.running.clone(),
            agents: state.agents.clone(),
        };
        let hook = match self.config.on_close_window(&input) {
            Ok(answer) => answer,
            Err(err) => {
                tracing::warn!("on_close_window: {err}");
                self.notify(None, "Lua error", &err, Level::Error, Source::App);
                None
            }
        };
        let again = self
            .close_stopped_at
            .take()
            .is_some_and(|at| at.elapsed() < Duration::from_secs(5));
        let rule = if again { decide_again } else { decide };
        match rule(&state, self.config.config.confirm_close, hook) {
            CloseDecision::Now => event_loop.exit(),
            CloseDecision::Ask(lines) => {
                self.close_question = Some(CloseQuestion {
                    target: CloseTarget::Window,
                    lines,
                });
                if let Some(running) = &self.running {
                    running.window.request_redraw();
                }
            }
            CloseDecision::Stop => {
                self.close_stopped_at = Some(Instant::now());
                self.notify(
                    None,
                    "fterm stays open",
                    "Closing was stopped by on_close_window in your config. Press × again to close anyway.",
                    Level::Info,
                    Source::App,
                );
            }
        }
    }

    fn close(&mut self, event_loop: &ActiveEventLoop, target: CloseTarget) {
        let Some(running) = &self.running else {
            return;
        };
        // Closing the last tab (or its last pane) closes the window: the window rule asks.
        let last_tab = running.mux.tabs().len() == 1;
        let last = match target {
            CloseTarget::Window => true,
            CloseTarget::Tab(_) => last_tab,
            CloseTarget::Pane(_) => last_tab && running.panes.len() == 1,
        };
        if last {
            return self.close_window(event_loop);
        }
        let panes = match target {
            CloseTarget::Tab(tab) => {
                let Some(tab_info) = running.mux.tabs().iter().find(|t| t.id == tab) else {
                    return;
                };
                tab_info.layout.panes()
            }
            CloseTarget::Pane(pane) => vec![pane],
            CloseTarget::Window => return,
        };
        let mut programs: Vec<String> = Vec::new();
        for pane in panes {
            for name in Self::pane_programs(running, pane) {
                if !programs.contains(&name) {
                    programs.push(name);
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
            CloseTarget::Pane(_) | CloseTarget::Window => "Close this pane?",
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

    /// Closes it. `false` = nothing is left: the window closes.
    fn close_now(&mut self, target: CloseTarget) -> bool {
        match target {
            CloseTarget::Tab(tab) => self.close_tab_now(tab),
            CloseTarget::Pane(pane) => self.close_pane_now(pane),
            CloseTarget::Window => false,
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
        running.window.set_title(&self.window_title());
    }

    /// `[tab/tabs] title — ⏳ 1 waiting`, or what `window_title` in the config says.
    fn window_title(&self) -> String {
        let Some(running) = &self.running else {
            return "fterm".to_owned();
        };
        let active = running.mux.active_index();
        let titles = running.tab_titles();
        let badges = running.tab_badges();
        let others = || {
            badges
                .iter()
                .enumerate()
                .filter(move |(i, _)| *i != active)
                .filter_map(|(_, b)| *b)
        };
        let info = crate::title::TitleInfo {
            tab: active + 1,
            tabs: running.mux.tabs().len(),
            title: titles.get(active).cloned().unwrap_or_default(),
            agent: running
                .active_pane()
                .and_then(|p| p.agent.as_ref())
                .map(|a| a.kind.name().to_owned()),
            waiting: others().filter(|k| *k == AgentKind::Waiting).count(),
            failed: others().filter(|k| *k == AgentKind::Error).count(),
        };
        let default = crate::title::default_title(&info);
        let input = fterm_config::load::TitleIn {
            tab: info.tab,
            tabs: info.tabs,
            title: info.title,
            agent: info.agent,
            waiting: info.waiting,
            failed: info.failed,
            default: default.clone(),
        };
        match self.config.window_title(&input) {
            Ok(Some(title)) => title,
            Ok(None) => default,
            Err(err) => {
                tracing::warn!("window_title: {err}");
                default
            }
        }
    }

    /// Shows a short message in the window title.
    fn title_message(&mut self, message: &str) {
        let title = self.window_title();
        if let Some(running) = &self.running {
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
                    self.notify(None, "Lua error", &err, Level::Error, Source::App);
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
            ApiCall::Notify { title, body, level } => {
                let pane = self.running.as_ref().and_then(|r| r.mux.active_pane());
                let level = Level::from_name(&level).unwrap_or(Level::Info);
                self.notify(pane, &title, &body, level, Source::Lua);
            }
            ApiCall::Copy(text) => self.copy_text(text),
            ApiCall::Action(builtin) => self.run_builtin(event_loop, builtin),
            ApiCall::SetTabColor { pane, color } => {
                let pane = pane
                    .map(PaneId)
                    .or_else(|| self.running.as_ref().and_then(|r| r.mux.active_pane()));
                if let Some(pane) = pane {
                    self.set_tab_color(pane, color);
                }
            }
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
            self.notify(
                None,
                "Cannot start",
                &format!("{err:#}"),
                Level::Error,
                Source::App,
            );
        }
    }

    fn run_builtin(&mut self, event_loop: &ActiveEventLoop, action: BuiltinAction) {
        use BuiltinAction as A;
        if action == A::SetAiKey {
            return self.start_key_prompt();
        }
        if action == A::ExplainError {
            return self.explain_error();
        }
        if action == A::TextToCommand {
            return self.text_to_command();
        }
        if action == A::RestoreSession {
            return self.restore_last_session();
        }
        if action == A::InstallClaudeHooks {
            return self.ask_install_hooks();
        }
        if action == A::ChooseTheme {
            return self.open_theme_popup();
        }
        if action == A::Sessions {
            return self.open_sessions_popup();
        }
        if action == A::NewScene {
            if let Err(err) =
                self.split_with(Direction::Right, |app, size| Ok(app.spawn_scene(size)))
            {
                tracing::error!("cannot open a scene: {err:#}");
            }
            return;
        }
        if action == A::SaveSessionAs {
            return self.start_name_prompt();
        }
        if action == A::AskAiSelection {
            return self.ask_ai_selection();
        }
        // Ctrl+C in the AI panel copies the last command (or the last answer).
        if action == A::Copy
            && self
                .running
                .as_ref()
                .is_some_and(|r| r.dock.focused && r.dock.showing(PanelKind::Ai))
        {
            return self.ai_copy();
        }
        // Ctrl+V in the AI panel goes into its input, not into the terminal.
        if action == A::Paste
            && self
                .running
                .as_ref()
                .is_some_and(|r| r.dock.focused && r.dock.showing(PanelKind::Ai))
        {
            return self.ai_paste();
        }
        let dock_action = matches!(
            action,
            A::ToggleDock | A::PanelEvents | A::PanelAgents | A::PanelAi | A::FocusDock
        );
        if matches!(action, A::HistoryCommands | A::HistoryDirs) {
            let kind = if action == A::HistoryCommands {
                PopupKind::Commands
            } else {
                PopupKind::Dirs
            };
            return self.open_history_popup(kind);
        }
        if dock_action {
            if let Some(running) = &mut self.running {
                match action {
                    A::ToggleDock => running.dock.toggle(),
                    A::PanelEvents => running.dock.show(PanelKind::Events),
                    A::PanelAgents => running.dock.show(PanelKind::Agents),
                    A::PanelAi => running.dock.show(PanelKind::Ai),
                    _ => running.dock.toggle_focus(),
                }
            }
            if let Some(running) = &self.running {
                tracing::debug!(?action, open = running.dock.open, focused = running.dock.focused, active = ?running.dock.active, "dock action");
            }
            return self.dock_changed();
        }
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
            A::HarmonizeMore => return self.step_harmonize(1),
            A::HarmonizeLess => return self.step_harmonize(-1),
            A::ToggleOriginalColors => {
                self.original_colors = !self.original_colors;
                let (title, body) = if self.original_colors {
                    ("Original colors", "Programs show their own colors.")
                } else {
                    (
                        "Theme colors",
                        "The colors of programs fit the theme (harmonize).",
                    )
                };
                if let Some(running) = &self.running {
                    running.window.request_redraw();
                }
                return self.notify(None, title, body, Level::Info, Source::App);
            }
            A::ToggleFullscreen => {
                // Borderless on the monitor of the window: no frame and no title bar.
                let full = running.window.fullscreen().is_some();
                running
                    .window
                    .set_fullscreen((!full).then_some(winit::window::Fullscreen::Borderless(None)));
                return;
            }
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
            A::ToggleDock | A::PanelEvents | A::PanelAgents | A::FocusDock => {}
            A::HistoryCommands | A::HistoryDirs => {}
            A::PanelAi
            | A::SetAiKey
            | A::ExplainError
            | A::AskAiSelection
            | A::TextToCommand
            | A::RestoreSession
            | A::Sessions
            | A::SaveSessionAs
            | A::ChooseTheme
            | A::InstallClaudeHooks
            | A::NewScene => {}
            A::ToggleRemoteControl => {
                let Some(pane) = running.mux.active_pane() else {
                    return;
                };
                let Some(p) = running.panes.get_mut(&pane) else {
                    return;
                };
                p.remote = !p.remote;
                let (title, body) = if p.remote {
                    (
                        "Remote control is on",
                        "Programs can ask to read and type into this pane.",
                    )
                } else {
                    (
                        "Remote control is off",
                        "No program can read or type into this pane.",
                    )
                };
                self.notify(Some(pane), title, body, Level::Info, Source::App);
                return;
            }
            A::InstallClaudeSkill => return self.install_claude_skill(),
            A::CopyClaudeHooks => {
                self.copy_text(crate::agent::CLAUDE_HOOKS.to_owned());
                self.notify(
                    None,
                    "Claude Code hooks copied",
                    "Put them into ~/.claude/settings.json (see docs/CLAUDE.md).",
                    Level::Success,
                    Source::App,
                );
                return;
            }
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
    /// The rows of a history popup, with its filters.
    fn history_rows(
        &self,
        kind: PopupKind,
        only_here: bool,
        only_ok: bool,
        here: Option<&str>,
    ) -> Vec<PopupRow> {
        if kind == PopupKind::Sessions {
            return self.session_rows();
        }
        if kind == PopupKind::Themes {
            let names = fterm_config::theme::list_themes(&self.theme_dirs());
            return crate::themes::theme_rows(&names, &self.theme.name);
        }
        let Some(history) = &self.history else {
            return Vec::new();
        };
        let now = now_ms();
        match kind {
            PopupKind::Sessions | PopupKind::Themes => Vec::new(),
            PopupKind::Commands => {
                let filter = CommandFilter {
                    cwd: if only_here {
                        here.map(str::to_owned)
                    } else {
                        None
                    },
                    only_ok,
                };
                let world = self.active_world();
                let commands: Vec<_> = history
                    .commands(&filter)
                    .into_iter()
                    .filter(|e| world.has_command(e.cwd.as_deref()))
                    .collect();
                command_rows(&commands, now)
            }
            PopupKind::Dirs => {
                let world = self.active_world();
                let dirs: Vec<_> = history
                    .dirs(now)
                    .into_iter()
                    .filter(|e| world.has(&e.dir))
                    .collect();
                dir_rows(&dirs, now, |dir| {
                    std::path::Path::new(&world.host_path(dir)).is_dir()
                })
            }
        }
    }

    /// Opens the command or folder popup. The query starts with the typed text of the active pane.
    fn open_history_popup(&mut self, kind: PopupKind) {
        let Some(history) = &mut self.history else {
            self.notify(
                None,
                "The history is off",
                "Turn it on with history = { enabled = true } in the config.",
                Level::Info,
                Source::App,
            );
            return;
        };
        for problem in history.refresh() {
            tracing::warn!("history: {problem}");
        }
        let (typed, here) = self
            .running
            .as_ref()
            .and_then(Running::active_pane)
            .map(|pane| {
                let typed = pane
                    .shell
                    .input_start()
                    .and_then(|start| pane.session.with_term(|term| typed_input(term, start)))
                    .map(|input| input.text)
                    .unwrap_or_default();
                (typed, pane.shell.cwd.clone())
            })
            .unwrap_or_default();
        let rows = self.history_rows(kind, false, false, here.as_deref());
        // Folders: the typed text is not a folder name, so it does not filter them.
        let query = if kind == PopupKind::Commands {
            typed.clone()
        } else {
            String::new()
        };
        let mut popup = HistoryPopup::new(kind, rows, query, here);
        popup.typed = typed;
        self.palette = None;
        self.history_popup = Some(popup);
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Builds the rows again (after a filter, a pin, or a forget). The query stays.
    fn refresh_history_popup(&mut self) {
        let Some(popup) = &self.history_popup else {
            return;
        };
        let rows = self.history_rows(
            popup.kind,
            popup.only_here,
            popup.only_ok,
            popup.here.as_deref(),
        );
        if let Some(popup) = &mut self.history_popup {
            popup.set_rows(rows);
        }
    }

    /// Enter in a history popup. `run` = Shift+Enter, `split` = Ctrl+Enter.
    fn accept_history_popup(&mut self, run: bool, split: bool) {
        let Some(popup) = self.history_popup.take() else {
            return;
        };
        let Some(row) = popup.selected().cloned() else {
            return;
        };
        let busy = self
            .running
            .as_ref()
            .and_then(Running::active_pane)
            .is_some_and(|pane| pane.shell.is_running());
        match popup.kind {
            PopupKind::Sessions => return self.restore_entry(&row.key),
            PopupKind::Themes => {
                // An error was shown as a toast.
                let _ = self.use_theme(crate::themes::Override::Named(row.key));
                return;
            }
            PopupKind::Dirs if run || split => {
                self.spawn_cwd = Some(row.text.clone());
                let place = if split {
                    SpawnWhere::SplitRight
                } else {
                    SpawnWhere::Tab
                };
                self.spawn(None, place);
                return;
            }
            _ if busy => {
                // Never type into a running program: copy instead.
                self.copy_text(row.text.clone());
                self.notify(
                    None,
                    "A program runs in this pane",
                    "The text is copied. Paste it where you need it.",
                    Level::Info,
                    Source::App,
                );
            }
            PopupKind::Commands => self.type_into_prompt(&popup.typed, &row.text, run),
            PopupKind::Dirs => {
                let program = self
                    .running
                    .as_ref()
                    .and_then(Running::active_pane)
                    .map(|pane| pane.session.program().to_owned())
                    .unwrap_or_default();
                let command = cd_command(&program, &row.text);
                self.type_into_prompt(&popup.typed, &command, true);
            }
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// The grey hint for the active pane now: (the rest of the command, cursor column, cursor line).
    /// Only with `history.hints`, at the prompt, at the bottom of the history, with the cursor at the end
    /// of the typed text. When the shell draws its own hint, the cursor is not at the end, so ours is not shown.
    fn current_hint(&self) -> Option<(String, usize, usize)> {
        if !self.config.config.history.hints {
            return None;
        }
        let history = self.history.as_ref()?;
        let pane = self.running.as_ref()?.active_pane()?;
        if !pane.shell.at_prompt() {
            return None;
        }
        let start = pane.shell.input_start()?;
        let (input, cursor) = pane.session.with_term(|term| {
            if term.grid().display_offset() != 0 || term.mode().contains(TermMode::ALT_SCREEN) {
                return None;
            }
            let point = term.grid().cursor.point;
            let line = usize::try_from(point.line.0).ok()?;
            Some((typed_input(term, start)?, (point.column.0, line)))
        })?;
        if !input.cursor_at_end || input.text.trim().is_empty() {
            return None;
        }
        let cwd = pane.shell.cwd.clone();
        let mut cache = self.hint_cache.borrow_mut();
        let hint = match &*cache {
            Some((typed, dir, changes, hint))
                if *typed == input.text && *dir == cwd && *changes == self.history_changes =>
            {
                hint.clone()
            }
            _ => {
                let here = history.commands(&CommandFilter {
                    cwd: cwd.clone(),
                    only_ok: false,
                });
                let world = self.active_world();
                let all: Vec<_> = history
                    .commands(&CommandFilter::default())
                    .into_iter()
                    .filter(|e| world.has_command(e.cwd.as_deref()))
                    .collect();
                let hint = crate::hints::pick_hint(&input.text, &here, &all);
                *cache = Some((input.text.clone(), cwd, self.history_changes, hint.clone()));
                hint
            }
        };
        hint.map(|text| (text, cursor.0, cursor.1))
    }

    /// Puts `text` into the prompt of the active pane in place of `typed`.
    fn type_into_prompt(&self, typed: &str, text: &str, run: bool) {
        if let Some(session) = self.running.as_ref().and_then(Running::session) {
            session.with_term_mut(|term, _| term.scroll_display(Scroll::Bottom));
            session.write(replace_input(typed, text, run));
        }
    }

    fn history_key(&mut self, event: &KeyEvent) {
        let Some(popup) = &mut self.history_popup else {
            return;
        };
        let ctrl = self.mods.control_key();
        let shift = self.mods.shift_key();
        let letter = if ctrl {
            physical_letter(event.physical_key)
        } else {
            None
        };
        let kind = popup.kind;
        match (&event.logical_key, letter) {
            (Key::Named(NamedKey::Escape), _) => self.history_popup = None,
            (Key::Named(NamedKey::Enter), _) => return self.accept_history_popup(shift, ctrl),
            (Key::Named(NamedKey::ArrowUp), _) => popup.move_selection(-1),
            (Key::Named(NamedKey::ArrowDown), _) => popup.move_selection(1),
            (Key::Named(NamedKey::PageUp), _) => popup.move_selection(-(VISIBLE_ROWS as i32)),
            (Key::Named(NamedKey::PageDown), _) => popup.move_selection(VISIBLE_ROWS as i32),
            (Key::Named(NamedKey::Backspace), _) => popup.backspace(),
            (Key::Named(NamedKey::Delete), _) if kind == PopupKind::Sessions => {
                if let Some(row) = popup.selected().cloned() {
                    let _ = std::fs::remove_file(&row.key);
                    self.refresh_history_popup();
                }
            }
            (Key::Named(NamedKey::Delete), _) => {
                if let Some(row) = popup.selected().cloned()
                    && let Some(history) = &mut self.history
                {
                    let result = match kind {
                        PopupKind::Commands => history.forget_command(&row.text),
                        PopupKind::Dirs => history.forget_dir(&row.text, now_ms()),
                        PopupKind::Sessions | PopupKind::Themes => Ok(()),
                    };
                    if let Err(err) = result {
                        tracing::warn!("cannot change the history: {err}");
                    }
                    self.refresh_history_popup();
                }
            }
            (_, Some('d')) if kind == PopupKind::Commands => {
                popup.only_here = !popup.only_here;
                self.refresh_history_popup();
            }
            (_, Some('g')) if kind == PopupKind::Commands => {
                popup.only_ok = !popup.only_ok;
                self.refresh_history_popup();
            }
            (_, Some('p')) if kind == PopupKind::Dirs => {
                if let Some(row) = popup.selected().cloned()
                    && let Some(history) = &mut self.history
                {
                    let pinned = row.hint.starts_with('★');
                    if let Err(err) = history.pin_dir(&row.text, !pinned, now_ms()) {
                        tracing::warn!("cannot change the history: {err}");
                    }
                    self.refresh_history_popup();
                }
            }
            (_, Some('c')) => {
                if let Some(row) = popup.selected().cloned() {
                    self.history_popup = None;
                    self.copy_text(row.text);
                }
            }
            _ => {
                if let Some(text) = event.text.as_deref()
                    && !ctrl
                    && !self.mods.alt_key()
                {
                    popup.type_text(text);
                }
            }
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

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

    /// `install_claude_hooks`: shows what changes in the Claude Code settings, and asks first.
    fn ask_install_hooks(&mut self) {
        let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(std::path::PathBuf::from);
        let home = fterm_config::profiles::home_dir();
        let Some(path) = crate::claude_hooks::settings_path(config_dir.as_deref(), home.as_deref())
        else {
            return self.notify(None, "No home folder", "", Level::Error, Source::App);
        };
        let shown = path.display().to_string();
        let settings = match std::fs::read_to_string(&path) {
            Ok(text) if text.trim().is_empty() => serde_json::json!({}),
            Ok(text) => match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
                Ok(value) => value,
                Err(err) => {
                    let body = format!("{shown}: {err}. It is not changed.");
                    return self.notify(
                        None,
                        "Bad Claude Code settings",
                        &body,
                        Level::Error,
                        Source::App,
                    );
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
            Err(err) => {
                let body = format!("{shown}: {err}");
                return self.notify(
                    None,
                    "Cannot read the Claude Code settings",
                    &body,
                    Level::Error,
                    Source::App,
                );
            }
        };
        let states: serde_json::Value =
            serde_json::from_str(crate::agent::CLAUDE_HOOKS).expect("the fterm hooks are JSON");
        // The plan review needs ftermctl; it is next to fterm.
        let ftermctl = std::env::current_exe().ok().and_then(|exe| {
            let name = if cfg!(windows) {
                "ftermctl.exe"
            } else {
                "ftermctl"
            };
            let path = exe.with_file_name(name);
            path.is_file().then_some(path)
        });
        let ours = crate::claude_hooks::fterm_hooks(&states, ftermctl.as_deref());
        let (merged, added) = match crate::claude_hooks::merge_hooks(&settings, &ours) {
            Ok(result) => result,
            Err(err) => {
                let body = format!("{shown}: {err}. It is not changed.");
                return self.notify(
                    None,
                    "Cannot add the hooks",
                    &body,
                    Level::Error,
                    Source::App,
                );
            }
        };
        if added.is_empty() {
            return self.notify(
                None,
                "The fterm hooks are already there",
                &shown,
                Level::Info,
                Source::App,
            );
        }
        let text = serde_json::to_string_pretty(&merged).expect("JSON") + "\n";
        self.palette = None;
        self.hooks_question = Some(HooksQuestion {
            lines: vec![
                "Add the fterm hooks to Claude Code?".to_owned(),
                String::new(),
                shown,
                format!("Hooks for: {}", added.join(", ")),
                "PreToolUse (ExitPlanMode) shows the plans of plan mode in a Review tab."
                    .to_owned(),
                "Your other settings and hooks stay. The old file is kept".to_owned(),
                "as settings.json.bak-fterm. New Claude Code sessions use them.".to_owned(),
                String::new(),
                "Enter = add, Esc = no".to_owned(),
            ],
            path,
            text,
        });
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Keys while the hooks question is open.
    fn hooks_question_key(&mut self, event: &KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => {
                if let Some(q) = self.hooks_question.take() {
                    let shown = q.path.display().to_string();
                    let written = (|| -> std::io::Result<()> {
                        if let Some(dir) = q.path.parent() {
                            std::fs::create_dir_all(dir)?;
                        }
                        if q.path.exists() {
                            std::fs::copy(&q.path, q.path.with_extension("json.bak-fterm"))?;
                        }
                        std::fs::write(&q.path, &q.text)
                    })();
                    match written {
                        Ok(()) => self.notify(
                            None,
                            "The fterm hooks are in Claude Code",
                            &format!("{shown}. Start Claude Code again to use them."),
                            Level::Success,
                            Source::App,
                        ),
                        Err(err) => self.notify(
                            None,
                            "Cannot write the Claude Code settings",
                            &format!("{shown}: {err}"),
                            Level::Error,
                            Source::App,
                        ),
                    }
                }
            }
            Key::Named(NamedKey::Escape) => self.hooks_question = None,
            _ => return,
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
        let cell = running.renderer.cell();
        let layout = layout_tabs(running.mux.tabs().len(), self.tabs_width(width, cell), cell);
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
        // A click on a toast: × closes it, the rest goes to its pane.
        if state == ElementState::Pressed
            && self.palette.is_none()
            && self.history_popup.is_none()
            && let Some((id, close)) = self.toast_under_mouse()
        {
            let pane = self.center.get(id).and_then(|n| n.pane);
            self.center.dismiss(id);
            if !close && let Some(pane) = pane {
                self.go_to_pane(pane);
            }
            if let Some(running) = &self.running {
                running.window.request_redraw();
            }
            return;
        }
        if self.close_question.is_some() || self.palette.is_some() || self.history_popup.is_some() {
            if state == ElementState::Pressed {
                // A click closes the palette (and the history popup).
                self.palette = None;
                self.history_popup = None;
                if let Some(running) = &self.running {
                    running.window.request_redraw();
                }
            }
            return;
        }
        if !self.mouse.selecting
            && self.mouse.dragging_divider.is_none()
            && self.dock_mouse(state, button)
        {
            return;
        }
        // A click on the unread number in the tab bar opens the Events panel.
        if state == ElementState::Pressed
            && button == MouseButton::Left
            && let Some(text) = self.tab_bar_corner()
            && let Some(running) = &mut self.running
        {
            let width = running.window.inner_size().width as f32;
            let rect = corner_rect(&text, width, running.renderer.cell());
            let (x, y) = self.mouse.position;
            if rect.contains(x as f32, y as f32) {
                running.dock.show(PanelKind::Events);
                self.dock_changed();
                return;
            }
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

        // The mouse over a toast stops its timer.
        let over = self.toast_under_mouse();
        let before: Vec<u64> = Vec::new();
        let _ = before;
        self.center.hover(over.map(|(id, _)| id), Instant::now());
        if over.is_some() {
            if let Some(running) = &self.running {
                running.window.request_redraw();
            }
            return;
        }

        if !self.mouse.selecting
            && self.mouse.dragging_divider.is_none()
            && self.dock_mouse_moved(x as f32, y as f32)
        {
            return;
        }

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
        if let Some(popup) = &mut self.history_popup {
            let step = match delta {
                MouseScrollDelta::LineDelta(_, y) => -y.signum() as i32,
                MouseScrollDelta::PixelDelta(p) => -(p.y.signum() as i32),
            };
            popup.move_selection(step);
            if let Some(running) = &self.running {
                running.window.request_redraw();
            }
            return;
        }
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
        if let Some(layout) = self.dock_layout()
            && layout
                .rect
                .contains(self.mouse.position.0 as f32, self.mouse.position.1 as f32)
        {
            let rows = self.dock_rows().len();
            let reader = self.reader_lines();
            if let Some(running) = &mut self.running {
                let lines = self
                    .mouse
                    .wheel
                    .lines(delta, running.renderer.cell().height);
                if let Some((text, visible)) = reader {
                    running
                        .dock
                        .scroll_reader(-(lines as isize), text.len(), visible);
                } else if running.dock.active == PanelKind::Ai {
                    self.ai.scroll = self.ai.scroll.saturating_add_signed(lines as isize);
                } else {
                    running
                        .dock
                        .scroll_by(-(lines as isize), rows, layout.visible_rows());
                }
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
        self.draw_reviews();
        let focused = self.focused;
        // The events count as read when the Events panel goes away (or fterm goes to the back).
        let events_seen = self
            .running
            .as_ref()
            .is_some_and(|r| r.dock.showing(PanelKind::Events))
            && focused;
        if self.events_seen && !events_seen {
            self.center.mark_all_read();
        }
        self.events_seen = events_seen;
        // The dock: its rows, and the selection kept inside the list.
        let dock_layout = self.dock_layout();
        let dock_rows: Vec<DockRow> = self.dock_rows().into_iter().map(|(row, _)| row).collect();
        let dock_tabs = self.dock_tabs();
        let corner = self.tab_bar_corner();
        // The full text of an event, when it is open.
        let reader = self.reader_lines();
        // The AI chat: its lines, its input, and the scroll kept inside the chat.
        let ai_view = match (&dock_layout, self.running.as_ref()) {
            (Some(layout), Some(r)) if r.dock.active == PanelKind::Ai => {
                let cell = r.renderer.cell();
                let width = ((layout.list.width / cell.width) as usize).saturating_sub(2);
                let lines = crate::ai_chat::layout(&self.ai.views(), width);
                let (input, cursor) = self.ai.input.layout(width);
                let rows = fterm_render::dock::chat_rows(layout, cell, input.len());
                let max_scroll = lines.len().saturating_sub(rows);
                let chips: Vec<String> = self.ai.context.iter().map(|c| c.label()).collect();
                Some((lines, input, cursor, self.ai_title(), max_scroll, chips))
            }
            _ => None,
        };
        if let Some((_, _, _, _, max_scroll, _)) = &ai_view {
            self.ai.scroll = self.ai.scroll.min(*max_scroll);
        }
        let ai_scroll = self.ai.scroll;
        if let (Some(layout), Some(running)) = (&dock_layout, &mut self.running) {
            let selected = running.dock.selected();
            running
                .dock
                .select(selected, dock_rows.len(), layout.visible_rows());
        }
        let ui = self.ui;
        let (titles, badges, tab_colors) = match &mut self.running {
            Some(running) => {
                if focused {
                    for id in running.visible_panes() {
                        if let Some(agent) =
                            running.panes.get_mut(&id).and_then(|p| p.agent.as_mut())
                        {
                            agent.seen = true;
                        }
                    }
                }
                let badges: Vec<_> = running
                    .tab_badges()
                    .into_iter()
                    .map(|kind| kind.map(|kind| badge_color(kind, &ui)))
                    .collect();
                let tab_colors: Vec<_> =
                    running
                        .mux
                        .tabs()
                        .iter()
                        .map(|t| {
                            t.color.map(|[r, g, b]| {
                                fterm_term::alacritty_terminal::vte::ansi::Rgb { r, g, b }
                            })
                        })
                        .collect();
                (running.tab_titles(), badges, tab_colors)
            }
            None => return,
        };
        let renaming = self
            .renaming
            .as_ref()
            .map(|(tab, text)| (*tab, text.clone()));
        let hover = self.mouse.tab_hover;
        let question = self
            .restore_question_lines()
            .or_else(|| self.name_prompt_lines())
            .or_else(|| self.key_prompt_lines())
            .or_else(|| self.access_question_lines())
            .or_else(|| self.close_question.as_ref().map(|q| q.lines.clone()))
            .or_else(|| self.hooks_question.as_ref().map(|q| q.lines.clone()))
            .or_else(|| self.message.clone());
        let hovered = self.toast_under_mouse().map(|(id, _)| id);
        let toast_layout = self.toast_layout();
        let toast_rects: Vec<Rect> = toast_layout.iter().map(|(_, r)| *r).collect();
        let toast_data: Vec<(String, String, ToastLevel, bool)> = toast_layout
            .iter()
            .filter_map(|(id, _)| {
                let n = self.center.get(*id)?;
                let level = toast_level(n.level);
                Some((n.title.clone(), n.body.clone(), level, hovered == Some(*id)))
            })
            .collect();
        let palette_rows: Option<ListView> = self
            .palette
            .as_ref()
            .map(|p| {
                let rows = p
                    .visible()
                    .into_iter()
                    .map(|(item, selected)| (item.label.clone(), item.key.clone(), selected))
                    .collect();
                ListView {
                    query: p.query().to_owned(),
                    rows,
                    title: String::new(),
                    footer: "",
                    bad: Vec::new(),
                }
            })
            .or_else(|| {
                self.history_popup.as_ref().map(|p| {
                    let visible = p.visible();
                    ListView {
                        query: p.query().to_owned(),
                        bad: visible.iter().map(|(row, _)| row.bad).collect(),
                        rows: visible
                            .into_iter()
                            .map(|(row, selected)| (row.text.clone(), row.hint.clone(), selected))
                            .collect(),
                        title: p.title(),
                        footer: p.footer(),
                    }
                })
            });
        let hint = self.pending_hint().or_else(|| self.current_hint());
        let tabs_width = self.running.as_ref().map_or(0.0, |r| {
            self.tabs_width(r.window.inner_size().width as f32, r.renderer.cell())
        });
        let original_colors = self.original_colors;
        let palette_changes = self.config.config.palette_changes;
        let running = self.running.as_mut().unwrap();
        let area = running.tab_area();
        let Running {
            gpu,
            renderer,
            mux,
            panes,
            window,
            dock,
        } = running;
        let size = window.inner_size();
        let cell = renderer.cell();
        let width = size.width as f32;
        let dock_focused = dock.focused;
        let (dock_active, dock_selected, dock_scroll, dock_filter) =
            (dock.active, dock.selected(), dock.scroll(), dock.filter);
        let reader_scroll = dock.reader_scroll();
        let dock_hints = match (dock_active, dock_filter) {
            (PanelKind::Events, _) if reader.is_some() => "Enter go · Ctrl+C copy · Esc back",
            (PanelKind::Events, EventFilter::All) => {
                "Enter go · Space read · F important only · M read · Tab · Esc"
            }
            (PanelKind::Events, EventFilter::Important) => {
                "Enter go · Space read · F show all · M read · Tab · Esc"
            }
            (PanelKind::Agents, _) => "Enter go · Tab next panel · Esc back",
            (PanelKind::Ai, _) => "Enter send · Shift+Enter new line · Esc stop / back · PageUp",
        };
        let dock_empty = match (dock_active, dock_filter) {
            (PanelKind::Events, EventFilter::All) => "No events yet.",
            (PanelKind::Events, EventFilter::Important) => "No important events.",
            (PanelKind::Agents, _) => {
                "No agents. For exact states: palette → Install Claude Code hooks."
            }
            (PanelKind::Ai, _) => "",
        };
        let dock_active_index = PanelKind::ALL
            .iter()
            .position(|k| *k == dock_active)
            .unwrap_or(0);
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
        let layout = layout_tabs(mux.tabs().len(), tabs_width, cell);
        let active = mux.active_index();
        let editing = renaming.as_ref().and_then(|(tab, text)| {
            let index = mux.tabs().iter().position(|t| t.id == *tab)?;
            Some((index, text.as_str()))
        });
        let view = Rect::new(0.0, 0.0, width, size.height as f32);

        let mut draw = |device: &wgpu::Device,
                        queue: &wgpu::Queue,
                        target: &wgpu::TextureView,
                        size: (u32, u32)| {
            renderer.render(device, queue, target, size, |parts| {
                parts.tab_bar(&TabBarInput {
                    layout: &layout,
                    titles: &titles,
                    active,
                    hover,
                    editing,
                    badges: &badges,
                    colors: &tab_colors,
                    corner: corner
                        .as_deref()
                        .map(|text| (text, level_color(Level::Attention, &ui))),
                    cell,
                    width,
                })?;
                for (id, rect) in &pane_rects {
                    if let Some(pane) = panes.get(id) {
                        let is_active = Some(*id) == active_pane;
                        let has_keys = focused && is_active && !dock_focused;
                        let harmonize = pane.harmonize && !original_colors;
                        let program_palette = pane.palette_changes.unwrap_or(palette_changes);
                        pane.session.with_term(|term| {
                            parts.pane(term, *rect, has_keys, harmonize, program_palette)
                        })?;
                    }
                }
                if let Some((text, column, line)) = &hint
                    && let Some((_, rect)) =
                        pane_rects.iter().find(|(p, _)| Some(*p) == active_pane)
                    && let Some(pane) = active_pane.and_then(|id| panes.get(&id))
                {
                    let columns = pane.session.with_term(|term| term.grid().columns());
                    parts.ghost(text, *rect, *column, *line, columns)?;
                }
                parts.pane_chrome(&dividers, active_frame);
                if let Some(layout) = &dock_layout {
                    parts.dock(
                        &DockView {
                            tabs: &dock_tabs,
                            active: dock_active_index,
                            rows: &dock_rows,
                            selected: Some(dock_selected),
                            scroll: dock_scroll,
                            focused: dock_focused && focused,
                            empty: dock_empty,
                            hints: dock_hints,
                            chat: ai_view.as_ref().map(
                                |(lines, input, cursor, title, _, chips)| {
                                    fterm_render::dock::ChatView {
                                        lines,
                                        scroll: ai_scroll,
                                        input,
                                        cursor: (dock_focused && focused).then_some(*cursor),
                                        title,
                                        chips,
                                    }
                                },
                            ),
                            reader: reader.as_ref().map(|(lines, _)| {
                                fterm_render::dock::ReaderView {
                                    lines,
                                    scroll: reader_scroll,
                                }
                            }),
                        },
                        layout,
                    )?;
                }
                let views: Vec<ToastView> = toast_data
                    .iter()
                    .map(|(title, body, level, hover)| ToastView {
                        title,
                        body,
                        level: *level,
                        hover: *hover,
                    })
                    .collect();
                parts.toasts(&views, &toast_rects)?;
                if let Some(list) = &palette_rows {
                    parts.palette(
                        &fterm_render::overlay::PaletteView {
                            query: &list.query,
                            rows: &list.rows,
                            title: &list.title,
                            footer: list.footer,
                            bad: &list.bad,
                        },
                        view,
                    )?;
                }
                if let Some(lines) = &question {
                    parts.message_box(lines, view)?;
                }
                Ok(())
            });
        };
        let result = gpu.frame(&mut draw);
        // Screenshots: the same frame, in a texture of its own, cut to the pane.
        if !self.shots.is_empty() {
            let tab = &mux.tabs()[mux.active_index()];
            let rects = match tab.zoomed {
                Some(pane) => vec![(pane, area)],
                None => tab.layout.rects(area),
            };
            for shot in std::mem::take(&mut self.shots) {
                let answer = match rects.iter().find(|(id, _)| *id == shot.pane) {
                    None => Err(fterm_api::protocol::RpcError::invalid_params(format!(
                        "pane {} is not on the screen (it is in another tab): show it first (focus)",
                        shot.pane.0
                    ))),
                    Some((_, rect)) => crate::screenshot::take(
                        gpu,
                        crate::gpu::pixel_area(*rect),
                        &mut draw,
                        &shot.path,
                    )
                    .map(|(width, height)| {
                        serde_json::json!({
                            "pane": shot.pane.0,
                            "path": shot.path.display().to_string(),
                            "width": width,
                            "height": height,
                        })
                    })
                    .map_err(|err| {
                        fterm_api::protocol::RpcError::new(
                            fterm_api::protocol::RpcError::INTERNAL,
                            format!("{err:#}"),
                        )
                    }),
                };
                let _ = shot.reply.send(answer);
            }
        }
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
            Ok(running) => {
                // winit sends `Focused` only on a change, so read the first state here.
                self.focused = running.window.has_focus();
                self.system_dark = running.window.theme() != Some(winit::window::Theme::Light);
                self.running = Some(running);
                if self.load_theme() {
                    self.apply_theme();
                }
                self.start_api();
                if let Some(dir) = std::env::var_os("FTERM_RECORD").filter(|d| !d.is_empty()) {
                    // A recording can have secrets: it must not stay on by mistake.
                    self.notify(
                        None,
                        "Recording the output of every pane",
                        &format!(
                            "FTERM_RECORD: {}. It can have secrets; remove FTERM_RECORD when you are done.",
                            std::path::Path::new(&dir).display()
                        ),
                        Level::Warning,
                        Source::App,
                    );
                }
            }
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
            // Dev only: FTERM_RUN="command" types this command into the first tab after start
            // (many commands: one per line). Test scripts use it, so they do not need to send keys to the window.
            if cfg!(debug_assertions)
                && let Ok(command) = std::env::var("FTERM_RUN")
                && let Some(session) = running.session()
            {
                tracing::info!(%command, "FTERM_RUN");
                for line in command.lines() {
                    session.write(format!("{line}\r").into_bytes());
                }
            }
        }
        self.tab_changed();
        self.offer_restore();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.save_or_forget_session();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        let event = match event {
            UserEvent::Api(request) => return self.api_call(event_loop, request),
            UserEvent::ApiGone(client) => return self.api_client_gone(client),
            UserEvent::Ai(id, event) => return self.ai_event(id, event),
            UserEvent::AiCommand(id, event) => return self.command_event(id, event),
            other => other,
        };
        let Some(running) = &mut self.running else {
            return;
        };
        let (pane, event) = match event {
            UserEvent::Term(pane, event) => (pane, event),
            UserEvent::ConfigChanged => {
                self.reload_at = Some(Instant::now() + CONFIG_DEBOUNCE);
                return;
            }
            UserEvent::ThemeFilesChanged => {
                self.theme_reload_at = Some(Instant::now() + CONFIG_DEBOUNCE);
                return;
            }
            UserEvent::Api(_)
            | UserEvent::ApiGone(_)
            | UserEvent::Ai(..)
            | UserEvent::AiCommand(..) => {
                return;
            }
        };
        match event {
            TermEvent::Redraw => {
                if running.mux.active_pane() == Some(pane) {
                    running.window.request_redraw();
                }
                self.check_text_waits(pane);
            }
            TermEvent::Title(title) => {
                if let Some(server) = &self.api_server {
                    server.broadcast(
                        "title",
                        serde_json::json!({ "pane": pane.0, "title": title }),
                    );
                }
                if let Some(p) = running.panes.get_mut(&pane) {
                    p.app_title = (!title.is_empty()).then_some(title);
                }
                running.window.request_redraw();
                self.update_window_title();
                self.detect_agent(event_loop, pane);
            }
            TermEvent::Osc(osc) => {
                tracing::debug!(pane = pane.0, ?osc, "osc event");
                let Some(p) = running.panes.get_mut(&pane) else {
                    return;
                };
                let new_dir = match &osc {
                    OscEvent::Cwd(dir) if p.shell.cwd.as_deref() != Some(dir) => Some(dir.clone()),
                    _ => None,
                };
                let done = p.shell.apply(&osc, Instant::now());
                if p.shell.at_prompt() {
                    // The first prompt of a restored pane: its old text above, its old command in it.
                    let lines = std::mem::take(&mut p.intro_lines);
                    if lines > 0 {
                        p.session.with_term_mut(|term, _| {
                            let up = crate::session_state::intro_scroll(lines, term.screen_lines());
                            term.scroll_display(Scroll::Delta(up as i32));
                        });
                    }
                    if let Some((text, run)) = p.rerun.take() {
                        p.session
                            .write(crate::history_popup::replace_input("", &text, run));
                    }
                }
                let program = p.session.program().to_owned();
                if let Some(dir) = &new_dir
                    && let Some(server) = &self.api_server
                {
                    server.broadcast("cwd", serde_json::json!({ "pane": pane.0, "cwd": dir }));
                }
                if let Some(dir) = new_dir
                    && let Some(history) = &mut self.history
                    && let Err(err) = history.visit_dir(&dir, now_ms())
                {
                    tracing::warn!("cannot save the folder: {err}");
                }
                // A command starts or ends (then the shell waits at its prompt): the agent tool may
                // start or end.
                if matches!(
                    osc,
                    OscEvent::Prompt(
                        fterm_term::osc::PromptMark::CommandExecuted
                            | fterm_term::osc::PromptMark::CommandFinished(_)
                    ) | OscEvent::InputStart { .. }
                ) {
                    self.detect_agent(event_loop, pane);
                }
                if let OscEvent::Notify { title, body } = &osc {
                    let title = title.clone().unwrap_or(program);
                    self.notify(Some(pane), &title, body, Level::Info, Source::Terminal);
                } else if let OscEvent::Agent { state, message } = &osc {
                    if let Some(p) = self.running.as_mut().and_then(|r| r.panes.get_mut(&pane)) {
                        p.agent_hooks = true;
                    }
                    self.agent_state(event_loop, pane, state, message);
                } else if let OscEvent::TabColor(text) = &osc {
                    // A bad color from a program is not worth a toast: it is only logged.
                    match fterm_config::colors::tab_color(text) {
                        Ok(color) => {
                            self.set_tab_color(pane, color);
                        }
                        Err(err) => tracing::warn!(pane = pane.0, "tab color: {err}"),
                    }
                } else if let Some(ShellEvent::CommandDone {
                    exit,
                    took,
                    command,
                    cwd,
                }) = done
                {
                    tracing::debug!(pane = pane.0, ?exit, ?took, ?command, "command done");
                    if let Some(p) = self.running.as_mut().and_then(|r| r.panes.get_mut(&pane)) {
                        p.last_command = Some(LastCommand {
                            command: command.clone(),
                            exit,
                            took_ms: took.as_millis() as u64,
                        });
                    }
                    let data = serde_json::json!({
                        "event": "command_done",
                        "pane": pane.0,
                        "command": command,
                        "exit": exit,
                        "took_ms": took.as_millis() as u64,
                        "cwd": cwd,
                    });
                    self.api_event("command_done", data.clone());
                    self.resolve_waits(pane, crate::waits::Happening::CommandDone, data);
                    self.save_command(pane, command, cwd, exit, took);
                    self.command_done(pane, exit, took);
                }
            }
            TermEvent::Bell => {
                if self.config.config.notifications.bell {
                    self.notify(Some(pane), "Bell", "", Level::Info, Source::Terminal);
                }
            }
            TermEvent::Exit => {
                tracing::info!(pane = pane.0, "the shell ended");
                self.api_pane_closed(pane);
                let Some(running) = &mut self.running else {
                    return;
                };
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
        self.scene_events();
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
        match self.theme_reload_at {
            Some(at) if now >= at => {
                self.theme_reload_at = None;
                if self.load_theme() {
                    self.apply_theme();
                }
            }
            Some(at) => wake_at = Some(wake_at.map_or(at, |t: Instant| t.min(at))),
            None => {}
        }
        if self.autoscroll() {
            let tick = now + AUTOSCROLL_TICK;
            wake_at = Some(wake_at.map_or(tick, |t: Instant| t.min(tick)));
        }
        self.center.tick(now);
        if let Some(at) = self.center.next_deadline() {
            wake_at = Some(wake_at.map_or(at, |t: Instant| t.min(at)));
            if let Some(running) = &self.running {
                running.window.request_redraw();
            }
        }
        if let Some(at) = self.expire_waits(now) {
            wake_at = Some(wake_at.map_or(at, |t: Instant| t.min(at)));
        }
        if let Some(at) = self.expire_reviews(now) {
            wake_at = Some(wake_at.map_or(at, |t: Instant| t.min(at)));
        }
        if self.running.is_some() {
            let at = self.autosave(now);
            wake_at = Some(wake_at.map_or(at, |t: Instant| t.min(at)));
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
            WindowEvent::CloseRequested => {
                // A question is open already: a second × closes for real.
                match self.close_question.as_ref().map(|q| q.target) {
                    Some(CloseTarget::Window) => event_loop.exit(),
                    _ => self.close(event_loop, CloseTarget::Window),
                }
            }
            WindowEvent::Resized(size) => {
                let running = self.running.as_mut().unwrap();
                running.gpu.resize(size);
                self.resize_all_panes();
                self.running.as_ref().unwrap().window.request_redraw();
            }
            WindowEvent::ThemeChanged(theme) => {
                self.system_dark = theme == winit::window::Theme::Dark;
                let follows = matches!(
                    self.config.config.theme,
                    fterm_config::theme::ThemeChoice::System { .. }
                );
                if follows && self.theme_override.is_none() && self.load_theme() {
                    self.apply_theme();
                }
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
                if self.restore_offer.is_some() {
                    self.restore_key(&event);
                    return;
                }
                if self.name_prompt.is_some() {
                    self.name_prompt_key(&event);
                    return;
                }
                if self.key_prompt.is_some() {
                    self.key_prompt_key(&event);
                    return;
                }
                if !self.api_questions.is_empty() {
                    let answer = match (&event.logical_key, physical_letter(event.physical_key)) {
                        (Key::Named(NamedKey::Enter), _) => Some(api_calls::AccessAnswer::Allow),
                        (_, Some('a')) => Some(api_calls::AccessAnswer::Always),
                        (Key::Named(NamedKey::Escape), _) | (_, Some('n')) => {
                            Some(api_calls::AccessAnswer::Deny)
                        }
                        _ => None,
                    };
                    if let Some(answer) = answer {
                        self.answer_access(event_loop, answer);
                    }
                    return;
                }
                if self.close_question.is_some() {
                    self.close_question_key(event_loop, &event);
                    return;
                }
                if self.hooks_question.is_some() {
                    self.hooks_question_key(&event);
                    return;
                }
                if self.palette.is_some() {
                    self.palette_key(event_loop, &event);
                    return;
                }
                if self.history_popup.is_some() {
                    self.history_key(&event);
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
                if self.running.as_ref().is_some_and(|r| r.dock.focused) {
                    match action {
                        Some(action) => self.run_action(event_loop, action),
                        None => self.dock_key(&event),
                    }
                    return;
                }
                if self.handle_copy_keys(&event, action.as_ref()) {
                    return;
                }
                if let Some(action) = action {
                    self.run_action(event_loop, action);
                    return;
                }
                if self.active_review().is_some() {
                    self.review_key(&event);
                    return;
                }
                if event.logical_key == Key::Named(NamedKey::Escape) && self.stop_command() {
                    return;
                }
                let plain =
                    !self.mods.control_key() && !self.mods.alt_key() && !self.mods.shift_key();
                if plain
                    && matches!(
                        event.logical_key,
                        Key::Named(NamedKey::ArrowRight | NamedKey::End)
                    )
                    && let Some((rest, _, _)) = self.current_hint()
                    && let Some(session) = self.running.as_ref().and_then(Running::session)
                {
                    session.write(rest.into_bytes());
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
    let folder = crate::paths::folder(
        None,
        crate::paths::data_dir().as_deref(),
        "shell",
        Some(base.join("fterm").join("shell")),
    )
    .unwrap_or_default();
    match install_scripts(&folder) {
        Ok(path) => Some(path),
        Err(err) => {
            tracing::warn!("cannot write the shell integration scripts: {err}");
            None
        }
    }
}

/// The height of a Braille dot / its width on the screen: a cell is 2 dots wide and 4 dots tall.
fn dot_aspect(cell: fterm_render::font::CellMetrics) -> f32 {
    (cell.height / 4.0) / (cell.width / 2.0)
}

/// bash that reads Windows paths. On Windows `bash.exe` from PATH can be `System32\bash.exe`
/// (that is WSL), so only a bash with its full path (Git Bash, MSYS2) counts there.
fn native_bash(program: &str) -> bool {
    !cfg!(windows)
        || (program.contains(['\\', '/']) && !program.to_ascii_lowercase().contains("system32"))
}

/// The profiles from the config, or the ones found on this computer.
fn profiles_for(config: &LoadedConfig) -> Vec<Profile> {
    if config.config.profiles.is_empty() {
        let distros = if cfg!(windows) && which("wsl") {
            fterm_config::profiles::installed_wsl_distros()
        } else {
            Vec::new()
        };
        detect_profiles(cfg!(windows), which, std::path::Path::exists, &distros)
    } else {
        config.config.profiles.clone()
    }
}

fn dock_side(place: DockPlace) -> DockSide {
    match place {
        DockPlace::Left => DockSide::Left,
        DockPlace::Right => DockSide::Right,
        DockPlace::Bottom => DockSide::Bottom,
    }
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
