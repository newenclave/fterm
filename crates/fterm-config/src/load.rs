//! Loading `fterm.lua`: run the Luau script and read the table it returns.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::colors::{ColorConfig, Rgb, parse_color};
use crate::copy_keys::{CopyBinding, CopyCommand, CopyKeymap, JumpKind, parse_keys};
use crate::keys::{Action, BuiltinAction, KeyChord, Keymap, SpawnWhere};
use crate::profiles::{Profile, expand_home, home_dir};

/// How Braille chars are drawn in panes with programs (scenes always use pixels).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BrailleStyle {
    /// Square pixels with no gaps (graphs look solid).
    Pixels,
    /// Round dots, like in a font (spinners of agents look right).
    #[default]
    Dots,
}

/// How pictures in the output (OSC 1337) are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pictures {
    /// Real pixels.
    #[default]
    Pixels,
    /// Braille dots in the cells of the picture.
    Braille,
    /// Not drawn.
    Off,
}

/// Where the companions of a pane are shown in it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompanionSide {
    /// A strip at the bottom of the pane.
    #[default]
    Bottom,
    /// A strip at the right of the pane.
    Right,
}

/// Where toasts show in the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastPosition {
    #[default]
    BottomRight,
    TopRight,
    BottomLeft,
    TopLeft,
    Bottom,
}

/// Where the dock with the service panels is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DockPlace {
    Left,
    #[default]
    Right,
    Bottom,
}

/// `panels = { dock = "right", size = 0.28, open = { "events" } }`
#[derive(Clone, Debug, PartialEq)]
pub struct PanelsConfig {
    pub dock: DockPlace,
    /// The dock part of the window.
    pub size: f32,
    /// The panel that shows at start (`None` = the dock is closed).
    pub open: Option<String>,
}

impl Default for PanelsConfig {
    fn default() -> Self {
        Self {
            dock: DockPlace::Right,
            size: 0.28,
            open: None,
        }
    }
}

/// `history = { enabled, commands, dirs, ignore_space, hints }`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryConfig {
    /// Save commands and folders.
    pub enabled: bool,
    /// How many commands and folders to keep.
    pub commands: usize,
    pub dirs: usize,
    /// A command that starts with a space is not saved.
    pub ignore_space: bool,
    /// Grey hints from the history while you type.
    pub hints: bool,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            commands: 10_000,
            dirs: 500,
            ignore_space: true,
            hints: false,
        }
    }
}

/// A command before `on_history` sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryIn {
    pub cmd: String,
    pub cwd: Option<String>,
    pub exit: Option<i32>,
    pub shell: String,
}

/// `api = { enabled, ask }`: the local API for `ftermctl`, MCP, and scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApiConfig {
    pub enabled: bool,
    /// Ask before a client reads or types into a pane that is not its own.
    pub ask: bool,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ask: true,
        }
    }
}

/// The protocol of an AI provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiKind {
    Anthropic,
    /// OpenAI chat completions (OpenAI, OpenRouter, Ollama, LM Studio).
    OpenAi,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiProvider {
    pub name: String,
    pub kind: AiKind,
    pub url: String,
    /// Empty = not set yet (the user must choose one).
    pub model: String,
    pub key_env: Option<String>,
    pub needs_key: bool,
}

/// `ai = { provider, providers, system, max_tokens, api_access }`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiConfig {
    pub provider: String,
    pub providers: Vec<AiProvider>,
    /// Extra instructions from the user.
    pub system: String,
    pub max_tokens: u32,
    /// The model for "text to command" (`None` = the model of the provider).
    pub command_model: Option<String>,
    /// May API clients (agents) ask questions in the AI panel? It costs the user's key and money.
    pub api_access: bool,
}

impl AiConfig {
    /// The provider to use now.
    pub fn current(&self) -> Option<&AiProvider> {
        self.providers.iter().find(|p| p.name == self.provider)
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        let preset = |name: &str,
                      kind: AiKind,
                      url: &str,
                      model: &str,
                      key_env: Option<&str>,
                      needs_key: bool| AiProvider {
            name: name.to_owned(),
            kind,
            url: url.to_owned(),
            model: model.to_owned(),
            key_env: key_env.map(str::to_owned),
            needs_key,
        };
        Self {
            provider: "anthropic".to_owned(),
            providers: vec![
                preset(
                    "anthropic",
                    AiKind::Anthropic,
                    "https://api.anthropic.com/v1",
                    "claude-haiku-4-5-20251001",
                    None,
                    true,
                ),
                preset(
                    "ollama",
                    AiKind::OpenAi,
                    "http://localhost:11434/v1",
                    "llama3.2",
                    None,
                    false,
                ),
                preset(
                    "openrouter",
                    AiKind::OpenAi,
                    "https://openrouter.ai/api/v1",
                    "",
                    Some("OPENROUTER_API_KEY"),
                    true,
                ),
                preset(
                    "openai",
                    AiKind::OpenAi,
                    "https://api.openai.com/v1",
                    "",
                    Some("OPENAI_API_KEY"),
                    true,
                ),
            ],
            system: String::new(),
            max_tokens: 2048,
            command_model: None,
            api_access: false,
        }
    }
}

/// A question before it goes to the AI (for `on_ai_request`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiRequestIn {
    /// What the user typed.
    pub question: String,
    /// What goes out: the context from the terminal and the question.
    pub text: String,
    pub provider: String,
    pub model: String,
}

/// What `window_title` gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleIn {
    pub tab: usize,
    pub tabs: usize,
    pub title: String,
    pub agent: Option<String>,
    pub waiting: usize,
    pub failed: usize,
    /// The title that fterm would show.
    pub default: String,
}

/// What fterm does with the last session at start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Restore {
    /// Ask: "Restore the last session?"
    #[default]
    Ask,
    Always,
    Never,
}

/// The GPU API that draws the window (`gpu.backend`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuBackend {
    /// DX12 on Windows, Metal on macOS, Vulkan or GL on Linux.
    #[default]
    Auto,
    Dx12,
    Vulkan,
    Gl,
    Metal,
}

/// Which GPU, when there are two (`gpu.power`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuPower {
    /// The fast one (a discrete GPU).
    #[default]
    High,
    /// The one that uses less power (an integrated GPU).
    Low,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuConfig {
    pub backend: GpuBackend,
    pub power: GpuPower,
}

/// What comes back in a restored pane where a program (or an agent) ran.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rerun {
    /// Put the command into the prompt; Enter runs it.
    #[default]
    Prompt,
    /// Run it at once.
    Run,
    Never,
}

/// When the window × asks first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConfirmClose {
    /// Only when a program or a working agent runs in some pane.
    #[default]
    Running,
    Always,
    Never,
}

/// Another machine with a mux server: its panes show in this window (`remotes` in the config).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    /// `user@host` (or a name of `~/.ssh/config`) for `ssh`.
    pub host: Option<String>,
    /// More arguments for `ssh`, before the host (for example `-p 2222`).
    pub ssh_args: Vec<String>,
    /// ftermctl on that machine (default: `ftermctl`, from its PATH).
    pub ftermctl: Option<String>,
    /// The whole connect command instead of `ssh` (WSL, a jump host, docker exec): it must run `ftermctl mux proxy`.
    pub command: Vec<String>,
}

impl Remote {
    /// An ad-hoc host ("Connect over SSH…").
    pub fn ssh(host: &str) -> Self {
        Self {
            name: host.to_owned(),
            host: Some(host.to_owned()),
            ssh_args: Vec::new(),
            ftermctl: None,
            command: Vec::new(),
        }
    }

    /// The command that connects to the mux server there (its stdin and stdout are the connection).
    pub fn connect_command(&self) -> Vec<String> {
        if !self.command.is_empty() {
            return self.command.clone();
        }
        // BatchMode: no password question (there is no console for it); a key or the ssh agent must work.
        // ConnectTimeout: a wrong host is an error after 10 s, not after the timeout of the network.
        let mut c: Vec<String> = [
            "ssh",
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
        ]
        .map(str::to_owned)
        .to_vec();
        c.extend(self.ssh_args.iter().cloned());
        c.push(self.host.clone().unwrap_or_default());
        c.push(
            self.ftermctl
                .clone()
                .unwrap_or_else(|| "ftermctl".to_owned()),
        );
        c.push("mux".to_owned());
        c.push("proxy".to_owned());
        c
    }
}

/// When a plan of Claude Code plan mode (the `ExitPlanMode` hook) opens a Review tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlanReview {
    /// Each time.
    Always,
    /// fterm asks first: R = review, Esc = the dialog of Claude Code.
    #[default]
    Ask,
    /// Never: Claude Code shows its own dialog.
    Never,
}

impl PlanReview {
    /// The name in the config.
    pub fn name(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Ask => "ask",
            Self::Never => "never",
        }
    }

    /// The next mode (for the palette action): ask, always, never, and ask again.
    pub fn next(self) -> Self {
        match self {
            Self::Ask => Self::Always,
            Self::Always => Self::Never,
            Self::Never => Self::Ask,
        }
    }
}

/// What runs in the window, for `on_close_window`. Tab numbers start at 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseIn {
    pub tabs: usize,
    pub panes: usize,
    /// (tab, program).
    pub running: Vec<(usize, String)>,
    /// (tab, name, state).
    pub agents: Vec<(usize, String, String)>,
}

/// The panel names.
pub const PANEL_NAMES: [&str; 2] = ["events", "agents"];

/// When fterm also sends a system (OS) notification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OsNotify {
    /// Never (the default: they can be very annoying).
    #[default]
    Never,
    Always,
    WhenUnfocused,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NotificationConfig {
    /// `None` = no toasts (the notifications still go to the Events panel).
    pub toasts: Option<ToastPosition>,
    pub max_visible: usize,
    pub os: OsNotify,
    /// Only these levels go to the OS. Empty = all levels.
    pub os_levels: Vec<String>,
    /// A command that runs longer than this (seconds) makes a notification when it ends. 0 = off.
    pub long_command: f64,
    /// The bell (BEL) makes a notification.
    pub bell: bool,
    /// Flash the taskbar for "attention" when the window is not in front.
    pub flash: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            toasts: Some(ToastPosition::BottomRight),
            max_visible: 4,
            os: OsNotify::Never,
            os_levels: Vec::new(),
            long_command: 10.0,
            bell: false,
            flash: true,
        }
    }
}

/// A notification before `on_notification` sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifyIn {
    pub title: String,
    pub body: String,
    pub level: String,
    pub source: String,
    pub pane: Option<u64>,
}

/// What `on_notification` said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotifyOut {
    Drop,
    Keep {
        title: String,
        body: String,
        level: String,
        /// `Some` = the function chose (true = send to the OS too).
        os: Option<bool>,
        toast: Option<bool>,
    },
}

/// An agent state change before `on_agent` sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentIn {
    pub pane: u64,
    /// `working`, `waiting`, `done`, `error`, or `idle`.
    pub state: String,
    pub previous: Option<String>,
    pub message: String,
    /// The tab title of the pane.
    pub name: String,
}

/// What `on_agent` said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentOut {
    /// Show the built-in notification (`false` when the function returned `false`).
    pub notify: bool,
    /// What the function asked fterm to do (`fterm.notify`, `fterm.spawn`, ...).
    pub calls: Vec<ApiCall>,
}

/// What `on_agent_row` gets: the agent of a pane, as the Agents panel knows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentRowIn {
    pub pane: u64,
    pub name: String,
    pub state: String,
    pub question: Option<String>,
    pub todos_done: usize,
    pub todos_total: usize,
    pub todo_current: Option<String>,
    pub cost: Option<f64>,
    pub context: Option<f64>,
    /// How many files it changed.
    pub files: usize,
    /// Its newest step.
    pub last: Option<String>,
}

/// What `on_agent_row` gave: each part that it set takes the place of the default one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentRowOut {
    pub title: Option<String>,
    pub detail: Option<String>,
    pub right: Option<String>,
}

/// Copy mode (`copy_mode` in the config).
#[derive(Clone, Debug)]
pub struct CopyModeConfig {
    pub keys: CopyKeymap,
    /// `copy` (y, Enter) leaves copy mode; `false` = it stays.
    pub exit_after_copy: bool,
    /// The bar on the last row of the pane: the mode, the line, the search.
    pub status_bar: bool,
}

impl Default for CopyModeConfig {
    fn default() -> Self {
        Self {
            keys: CopyKeymap::with_defaults(),
            exit_after_copy: true,
            status_bar: true,
        }
    }
}

/// What a Lua action of copy mode gets as `cm`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CopyModeIn {
    /// The selected text, if there is a selection.
    pub selection: Option<String>,
    /// The text of the cursor line.
    pub line: String,
    /// The word at the cursor.
    pub word: Option<String>,
    /// The line of the cursor, from 1 at the top of the history.
    pub row: i64,
    /// The column of the cursor, from 1.
    pub column: usize,
    /// `normal`, `visual`, `visual_line`, or `visual_block`.
    pub mode: String,
    /// The pattern of the last search.
    pub search: Option<String>,
}

/// What `on_agent_event` gets: one step of an agent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentEventIn {
    pub pane: u64,
    /// `prompt`, `tool`, `question`, `done`, or `usage`.
    pub kind: String,
    pub tool: Option<String>,
    pub file: Option<String>,
    pub command: Option<String>,
    /// The prompt, the question, or the last message.
    pub text: Option<String>,
    pub cost: Option<f64>,
    pub context: Option<f64>,
}

/// The parts that the Agents panel can show (`agents.show`).
pub const AGENT_PARTS: [&str; 6] = ["question", "todos", "cost", "context", "files", "last"];

/// `agents = { ... }`: the Agents panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentsConfig {
    /// Keep past conversations in `agents.jsonl` (H in the panel lists them).
    pub history: bool,
    /// The parts that the rows and the details show (see `AGENT_PARTS`).
    pub show: Vec<String>,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            history: true,
            show: AGENT_PARTS.iter().map(|p| (*p).to_owned()).collect(),
        }
    }
}

impl AgentsConfig {
    pub fn shows(&self, part: &str) -> bool {
        self.show.iter().any(|p| p == part)
    }
}

/// The dashboard (`dashboard` in the config): its tiles in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DashboardConfig {
    /// Columns of tiles; 0 = fterm chooses from the number of tiles and the window.
    pub columns: usize,
    pub tiles: Vec<DashTile>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            columns: 0,
            tiles: vec![
                DashTile::Panes(PaneFilter::Agents),
                DashTile::Panes(PaneFilter::Busy),
            ],
        }
    }
}

/// One entry of `dashboard.tiles`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DashTile {
    /// A tile for each pane that matches.
    Panes(PaneFilter),
    /// The output of a command, run again every `every` seconds. Each line is an item of a list that `actions` work
    /// on (fields between tabs are `{1}`, `{2}`, ...).
    Command {
        title: String,
        command: String,
        every: u64,
        /// The field of a line to show (from 1); `None` = the whole line.
        show: Option<usize>,
        actions: Vec<ItemAction>,
        /// `waiting_when = "output"`: the tile waits for you (yellow) when the list is not empty.
        waiting: bool,
        /// `command` as a list: the program and its arguments, with no shell (quotes reach the program as they are).
        /// `command` is then the words joined, for the display.
        argv: Option<Vec<String>>,
    },
    /// Buttons that run something.
    Buttons {
        title: String,
        buttons: Vec<DashButton>,
    },
}

/// Which panes a `panes` tile shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneFilter {
    /// Panes with an agent in them.
    Agents,
    /// Panes where a command runs, or the last one failed.
    Busy,
    All,
    Match(PaneMatch),
}

/// A pane matches when each given text is in its program, title, workspace, or profile (lower case).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneMatch {
    pub program: Option<String>,
    pub title: Option<String>,
    pub workspace: Option<String>,
    pub profile: Option<String>,
}

/// A key on an item of a list tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemAction {
    pub key: ItemKey,
    pub label: String,
    pub does: ItemDo,
    /// Ask first (a second press).
    pub confirm: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKey {
    Enter,
    Char(char),
}

/// What an item action does. `{1}`, `{2}`, ... are the fields of the item, `{line}` is the whole line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemDo {
    /// Open a link.
    Open(String),
    /// Programs with their arguments, one after another (no shell, so a field is always one argument): in the
    /// background with the result as a toast, or in an overlay to see the output.
    Run {
        steps: Vec<Vec<String>>,
        overlay: bool,
    },
    /// An action; a Lua function gets `(fterm, item)`.
    Action(Action),
}

/// A button of the dashboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DashButton {
    pub label: String,
    pub press: Press,
    /// Ask first.
    pub confirm: bool,
}

/// What a button does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Press {
    /// A command for the shell: its result as a toast, or in an overlay (`show = "overlay"`) to see the output.
    Run { command: String, overlay: bool },
    /// An action, as in `keys` and `commands`.
    Action(Action),
}

/// A line in the command palette from the config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserCommand {
    pub name: String,
    pub action: Action,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub font_size: f32,
    pub padding: f32,
    pub scrollback: usize,
    pub braille_style: BrailleStyle,
    /// How pictures in the output are drawn.
    pub pictures: Pictures,
    /// The size of an overlay in percent of the pane under it (20..100; 100 = all of it).
    pub overlay_size: u8,
    /// The height of the scratch shell in percent of the tab (20..100).
    pub scratch_size: u8,
    /// The command of the window overlay (for example `far`): it opens without a question the first time.
    pub window_overlay: Option<String>,
    /// Where the companions of a pane are shown in it.
    pub companion_side: CompanionSide,
    /// The size of a companion in percent of its pane (10..90).
    pub companion_size: u8,
    /// How many times a second a change off the screen (the title of a pane in another tab, for example the spinner
    /// of an agent) may redraw the window (0.1..60).
    pub background_redraws: f64,
    pub colors: ColorConfig,
    /// The theme: the colors of the terminal and the UI. `colors` changes it.
    pub theme: crate::theme::ThemeChoice,
    /// Changes the `harmonize` of the theme (`None` = the theme's value).
    pub harmonize: HarmonizeConfig,
    /// Programs may change the palette colors (OSC 4, 10, 11). `false` = the theme's colors stay.
    pub palette_changes: bool,
    /// When a plan of Claude Code plan mode opens a Review tab.
    pub plan_review: PlanReview,
    /// The language of the UI: a code (`ru`), `system`, or a path to a `.json`. `None` = English.
    pub language: Option<String>,
    /// New panes live in the mux server: their programs go on when the window closes.
    pub mux: bool,
    /// Other machines with a mux server (`remotes`).
    pub remotes: Vec<Remote>,
    /// The profile for new tabs and splits. `None` = the first profile.
    pub default_profile: Option<String>,
    /// Profiles from the config. Empty = fterm finds them itself.
    pub profiles: Vec<Profile>,
    pub keys: Keymap,
    /// Copy mode: its keys and options.
    pub copy_mode: CopyModeConfig,
    pub commands: Vec<UserCommand>,
    /// The dashboard (Ctrl+Shift+D).
    pub dashboard: DashboardConfig,
    /// The Agents panel.
    pub agents: AgentsConfig,
    /// Load the shell integration script (PowerShell now; bash and zsh by hand, see the docs).
    pub shell_integration: bool,
    pub notifications: NotificationConfig,
    pub panels: PanelsConfig,
    pub history: HistoryConfig,
    pub confirm_close: ConfirmClose,
    pub restore: Restore,
    pub restore_programs: Rerun,
    pub restore_agents: Rerun,
    /// Lines of old text that a restored pane shows in grey (0 = none).
    pub restore_history: usize,
    /// The GPU that draws the window (used at start).
    pub gpu: GpuConfig,
    /// The folder for the history, the sessions, and the shell scripts (used at start), as written:
    /// a relative path is from the folder of the config file.
    pub data_dir: Option<String>,
    pub api: ApiConfig,
    pub ai: AiConfig,
    /// The Lua function `on_notification` (its number), if there is one.
    pub on_notification: Option<usize>,
    /// The Lua function `on_agent` (its number), if there is one.
    pub on_agent: Option<usize>,
    /// The Lua function `on_agent_row` (its number): the row of an agent in the Agents panel.
    pub on_agent_row: Option<usize>,
    /// The Lua function `on_agent_event` (its number): every step of an agent.
    pub on_agent_event: Option<usize>,
    /// The Lua function `on_history` (its number), if there is one.
    pub on_history: Option<usize>,
    /// The Lua function `on_close_window` (its number), if there is one.
    pub on_close_window: Option<usize>,
    /// The Lua function `window_title` (its number), if there is one.
    pub window_title: Option<usize>,
    /// The Lua function `on_ai_request` (its number), if there is one.
    pub on_ai_request: Option<usize>,
    /// The Lua function `on_restore` (its number), if there is one.
    pub on_restore: Option<usize>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            padding: 6.0,
            scrollback: 10_000,
            braille_style: BrailleStyle::Dots,
            pictures: Pictures::Pixels,
            overlay_size: 80,
            scratch_size: 50,
            window_overlay: None,
            companion_side: CompanionSide::Bottom,
            companion_size: 40,
            background_redraws: 1.0,
            colors: ColorConfig::default(),
            theme: crate::theme::ThemeChoice::Default,
            harmonize: HarmonizeConfig::default(),
            palette_changes: true,
            plan_review: PlanReview::default(),
            language: None,
            mux: false,
            remotes: Vec::new(),
            default_profile: None,
            profiles: Vec::new(),
            keys: Keymap::with_defaults(),
            copy_mode: CopyModeConfig::default(),
            commands: Vec::new(),
            dashboard: DashboardConfig::default(),
            agents: AgentsConfig::default(),
            shell_integration: true,
            notifications: NotificationConfig::default(),
            panels: PanelsConfig::default(),
            history: HistoryConfig::default(),
            confirm_close: ConfirmClose::default(),
            restore: Restore::default(),
            restore_programs: Rerun::default(),
            restore_agents: Rerun::default(),
            restore_history: 200,
            gpu: GpuConfig::default(),
            data_dir: None,
            api: ApiConfig::default(),
            ai: AiConfig::default(),
            on_notification: None,
            on_agent: None,
            on_agent_row: None,
            on_agent_event: None,
            on_history: None,
            on_close_window: None,
            window_title: None,
            on_ai_request: None,
            on_restore: None,
        }
    }
}

/// A call from a Lua function to fterm. The app runs them after the function ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiCall {
    Spawn {
        profile: Option<String>,
        place: SpawnWhere,
    },
    SendText(String),
    Notify {
        title: String,
        body: String,
        level: String,
    },
    Copy(String),
    Action(BuiltinAction),
    /// `fterm.copy_mode`: a step of copy mode in the active pane (copy mode starts first when it is off).
    CopyMode(CopyCommand),
    /// `fterm.run`: programs with their arguments, one after another (no shell).
    Run {
        steps: Vec<Vec<String>>,
        overlay: bool,
    },
    /// `fterm.open`: a link.
    Open(String),
    /// A color for the tab of a pane (`None` = the active pane); `color: None` = no color.
    SetTabColor {
        pane: Option<u64>,
        color: Option<[u8; 3]>,
    },
}

/// A loaded config and its Lua state (Lua functions in the config need it).
pub struct LoadedConfig {
    pub config: Config,
    /// Lua functions from the config. `Action::Lua(i)` is `functions[i]`.
    functions: Vec<Function>,
    /// The `fterm` object that Lua functions get.
    api: Option<Table>,
    /// Calls from Lua functions, collected while a function runs.
    queue: Rc<RefCell<Vec<ApiCall>>>,
    /// Keeps the Lua state alive.
    _lua: Option<Lua>,
}

impl LoadedConfig {
    /// The config with no file.
    pub fn defaults() -> Self {
        Self {
            config: Config::default(),
            functions: Vec::new(),
            api: None,
            queue: Rc::default(),
            _lua: None,
        }
    }

    /// Asks `on_notification` (if the config has it) what to do with a notification.
    pub fn filter_notification(&self, input: &NotifyIn) -> Result<NotifyOut, String> {
        let keep = || NotifyOut::Keep {
            title: input.title.clone(),
            body: input.body.clone(),
            level: input.level.clone(),
            os: None,
            toast: None,
        };
        let (Some(index), Some(lua), Some(api)) =
            (self.config.on_notification, &self._lua, &self.api)
        else {
            return Ok(keep());
        };
        let Some(function) = self.functions.get(index) else {
            return Ok(keep());
        };
        let run = || -> mlua::Result<NotifyOut> {
            let n = lua.create_table()?;
            n.set("title", input.title.as_str())?;
            n.set("body", input.body.as_str())?;
            n.set("level", input.level.as_str())?;
            n.set("source", input.source.as_str())?;
            n.set("pane", input.pane)?;
            match function.call::<Value>((n, api.clone()))? {
                Value::Nil | Value::Boolean(false) => Ok(NotifyOut::Drop),
                Value::Table(out) => Ok(NotifyOut::Keep {
                    title: out.get::<Option<String>>("title")?.unwrap_or_default(),
                    body: out.get::<Option<String>>("body")?.unwrap_or_default(),
                    level: out
                        .get::<Option<String>>("level")?
                        .unwrap_or_else(|| input.level.clone()),
                    os: out.get::<Option<bool>>("os")?,
                    toast: out.get::<Option<bool>>("toast")?,
                }),
                other => Err(mlua::Error::runtime(format!(
                    "on_notification must return a table or nil, got {}",
                    other.type_name()
                ))),
            }
        };
        run().map_err(|err| err.to_string())
    }

    /// Tells `on_agent` (if the config has it) that an agent changed its state.
    pub fn on_agent(&self, input: &AgentIn) -> Result<AgentOut, String> {
        let (Some(index), Some(lua), Some(api)) = (self.config.on_agent, &self._lua, &self.api)
        else {
            return Ok(AgentOut {
                notify: true,
                calls: Vec::new(),
            });
        };
        let Some(function) = self.functions.get(index) else {
            return Err(format!("no Lua function number {index}"));
        };
        self.queue.borrow_mut().clear();
        let run = || -> mlua::Result<bool> {
            let a = lua.create_table()?;
            a.set("pane", input.pane)?;
            a.set("state", input.state.as_str())?;
            a.set("previous", input.previous.as_deref())?;
            a.set("message", input.message.as_str())?;
            a.set("name", input.name.as_str())?;
            let result = function.call::<Value>((a, api.clone()))?;
            Ok(!matches!(result, Value::Boolean(false)))
        };
        let notify = run().map_err(|err| err.to_string())?;
        Ok(AgentOut {
            notify,
            calls: std::mem::take(&mut *self.queue.borrow_mut()),
        })
    }

    /// Asks `on_agent_row` (if the config has it) for the row of an agent. `None` = the default row.
    pub fn on_agent_row(&self, input: &AgentRowIn) -> Result<Option<AgentRowOut>, String> {
        let (Some(index), Some(lua), Some(api)) = (self.config.on_agent_row, &self._lua, &self.api)
        else {
            return Ok(None);
        };
        let Some(function) = self.functions.get(index) else {
            return Err(format!("no Lua function number {index}"));
        };
        let run = || -> mlua::Result<Option<AgentRowOut>> {
            let a = lua.create_table()?;
            a.set("pane", input.pane)?;
            a.set("name", input.name.as_str())?;
            a.set("state", input.state.as_str())?;
            a.set("question", input.question.as_deref())?;
            let todos = lua.create_table()?;
            todos.set("done", input.todos_done)?;
            todos.set("total", input.todos_total)?;
            todos.set("current", input.todo_current.as_deref())?;
            a.set("todos", todos)?;
            a.set("cost", input.cost)?;
            a.set("context", input.context)?;
            a.set("files", input.files)?;
            a.set("last", input.last.as_deref())?;
            match function.call::<Value>((a, api.clone()))? {
                Value::Table(t) => Ok(Some(AgentRowOut {
                    title: t.get::<Option<String>>("title")?,
                    detail: t.get::<Option<String>>("detail")?,
                    right: t.get::<Option<String>>("right")?,
                })),
                _ => Ok(None),
            }
        };
        run().map_err(|err| err.to_string())
    }

    /// Tells `on_agent_event` (if the config has it) about a step of an agent. Gives what it asked fterm to do.
    pub fn on_agent_event(&self, input: &AgentEventIn) -> Result<Vec<ApiCall>, String> {
        let (Some(index), Some(lua), Some(api)) =
            (self.config.on_agent_event, &self._lua, &self.api)
        else {
            return Ok(Vec::new());
        };
        let Some(function) = self.functions.get(index) else {
            return Err(format!("no Lua function number {index}"));
        };
        self.queue.borrow_mut().clear();
        let run = || -> mlua::Result<()> {
            let e = lua.create_table()?;
            e.set("pane", input.pane)?;
            e.set("kind", input.kind.as_str())?;
            e.set("tool", input.tool.as_deref())?;
            e.set("file", input.file.as_deref())?;
            e.set("command", input.command.as_deref())?;
            e.set("text", input.text.as_deref())?;
            e.set("cost", input.cost)?;
            e.set("context", input.context)?;
            function.call::<Value>((e, api.clone()))?;
            Ok(())
        };
        run().map_err(|err| err.to_string())?;
        Ok(std::mem::take(&mut *self.queue.borrow_mut()))
    }

    /// Asks `on_history` (if the config has it) about a command before it goes to the history.
    /// `None` = do not save it; `Some(text)` = save this text (maybe changed).
    pub fn on_history(&self, input: &HistoryIn) -> Result<Option<String>, String> {
        let (Some(index), Some(lua), Some(api)) = (self.config.on_history, &self._lua, &self.api)
        else {
            return Ok(Some(input.cmd.clone()));
        };
        let Some(function) = self.functions.get(index) else {
            return Err(format!("no Lua function number {index}"));
        };
        let run = || -> mlua::Result<Option<String>> {
            let h = lua.create_table()?;
            h.set("cmd", input.cmd.as_str())?;
            h.set("cwd", input.cwd.as_deref())?;
            h.set("exit", input.exit)?;
            h.set("shell", input.shell.as_str())?;
            match function.call::<Value>((h, api.clone()))? {
                Value::Nil | Value::Boolean(true) => Ok(Some(input.cmd.clone())),
                Value::Boolean(false) => Ok(None),
                Value::Table(out) => Ok(out.get::<Option<String>>("cmd")?),
                other => Err(mlua::Error::runtime(format!(
                    "on_history must return a table, false, or nil, got {}",
                    other.type_name()
                ))),
            }
        };
        run().map_err(|err| err.to_string())
    }

    /// Asks `on_close_window` (if the config has it): `Some(true)` = close now, `Some(false)` = do not close,
    /// `None` = the `confirm_close` rule.
    pub fn on_close_window(&self, input: &CloseIn) -> Result<Option<bool>, String> {
        let (Some(index), Some(lua), Some(api)) =
            (self.config.on_close_window, &self._lua, &self.api)
        else {
            return Ok(None);
        };
        let Some(function) = self.functions.get(index) else {
            return Err(format!("no Lua function number {index}"));
        };
        let run = || -> mlua::Result<Option<bool>> {
            let info = lua.create_table()?;
            info.set("tabs", input.tabs)?;
            info.set("panes", input.panes)?;
            let running = lua.create_table()?;
            for (i, (tab, program)) in input.running.iter().enumerate() {
                let row = lua.create_table()?;
                row.set("tab", *tab)?;
                row.set("program", program.as_str())?;
                running.set(i + 1, row)?;
            }
            info.set("running", running)?;
            let agents = lua.create_table()?;
            for (i, (tab, name, state)) in input.agents.iter().enumerate() {
                let row = lua.create_table()?;
                row.set("tab", *tab)?;
                row.set("name", name.as_str())?;
                row.set("state", state.as_str())?;
                agents.set(i + 1, row)?;
            }
            info.set("agents", agents)?;
            match function.call::<Value>((info, api.clone()))? {
                Value::Nil => Ok(None),
                Value::Boolean(close) => Ok(Some(close)),
                other => Err(mlua::Error::runtime(format!(
                    "on_close_window must return true, false, or nil, got {}",
                    other.type_name()
                ))),
            }
        };
        run().map_err(|err| err.to_string())
    }

    /// Asks `window_title` (if the config has it) for the window title. `None` = the default title.
    pub fn window_title(&self, input: &TitleIn) -> Result<Option<String>, String> {
        let (Some(index), Some(lua), Some(api)) = (self.config.window_title, &self._lua, &self.api)
        else {
            return Ok(None);
        };
        let Some(function) = self.functions.get(index) else {
            return Err(format!("no Lua function number {index}"));
        };
        let run = || -> mlua::Result<Option<String>> {
            let t = lua.create_table()?;
            t.set("tab", input.tab)?;
            t.set("tabs", input.tabs)?;
            t.set("title", input.title.as_str())?;
            t.set("agent", input.agent.as_deref())?;
            t.set("waiting", input.waiting)?;
            t.set("failed", input.failed)?;
            t.set("default", input.default.as_str())?;
            match function.call::<Value>((t, api.clone()))? {
                Value::Nil => Ok(None),
                Value::String(text) => Ok(Some(text.to_string_lossy())),
                other => Err(mlua::Error::runtime(format!(
                    "window_title must return a string or nil, got {}",
                    other.type_name()
                ))),
            }
        };
        run().map_err(|err| err.to_string())
    }

    /// Asks `on_ai_request` (if the config has it) about a question. `None` = do not send it;
    /// `Some(text)` = send this text.
    pub fn on_ai_request(&self, input: &AiRequestIn) -> Result<Option<String>, String> {
        let (Some(index), Some(lua), Some(api)) =
            (self.config.on_ai_request, &self._lua, &self.api)
        else {
            return Ok(Some(input.text.clone()));
        };
        let Some(function) = self.functions.get(index) else {
            return Err(format!("no Lua function number {index}"));
        };
        let run = || -> mlua::Result<Option<String>> {
            let r = lua.create_table()?;
            r.set("question", input.question.as_str())?;
            r.set("text", input.text.as_str())?;
            r.set("provider", input.provider.as_str())?;
            r.set("model", input.model.as_str())?;
            match function.call::<Value>((r, api.clone()))? {
                Value::Nil | Value::Boolean(true) => Ok(Some(input.text.clone())),
                Value::Boolean(false) => Ok(None),
                Value::Table(out) => Ok(out.get::<Option<String>>("text")?),
                other => Err(mlua::Error::runtime(format!(
                    "on_ai_request must return a table, false, or nil, got {}",
                    other.type_name()
                ))),
            }
        };
        run().map_err(|err| err.to_string())
    }

    /// Asks `on_restore` (if the config has it) about a session before it opens (the session file as JSON).
    /// `None` = do not restore it; `Some(session)` = restore this.
    pub fn on_restore(
        &self,
        session: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>, String> {
        use mlua::LuaSerdeExt;
        let (Some(index), Some(lua), Some(api)) = (self.config.on_restore, &self._lua, &self.api)
        else {
            return Ok(Some(session.clone()));
        };
        let Some(function) = self.functions.get(index) else {
            return Err(format!("no Lua function number {index}"));
        };
        let run = || -> mlua::Result<Option<serde_json::Value>> {
            let table = lua.to_value(session)?;
            match function.call::<Value>((table, api.clone()))? {
                Value::Nil | Value::Boolean(true) => Ok(Some(session.clone())),
                Value::Boolean(false) => Ok(None),
                value @ Value::Table(_) => Ok(Some(lua.from_value(value)?)),
                other => Err(mlua::Error::runtime(format!(
                    "on_restore must return a table, true, false, or nil, got {}",
                    other.type_name()
                ))),
            }
        };
        run().map_err(|err| err.to_string())
    }

    /// Runs the Lua function `index` (from `Action::Lua`) and returns what it asked fterm to do.
    pub fn call(&self, index: usize) -> Result<Vec<ApiCall>, String> {
        let (Some(function), Some(api)) = (self.functions.get(index), &self.api) else {
            return Err(format!("no Lua function number {index}"));
        };
        self.queue.borrow_mut().clear();
        function
            .call::<()>(api.clone())
            .map_err(|err| err.to_string())?;
        Ok(std::mem::take(&mut *self.queue.borrow_mut()))
    }

    /// Calls Lua function number `index` with `(fterm, item)`: the item of a list tile, `{ line = ..., fields = {...} }`.
    pub fn call_with_item(
        &self,
        index: usize,
        line: &str,
        fields: &[String],
    ) -> Result<Vec<ApiCall>, String> {
        let (Some(function), Some(api), Some(lua)) =
            (self.functions.get(index), &self.api, &self._lua)
        else {
            return Err(format!("no Lua function number {index}"));
        };
        self.queue.borrow_mut().clear();
        let item = lua.create_table().map_err(|err| err.to_string())?;
        item.set("line", line).map_err(|err| err.to_string())?;
        let list = lua
            .create_sequence_from(fields.iter().map(String::as_str))
            .map_err(|err| err.to_string())?;
        item.set("fields", list).map_err(|err| err.to_string())?;
        function
            .call::<()>((api.clone(), item))
            .map_err(|err| err.to_string())?;
        Ok(std::mem::take(&mut *self.queue.borrow_mut()))
    }

    /// Calls the Lua action `index` of a copy mode key with `(fterm, cm)`.
    pub fn call_copy(&self, index: usize, input: &CopyModeIn) -> Result<Vec<ApiCall>, String> {
        let (Some(function), Some(api), Some(lua)) =
            (self.functions.get(index), &self.api, &self._lua)
        else {
            return Err(format!("no Lua function number {index}"));
        };
        self.queue.borrow_mut().clear();
        let run = || -> mlua::Result<()> {
            let cm = lua.create_table()?;
            cm.set("selection", input.selection.as_deref())?;
            cm.set("line", input.line.as_str())?;
            cm.set("word", input.word.as_deref())?;
            cm.set("row", input.row)?;
            cm.set("column", input.column)?;
            cm.set("mode", input.mode.as_str())?;
            cm.set("search", input.search.as_deref())?;
            function.call::<()>((api.clone(), cm))
        };
        run().map_err(|err| err.to_string())?;
        Ok(std::mem::take(&mut *self.queue.borrow_mut()))
    }
}

/// A copy mode action: a name (`"select_word"`) or a jump (`{ jump = "f", char = "x" }`).
fn copy_command(value: &Value) -> Result<CopyCommand, String> {
    match value {
        Value::String(name) => {
            let name = name.to_string_lossy();
            CopyCommand::from_name(&name).ok_or_else(|| {
                format!(
                    "unknown copy mode action `{name}`; the actions are {}",
                    CopyCommand::names().collect::<Vec<_>>().join(", ")
                )
            })
        }
        Value::Table(t) => {
            let jump: Option<String> = t.get("jump").map_err(|err| err.to_string())?;
            let ch: Option<String> = t.get("char").map_err(|err| err.to_string())?;
            let kind = jump
                .as_deref()
                .and_then(JumpKind::from_name)
                .ok_or("`jump` must be \"f\", \"F\", \"t\", or \"T\"")?;
            let mut chars = ch.as_deref().unwrap_or("").chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(CopyCommand::JumpTo(kind, c)),
                _ => Err("`char` must be one char".to_owned()),
            }
        }
        other => Err(format!(
            "expected an action name or {{ jump = ..., char = ... }}, got {}",
            other.type_name()
        )),
    }
}

/// `{ "gh", "pr", "list" }` (one program) or `{ { "a" }, { "b", "c" } }` (one after another): the steps of a run.
fn run_steps(value: &Value, path: &str) -> Result<Vec<Vec<String>>, String> {
    let Value::Table(table) = value else {
        return Err(format!(
            "{path}: must be a list of arguments, like {{ \"gh\", \"pr\", \"view\", \"{{1}}\" }} (a text would go through the shell), got {}",
            value.type_name()
        ));
    };
    let items = list(table);
    if items.is_empty() {
        return Err(format!("{path}: empty"));
    }
    let words = |t: &Table, path: &str| -> Result<Vec<String>, String> {
        let words: Vec<String> = list(t)
            .into_iter()
            .map(|(i, v)| match v {
                Value::String(s) => Ok(s.to_string_lossy()),
                Value::Integer(n) => Ok(n.to_string()),
                Value::Number(n) => Ok(n.to_string()),
                other => Err(format!(
                    "{path}[{i}]: expected a text, got {}",
                    other.type_name()
                )),
            })
            .collect::<Result<_, _>>()?;
        if words.first().is_none_or(|w| w.trim().is_empty()) {
            return Err(format!("{path}: empty"));
        }
        Ok(words)
    };
    if items.iter().all(|(_, v)| matches!(v, Value::Table(_))) {
        items
            .iter()
            .map(|(i, v)| match v {
                Value::Table(t) => words(t, &format!("{path}[{i}]")),
                _ => unreachable!("checked above"),
            })
            .collect()
    } else {
        Ok(vec![words(table, path)?])
    }
}

/// Runs a config script. `name` is shown in error messages (for example the file path).
pub fn load_str(source: &str, name: &str) -> Result<LoadedConfig, String> {
    let lua = Lua::new();
    // Luau sandbox: no files, no programs, and the built-in tables cannot be changed.
    lua.sandbox(true).map_err(|err| err.to_string())?;
    let value: Value = lua
        .load(source)
        .set_name(format!("={name}"))
        .eval()
        .map_err(|err| err.to_string())?;
    let Value::Table(root) = value else {
        return Err(format!(
            "{name}: the config must return a table, got {}",
            value.type_name()
        ));
    };

    let queue: Rc<RefCell<Vec<ApiCall>>> = Rc::default();
    let api = make_api(&lua, &queue).map_err(|err| err.to_string())?;
    let mut reader = Reader {
        functions: Vec::new(),
    };
    let config = reader.config(&root)?;
    Ok(LoadedConfig {
        config,
        functions: reader.functions,
        api: Some(api),
        queue,
        _lua: Some(lua),
    })
}

/// The `fterm` object for Lua functions. Each call is put into `queue`.
fn make_api(lua: &Lua, queue: &Rc<RefCell<Vec<ApiCall>>>) -> mlua::Result<Table> {
    let api = lua.create_table()?;

    let q = queue.clone();
    api.set(
        "spawn",
        lua.create_function(
            move |_, (profile, options): (Option<String>, Option<Table>)| {
                let split = match &options {
                    Some(options) => options.get::<Option<String>>("split")?,
                    None => None,
                };
                let place = spawn_place(split.as_deref()).map_err(mlua::Error::runtime)?;
                q.borrow_mut().push(ApiCall::Spawn { profile, place });
                Ok(())
            },
        )?,
    )?;
    let q = queue.clone();
    api.set(
        "send_text",
        lua.create_function(move |_, text: String| {
            q.borrow_mut().push(ApiCall::SendText(text));
            Ok(())
        })?,
    )?;
    let q = queue.clone();
    api.set(
        "notify",
        lua.create_function(move |_, value: Value| {
            let call = match value {
                Value::String(text) => ApiCall::Notify {
                    title: String::new(),
                    body: text.to_string_lossy(),
                    level: "info".to_owned(),
                },
                Value::Table(t) => ApiCall::Notify {
                    title: t.get::<Option<String>>("title")?.unwrap_or_default(),
                    body: t.get::<Option<String>>("body")?.unwrap_or_default(),
                    level: t
                        .get::<Option<String>>("level")?
                        .unwrap_or_else(|| "info".to_owned()),
                },
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "notify: expected a string or a table, got {}",
                        other.type_name()
                    )));
                }
            };
            q.borrow_mut().push(call);
            Ok(())
        })?,
    )?;
    let q = queue.clone();
    api.set(
        "set_tab_color",
        lua.create_function(move |_, (color, pane): (String, Option<u64>)| {
            let color = crate::colors::tab_color(&color).map_err(mlua::Error::runtime)?;
            q.borrow_mut().push(ApiCall::SetTabColor { pane, color });
            Ok(())
        })?,
    )?;
    let q = queue.clone();
    api.set(
        "copy",
        lua.create_function(move |_, text: String| {
            q.borrow_mut().push(ApiCall::Copy(text));
            Ok(())
        })?,
    )?;
    let q = queue.clone();
    api.set(
        "run",
        lua.create_function(move |_, (steps, options): (Value, Option<Table>)| {
            let steps = run_steps(&steps, "fterm.run").map_err(mlua::Error::runtime)?;
            let overlay = match &options {
                Some(o) => o.get::<Option<String>>("show")?.as_deref() == Some("overlay"),
                None => false,
            };
            q.borrow_mut().push(ApiCall::Run { steps, overlay });
            Ok(())
        })?,
    )?;
    let q = queue.clone();
    api.set(
        "open",
        lua.create_function(move |_, url: String| {
            q.borrow_mut().push(ApiCall::Open(url));
            Ok(())
        })?,
    )?;
    let q = queue.clone();
    api.set(
        "action",
        lua.create_function(move |_, name: String| {
            let action = BuiltinAction::from_name(&name)
                .ok_or_else(|| mlua::Error::runtime(format!("unknown action `{name}`")))?;
            q.borrow_mut().push(ApiCall::Action(action));
            Ok(())
        })?,
    )?;
    let q = queue.clone();
    api.set(
        "copy_mode",
        lua.create_function(move |_, value: Value| {
            let command = copy_command(&value).map_err(mlua::Error::runtime)?;
            q.borrow_mut().push(ApiCall::CopyMode(command));
            Ok(())
        })?,
    )?;
    Ok(api)
}

fn spawn_place(split: Option<&str>) -> Result<SpawnWhere, String> {
    match split {
        None | Some("tab") => Ok(SpawnWhere::Tab),
        Some("right") => Ok(SpawnWhere::SplitRight),
        Some("down") => Ok(SpawnWhere::SplitDown),
        Some(other) => Err(format!(
            "split must be \"right\", \"down\", or \"tab\", got `{other}`"
        )),
    }
}

/// `harmonize = { strength = 0.6, min_contrast = 4.5 }` in the config: changes the values of the theme.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HarmonizeConfig {
    pub strength: Option<f32>,
    pub min_contrast: Option<f32>,
}

/// `theme`: a name, JSON text, `{ light = ..., dark = ... }`, or a theme as a Lua table.
fn theme_choice(root: &Table) -> Result<crate::theme::ThemeChoice, String> {
    use crate::theme::{Theme, ThemeChoice};
    let inline = |json: &serde_json::Value| {
        Theme::from_json(json)
            .map(|t| ThemeChoice::Inline(Box::new(t)))
            .map_err(|err| format!("theme: {err}"))
    };
    match root
        .get::<Value>("theme")
        .map_err(|err| format!("theme: {err}"))?
    {
        Value::Nil => Ok(ThemeChoice::Default),
        Value::String(text) => {
            let text = text.to_string_lossy();
            if text.trim_start().starts_with('{') {
                let json: serde_json::Value =
                    serde_json::from_str(&text).map_err(|err| format!("theme: bad JSON: {err}"))?;
                inline(&json)
            } else {
                Ok(ThemeChoice::Named(text.trim().to_owned()))
            }
        }
        Value::Table(table) => {
            let light = string_field(&table, "light", "theme.light")?;
            let dark = string_field(&table, "dark", "theme.dark")?;
            if light.is_some() || dark.is_some() {
                return Ok(ThemeChoice::System {
                    light: light.unwrap_or_else(|| "Catppuccin Latte".to_owned()),
                    dark: dark.unwrap_or_else(|| crate::theme::DEFAULT_THEME.to_owned()),
                });
            }
            inline(&lua_to_json(&Value::Table(table), "theme")?)
        }
        other => Err(format!(
            "theme: expected a name, a table, or JSON text, got {}",
            other.type_name()
        )),
    }
}

/// A Lua value as JSON: a table with keys 1..n is a list, other tables are objects.
fn lua_to_json(value: &Value, path: &str) -> Result<serde_json::Value, String> {
    use serde_json::Value as Json;
    Ok(match value {
        Value::Nil => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(n) => Json::from(*n),
        Value::Number(n) => Json::from(*n),
        Value::String(s) => Json::String(s.to_string_lossy()),
        Value::Table(table) => {
            let items = list(table);
            let count = table.pairs::<Value, Value>().count();
            if !items.is_empty() && items.len() == count {
                Json::Array(
                    items
                        .iter()
                        .map(|(i, v)| lua_to_json(v, &format!("{path}[{i}]")))
                        .collect::<Result<_, _>>()?,
                )
            } else {
                let mut map = serde_json::Map::new();
                for pair in table.pairs::<String, Value>() {
                    let (key, v) = pair.map_err(|err| format!("{path}: {err}"))?;
                    let json = lua_to_json(&v, &format!("{path}.{key}"))?;
                    map.insert(key, json);
                }
                Json::Object(map)
            }
        }
        other => {
            return Err(format!(
                "{path}: expected a string, a number, or a table, got {}",
                other.type_name()
            ));
        }
    })
}

/// Reads the config table. Error messages have the path to the bad field.
struct Reader {
    functions: Vec<Function>,
}

impl Reader {
    /// An event function like `on_notification`: keeps it and gives its number.
    fn hook(&mut self, root: &Table, name: &str) -> Result<Option<usize>, String> {
        match root
            .get::<Value>(name)
            .map_err(|err| format!("{name}: {err}"))?
        {
            Value::Nil => Ok(None),
            Value::Function(function) => {
                self.functions.push(function);
                Ok(Some(self.functions.len() - 1))
            }
            other => Err(format!(
                "{name}: expected a function, got {}",
                other.type_name()
            )),
        }
    }

    fn config(&mut self, root: &Table) -> Result<Config, String> {
        let mut config = Config::default();
        if let Some(font) = table_field(root, "font", "font")? {
            if let Some(size) = number_field(&font, "size", "font.size")? {
                if !(4.0..=200.0).contains(&size) {
                    return Err(format!("font.size: must be between 4 and 200, got {size}"));
                }
                config.font_size = size as f32;
            }
        }
        if let Some(padding) = number_field(root, "padding", "padding")? {
            config.padding = padding.max(0.0) as f32;
        }
        if let Some(lines) = number_field(root, "restore_history", "restore_history")? {
            config.restore_history = lines.max(0.0) as usize;
        }
        if let Some(lines) = number_field(root, "scrollback", "scrollback")? {
            config.scrollback = lines.max(0.0) as usize;
        }
        if let Some(style) = string_field(root, "braille_style", "braille_style")? {
            config.braille_style = match style.as_str() {
                "pixels" => BrailleStyle::Pixels,
                "dots" => BrailleStyle::Dots,
                other => {
                    return Err(format!(
                        "braille_style: must be \"pixels\" or \"dots\", got `{other}`"
                    ));
                }
            };
        }
        if let Some(size) = number_field(root, "overlay_size", "overlay_size")? {
            config.overlay_size = percent(size);
        }
        if let Some(size) = number_field(root, "scratch_size", "scratch_size")? {
            config.scratch_size = percent(size);
        }
        if let Some(command) = string_field(root, "window_overlay", "window_overlay")? {
            config.window_overlay = Some(command.trim().to_owned()).filter(|c| !c.is_empty());
        }
        if let Some(rate) = number_field(root, "background_redraws", "background_redraws")? {
            config.background_redraws = rate.clamp(0.1, 60.0);
        }
        if let Some(side) = string_field(root, "companion_side", "companion_side")? {
            config.companion_side = match side.as_str() {
                "bottom" => CompanionSide::Bottom,
                "right" => CompanionSide::Right,
                other => {
                    return Err(format!(
                        "companion_side: must be \"bottom\" or \"right\", got `{other}`"
                    ));
                }
            };
        }
        if let Some(size) = number_field(root, "companion_size", "companion_size")? {
            config.companion_size = size.round().clamp(10.0, 90.0) as u8;
        }
        if let Some(mode) = string_field(root, "pictures", "pictures")? {
            config.pictures = match mode.as_str() {
                "pixels" => Pictures::Pixels,
                "braille" => Pictures::Braille,
                "off" => Pictures::Off,
                other => {
                    return Err(format!(
                        "pictures: must be \"pixels\", \"braille\", or \"off\", got `{other}`"
                    ));
                }
            };
        }
        if let Some(colors) = table_field(root, "colors", "colors")? {
            config.colors = self.colors(&colors)?;
        }
        config.theme = theme_choice(root)?;
        if let Some(on) = bool_field(root, "palette_changes", "palette_changes")? {
            config.palette_changes = on;
        }
        config.language = string_field(root, "language", "language")?;
        if let Some(on) = bool_field(root, "mux", "mux")? {
            config.mux = on;
        }
        if let Some(mode) = string_field(root, "plan_review", "plan_review")? {
            config.plan_review = match mode.as_str() {
                "always" => PlanReview::Always,
                "ask" => PlanReview::Ask,
                "never" => PlanReview::Never,
                other => {
                    return Err(format!(
                        "plan_review: must be \"always\", \"ask\", or \"never\", got `{other}`"
                    ));
                }
            };
        }
        if let Some(table) = table_field(root, "harmonize", "harmonize")? {
            let number = |key: &str| -> Result<Option<f32>, String> {
                let path = format!("harmonize.{key}");
                Ok(number_field(&table, key, &path)?.map(|n| n as f32))
            };
            config.harmonize = HarmonizeConfig {
                strength: number("strength")?
                    .map(crate::theme::check_strength)
                    .transpose()
                    .map_err(|e| format!("harmonize.strength: {e}"))?,
                min_contrast: number("min_contrast")?
                    .map(crate::theme::check_contrast)
                    .transpose()
                    .map_err(|e| format!("harmonize.min_contrast: {e}"))?,
            };
        }
        config.default_profile = string_field(root, "default_profile", "default_profile")?;
        if let Some(on) = bool_field(root, "shell_integration", "shell_integration")? {
            config.shell_integration = on;
        }
        if let Some(table) = table_field(root, "notifications", "notifications")? {
            config.notifications = notifications(&table)?;
        }
        if let Some(table) = table_field(root, "panels", "panels")? {
            config.panels = panels(&table)?;
        }
        config.on_notification = self.hook(root, "on_notification")?;
        config.on_agent = self.hook(root, "on_agent")?;
        config.on_agent_row = self.hook(root, "on_agent_row")?;
        config.on_agent_event = self.hook(root, "on_agent_event")?;
        config.on_history = self.hook(root, "on_history")?;
        config.on_close_window = self.hook(root, "on_close_window")?;
        config.window_title = self.hook(root, "window_title")?;
        config.on_ai_request = self.hook(root, "on_ai_request")?;
        config.on_restore = self.hook(root, "on_restore")?;
        if let Some(table) = table_field(root, "ai", "ai")? {
            config.ai = ai(&table)?;
        }
        if let Some(table) = table_field(root, "api", "api")? {
            for key in ["enabled", "ask"] {
                match table
                    .get::<Value>(key)
                    .map_err(|err| format!("api.{key}: {err}"))?
                {
                    Value::Nil => {}
                    Value::Boolean(on) if key == "enabled" => config.api.enabled = on,
                    Value::Boolean(on) => config.api.ask = on,
                    other => {
                        return Err(format!(
                            "api.{key}: expected true or false, got {}",
                            other.type_name()
                        ));
                    }
                }
            }
        }
        if let Some(text) = string_field(root, "restore", "restore")? {
            config.restore = match text.as_str() {
                "ask" => Restore::Ask,
                "always" => Restore::Always,
                "never" => Restore::Never,
                other => {
                    return Err(format!(
                        "restore: must be \"ask\", \"always\", or \"never\", got `{other}`"
                    ));
                }
            };
        }
        if let Some(dir) = string_field(root, "data_dir", "data_dir")? {
            config.data_dir = Some(dir);
        }
        if let Some(gpu) = table_field(root, "gpu", "gpu")? {
            if let Some(text) = string_field(&gpu, "backend", "gpu.backend")? {
                config.gpu.backend = match text.as_str() {
                    "auto" => GpuBackend::Auto,
                    "dx12" => GpuBackend::Dx12,
                    "vulkan" => GpuBackend::Vulkan,
                    "gl" => GpuBackend::Gl,
                    "metal" => GpuBackend::Metal,
                    other => {
                        return Err(format!(
                            "gpu.backend: must be \"auto\", \"dx12\", \"vulkan\", \"gl\", or \"metal\", got `{other}`"
                        ));
                    }
                };
            }
            if let Some(text) = string_field(&gpu, "power", "gpu.power")? {
                config.gpu.power = match text.as_str() {
                    "high" => GpuPower::High,
                    "low" => GpuPower::Low,
                    other => {
                        return Err(format!(
                            "gpu.power: must be \"high\" or \"low\", got `{other}`"
                        ));
                    }
                };
            }
        }
        for key in ["restore_programs", "restore_agents"] {
            if let Some(text) = string_field(root, key, key)? {
                let value = match text.as_str() {
                    "prompt" => Rerun::Prompt,
                    "run" => Rerun::Run,
                    "never" => Rerun::Never,
                    other => {
                        return Err(format!(
                            "{key}: must be \"prompt\", \"run\", or \"never\", got `{other}`"
                        ));
                    }
                };
                if key == "restore_programs" {
                    config.restore_programs = value;
                } else {
                    config.restore_agents = value;
                }
            }
        }
        if let Some(text) = string_field(root, "confirm_close", "confirm_close")? {
            config.confirm_close = match text.as_str() {
                "running" => ConfirmClose::Running,
                "always" => ConfirmClose::Always,
                "never" => ConfirmClose::Never,
                other => {
                    return Err(format!(
                        "confirm_close: must be \"running\", \"always\", or \"never\", got `{other}`"
                    ));
                }
            };
        }
        if let Some(table) = table_field(root, "history", "history")? {
            config.history = history(&table)?;
        }
        if let Some(remotes) = table_field(root, "remotes", "remotes")? {
            for (i, value) in list(&remotes) {
                let path = format!("remotes[{i}]");
                let Value::Table(table) = value else {
                    return Err(format!(
                        "{path}: expected a table, got {}",
                        value.type_name()
                    ));
                };
                config.remotes.push(remote(&table, &path)?);
            }
        }
        if let Some(profiles) = table_field(root, "profiles", "profiles")? {
            for (i, value) in list(&profiles) {
                let path = format!("profiles[{i}]");
                let Value::Table(table) = value else {
                    return Err(format!(
                        "{path}: expected a table, got {}",
                        value.type_name()
                    ));
                };
                config.profiles.push(profile(&table, &path)?);
            }
        }
        if let Some(keys) = table_field(root, "keys", "keys")? {
            for (i, value) in list(&keys) {
                let path = format!("keys[{i}]");
                let Value::Table(table) = value else {
                    return Err(format!(
                        "{path}: expected a table, got {}",
                        value.type_name()
                    ));
                };
                let key = string_field(&table, "key", &format!("{path}.key"))?
                    .ok_or_else(|| format!("{path}.key: missing"))?;
                let chord = KeyChord::parse(&key).map_err(|err| format!("{path}.key: {err}"))?;
                let action = self.action(&table, &format!("{path}.action"))?;
                config.keys.bind(chord, action);
            }
        }
        if let Some(cm) = table_field(root, "copy_mode", "copy_mode")? {
            if let Some(exit) = bool_field(&cm, "exit_after_copy", "copy_mode.exit_after_copy")? {
                config.copy_mode.exit_after_copy = exit;
            }
            if let Some(bar) = bool_field(&cm, "status_bar", "copy_mode.status_bar")? {
                config.copy_mode.status_bar = bar;
            }
            if let Some(keys) = table_field(&cm, "keys", "copy_mode.keys")? {
                for (i, value) in list(&keys) {
                    let path = format!("copy_mode.keys[{i}]");
                    let Value::Table(table) = value else {
                        return Err(format!(
                            "{path}: expected a table, got {}",
                            value.type_name()
                        ));
                    };
                    let key = string_field(&table, "key", &format!("{path}.key"))?
                        .ok_or_else(|| format!("{path}.key: missing"))?;
                    let keys = parse_keys(&key).map_err(|err| format!("{path}.key: {err}"))?;
                    let binding = self.copy_binding(&table, &format!("{path}.action"))?;
                    config.copy_mode.keys.bind(keys, binding);
                }
            }
        }
        if let Some(agents) = table_field(root, "agents", "agents")? {
            if let Some(history) = bool_field(&agents, "history", "agents.history")? {
                config.agents.history = history;
            }
            if let Some(show) = table_field(&agents, "show", "agents.show")? {
                let mut parts = Vec::new();
                for (i, value) in list(&show) {
                    let Value::String(part) = value else {
                        return Err(format!(
                            "agents.show[{i}]: expected a text, got {}",
                            value.type_name()
                        ));
                    };
                    let part = part.to_string_lossy();
                    if !AGENT_PARTS.contains(&part.as_str()) {
                        return Err(format!(
                            "agents.show[{i}]: `{part}` is not a part; the parts are {}",
                            AGENT_PARTS.join(", ")
                        ));
                    }
                    parts.push(part);
                }
                config.agents.show = parts;
            }
        }
        if let Some(dash) = table_field(root, "dashboard", "dashboard")? {
            config.dashboard = self.dashboard(&dash)?;
        }
        if let Some(commands) = table_field(root, "commands", "commands")? {
            for (i, value) in list(&commands) {
                let path = format!("commands[{i}]");
                let Value::Table(table) = value else {
                    return Err(format!(
                        "{path}: expected a table, got {}",
                        value.type_name()
                    ));
                };
                let name = string_field(&table, "name", &format!("{path}.name"))?
                    .ok_or_else(|| format!("{path}.name: missing"))?;
                let action = self
                    .action(&table, &format!("{path}.action"))?
                    .ok_or_else(|| format!("{path}.action: \"none\" is not a command"))?;
                config.commands.push(UserCommand { name, action });
            }
        }
        Ok(config)
    }

    fn colors(&mut self, table: &Table) -> Result<ColorConfig, String> {
        let mut colors = ColorConfig::default();
        let one = |field: &str| -> Result<Option<Rgb>, String> {
            let path = format!("colors.{field}");
            string_field(table, field, &path)?
                .map(|text| parse_color(&text).map_err(|err| format!("{path}: {err}")))
                .transpose()
        };
        colors.background = one("background")?;
        colors.foreground = one("foreground")?;
        colors.cursor = one("cursor")?;
        colors.selection = one("selection")?;
        for (field, target) in [("ansi", &mut colors.ansi), ("bright", &mut colors.bright)] {
            let Some(list_table) = table_field(table, field, &format!("colors.{field}"))? else {
                continue;
            };
            for i in 1..=8 {
                let path = format!("colors.{field}[{i}]");
                let value: Value = list_table.get(i).map_err(|err| format!("{path}: {err}"))?;
                match value {
                    Value::Nil => {}
                    Value::String(text) => {
                        let text = text.to_string_lossy();
                        target[i - 1] =
                            Some(parse_color(&text).map_err(|err| format!("{path}: {err}"))?);
                    }
                    other => {
                        return Err(format!(
                            "{path}: expected a color string, got {}",
                            other.type_name()
                        ));
                    }
                }
            }
        }
        Ok(colors)
    }

    /// `action` in a key or a command. `Ok(None)` = "none".
    fn dashboard(&mut self, table: &Table) -> Result<DashboardConfig, String> {
        let mut dash = DashboardConfig::default();
        if let Some(n) = number_field(table, "columns", "dashboard.columns")? {
            dash.columns = n.round().clamp(0.0, 12.0) as usize;
        }
        let Some(tiles) = table_field(table, "tiles", "dashboard.tiles")? else {
            return Ok(dash);
        };
        dash.tiles.clear();
        for (i, value) in list(&tiles) {
            let path = format!("dashboard.tiles[{i}]");
            let Value::Table(tile) = value else {
                return Err(format!(
                    "{path}: expected a table, got {}",
                    value.type_name()
                ));
            };
            dash.tiles.push(self.dash_tile(&tile, &path)?);
        }
        Ok(dash)
    }

    fn dash_tile(&mut self, tile: &Table, path: &str) -> Result<DashTile, String> {
        let title = string_field(tile, "title", &format!("{path}.title"))?;
        let panes: Value = tile
            .get("panes")
            .map_err(|err| format!("{path}.panes: {err}"))?;
        match panes {
            Value::Nil => {}
            Value::String(name) => {
                let filter = match name.to_string_lossy().as_str() {
                    "agents" => PaneFilter::Agents,
                    "busy" => PaneFilter::Busy,
                    "all" => PaneFilter::All,
                    other => {
                        return Err(format!(
                            "{path}.panes: must be \"agents\", \"busy\", \"all\", or a table, got `{other}`"
                        ));
                    }
                };
                return Ok(DashTile::Panes(filter));
            }
            Value::Table(m) => {
                let field = |key: &str| -> Result<Option<String>, String> {
                    Ok(string_field(&m, key, &format!("{path}.panes.{key}"))?
                        .map(|t| t.trim().to_lowercase())
                        .filter(|t| !t.is_empty()))
                };
                return Ok(DashTile::Panes(PaneFilter::Match(PaneMatch {
                    program: field("program")?,
                    title: field("title")?,
                    workspace: field("workspace")?,
                    profile: field("profile")?,
                })));
            }
            other => {
                return Err(format!(
                    "{path}.panes: expected a name or a table, got {}",
                    other.type_name()
                ));
            }
        }
        let command_value: Value = tile
            .get("command")
            .map_err(|err| format!("{path}.command: {err}"))?;
        let (command, argv) = match &command_value {
            Value::Nil => (None, None),
            Value::String(s) => (Some(s.to_string_lossy()), None),
            Value::Table(_) => {
                let mut steps = run_steps(&command_value, &format!("{path}.command"))?;
                if steps.len() != 1 {
                    return Err(format!("{path}.command: one program and its arguments"));
                }
                let argv = steps.remove(0);
                (Some(argv.join(" ")), Some(argv))
            }
            other => {
                return Err(format!(
                    "{path}.command: expected a text or a list of arguments, got {}",
                    other.type_name()
                ));
            }
        };
        if let Some(command) = command {
            let every = match number_field(tile, "every", &format!("{path}.every"))? {
                None => 60,
                Some(n) if n >= 5.0 => n.round() as u64,
                Some(n) => return Err(format!("{path}.every: at least 5 seconds, got {n}")),
            };
            let show = number_field(tile, "show", &format!("{path}.show"))?
                .map(|n| n.round().max(1.0) as usize);
            let waiting = match string_field(tile, "waiting_when", &format!("{path}.waiting_when"))?
                .as_deref()
            {
                None | Some("never") => false,
                Some("output") => true,
                Some(other) => {
                    return Err(format!(
                        "{path}.waiting_when: must be \"output\" or \"never\", got `{other}`"
                    ));
                }
            };
            let mut actions = Vec::new();
            if let Some(list_table) = table_field(tile, "actions", &format!("{path}.actions"))? {
                for (a, value) in list(&list_table) {
                    let path = format!("{path}.actions[{a}]");
                    let Value::Table(action) = value else {
                        return Err(format!(
                            "{path}: expected a table, got {}",
                            value.type_name()
                        ));
                    };
                    actions.push(self.item_action(&action, &path)?);
                }
            }
            return Ok(DashTile::Command {
                title: title.unwrap_or_else(|| command.clone()),
                command,
                every,
                show,
                actions,
                waiting,
                argv,
            });
        }
        if let Some(list_table) = table_field(tile, "buttons", &format!("{path}.buttons"))? {
            let mut buttons = Vec::new();
            for (b, value) in list(&list_table) {
                let path = format!("{path}.buttons[{b}]");
                let Value::Table(button) = value else {
                    return Err(format!(
                        "{path}: expected a table, got {}",
                        value.type_name()
                    ));
                };
                let label = string_field(&button, "label", &format!("{path}.label"))?
                    .ok_or_else(|| format!("{path}.label: missing"))?;
                let confirm =
                    bool_field(&button, "confirm", &format!("{path}.confirm"))?.unwrap_or(false);
                let press = match string_field(&button, "run", &format!("{path}.run"))? {
                    Some(command) => {
                        let overlay = match string_field(&button, "show", &format!("{path}.show"))?
                            .as_deref()
                        {
                            None | Some("toast") => false,
                            Some("overlay") => true,
                            Some(other) => {
                                return Err(format!(
                                    "{path}.show: must be \"toast\" or \"overlay\", got `{other}`"
                                ));
                            }
                        };
                        Press::Run { command, overlay }
                    }
                    None => {
                        let has_action = !matches!(
                            button
                                .get::<Value>("action")
                                .map_err(|err| format!("{path}.action: {err}"))?,
                            Value::Nil
                        );
                        if !has_action {
                            return Err(format!("{path}: needs `run` (a command) or `action`"));
                        }
                        match self.action(&button, &format!("{path}.action"))? {
                            Some(action) => Press::Action(action),
                            None => return Err(format!("{path}.action: \"none\" is not a button")),
                        }
                    }
                };
                buttons.push(DashButton {
                    label,
                    press,
                    confirm,
                });
            }
            return Ok(DashTile::Buttons {
                title: title.unwrap_or_default(),
                buttons,
            });
        }
        Err(format!("{path}: needs `panes`, `command`, or `buttons`"))
    }

    fn item_action(&mut self, table: &Table, path: &str) -> Result<ItemAction, String> {
        let key = string_field(table, "key", &format!("{path}.key"))?
            .ok_or_else(|| format!("{path}.key: missing"))?;
        let key = match key.as_str() {
            "enter" => ItemKey::Enter,
            k if k.chars().count() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => {
                ItemKey::Char(k.chars().next().unwrap_or(' ').to_ascii_lowercase())
            }
            other => {
                return Err(format!(
                    "{path}.key: \"enter\" or one letter or digit, got `{other}`"
                ));
            }
        };
        let label = string_field(table, "label", &format!("{path}.label"))?
            .ok_or_else(|| format!("{path}.label: missing"))?;
        let confirm = bool_field(table, "confirm", &format!("{path}.confirm"))?.unwrap_or(false);
        let does = if let Some(url) = string_field(table, "open", &format!("{path}.open"))? {
            ItemDo::Open(url)
        } else {
            let run: Value = table
                .get("run")
                .map_err(|err| format!("{path}.run: {err}"))?;
            if !matches!(run, Value::Nil) {
                let overlay = match string_field(table, "show", &format!("{path}.show"))?.as_deref()
                {
                    None | Some("toast") => false,
                    Some("overlay") => true,
                    Some(other) => {
                        return Err(format!(
                            "{path}.show: must be \"toast\" or \"overlay\", got `{other}`"
                        ));
                    }
                };
                ItemDo::Run {
                    steps: run_steps(&run, &format!("{path}.run"))?,
                    overlay,
                }
            } else if !matches!(
                table
                    .get::<Value>("action")
                    .map_err(|err| format!("{path}.action: {err}"))?,
                Value::Nil
            ) {
                match self.action(table, &format!("{path}.action"))? {
                    Some(action) => ItemDo::Action(action),
                    None => return Err(format!("{path}.action: \"none\" does nothing")),
                }
            } else {
                return Err(format!("{path}: needs `open`, `run`, or `action`"));
            }
        };
        Ok(ItemAction {
            key,
            label,
            does,
            confirm,
        })
    }

    /// The action of a copy mode key: a name, `"none"`, a jump table, or a Lua function.
    fn copy_binding(&mut self, table: &Table, path: &str) -> Result<Option<CopyBinding>, String> {
        let value: Value = table
            .get("action")
            .map_err(|err| format!("{path}: {err}"))?;
        match value {
            Value::Nil => Err(format!("{path}: missing")),
            Value::String(ref name) if name.to_string_lossy() == "none" => Ok(None),
            Value::Function(function) => {
                self.functions.push(function);
                Ok(Some(CopyBinding::Lua(self.functions.len() - 1)))
            }
            other => copy_command(&other)
                .map(|c| Some(CopyBinding::Command(c)))
                .map_err(|err| format!("{path}: {err}")),
        }
    }

    fn action(&mut self, table: &Table, path: &str) -> Result<Option<Action>, String> {
        let value: Value = table
            .get("action")
            .map_err(|err| format!("{path}: {err}"))?;
        match value {
            Value::Nil => Err(format!("{path}: missing")),
            Value::String(name) => {
                let name = name.to_string_lossy();
                if name == "none" {
                    return Ok(None);
                }
                BuiltinAction::from_name(&name)
                    .map(|a| Some(Action::Builtin(a)))
                    .ok_or_else(|| format!("{path}: unknown action `{name}`"))
            }
            Value::Table(spawn) => {
                let profile = string_field(&spawn, "spawn", &format!("{path}.spawn"))?;
                let split = string_field(&spawn, "split", &format!("{path}.split"))?;
                let place =
                    spawn_place(split.as_deref()).map_err(|err| format!("{path}.split: {err}"))?;
                Ok(Some(Action::Spawn { profile, place }))
            }
            Value::Function(function) => {
                self.functions.push(function);
                Ok(Some(Action::Lua(self.functions.len() - 1)))
            }
            other => Err(format!(
                "{path}: expected an action name, a table, or a function, got {}",
                other.type_name()
            )),
        }
    }
}

fn ai(table: &Table) -> Result<AiConfig, String> {
    let mut ai = AiConfig::default();
    if let Some(system) = string_field(table, "system", "ai.system")? {
        ai.system = system;
    }
    ai.command_model =
        string_field(table, "command_model", "ai.command_model")?.filter(|m| !m.trim().is_empty());
    if let Some(n) = number_field(table, "max_tokens", "ai.max_tokens")? {
        if !(1.0..=200_000.0).contains(&n) {
            return Err(format!(
                "ai.max_tokens: must be between 1 and 200000, got {n}"
            ));
        }
        ai.max_tokens = n as u32;
    }
    if let Some(on) = bool_field(table, "api_access", "ai.api_access")? {
        ai.api_access = on;
    }
    if let Some(providers) = table_field(table, "providers", "ai.providers")? {
        for pair in providers.pairs::<String, Table>() {
            let (name, p) = pair.map_err(|err| format!("ai.providers: {err}"))?;
            let path = format!("ai.providers.{name}");
            let mut provider = ai
                .providers
                .iter()
                .find(|x| x.name == name)
                .cloned()
                .unwrap_or(AiProvider {
                    name: name.clone(),
                    kind: AiKind::OpenAi,
                    url: String::new(),
                    model: String::new(),
                    key_env: None,
                    needs_key: true,
                });
            if let Some(kind) = string_field(&p, "kind", &format!("{path}.kind"))? {
                provider.kind = match kind.as_str() {
                    "anthropic" => AiKind::Anthropic,
                    "openai" => AiKind::OpenAi,
                    other => {
                        return Err(format!(
                            "{path}.kind: must be \"anthropic\" or \"openai\", got `{other}`"
                        ));
                    }
                };
            }
            if let Some(url) = string_field(&p, "url", &format!("{path}.url"))? {
                provider.url = url;
            }
            if let Some(model) = string_field(&p, "model", &format!("{path}.model"))? {
                provider.model = model;
            }
            match p
                .get::<Value>("key")
                .map_err(|err| format!("{path}.key: {err}"))?
            {
                Value::Nil => {}
                Value::Boolean(on) => provider.needs_key = on,
                Value::String(env) => {
                    provider.key_env = Some(env.to_string_lossy());
                    provider.needs_key = true;
                }
                other => {
                    return Err(format!(
                        "{path}.key: false (no key) or the name of an env var, got {}",
                        other.type_name()
                    ));
                }
            }
            if provider.url.is_empty() {
                return Err(format!("{path}: give a `url` (and a `model`)"));
            }
            match ai.providers.iter_mut().find(|x| x.name == name) {
                Some(old) => *old = provider,
                None => ai.providers.push(provider),
            }
        }
    }
    if let Some(name) = string_field(table, "provider", "ai.provider")? {
        if !ai.providers.iter().any(|p| p.name == name) {
            let names: Vec<&str> = ai.providers.iter().map(|p| p.name.as_str()).collect();
            return Err(format!(
                "ai.provider: no provider `{name}` (there are: {})",
                names.join(", ")
            ));
        }
        ai.provider = name;
    }
    Ok(ai)
}

fn history(table: &Table) -> Result<HistoryConfig, String> {
    let mut h = HistoryConfig::default();
    let flag = |key: &str| -> Result<Option<bool>, String> {
        match table
            .get::<Value>(key)
            .map_err(|err| format!("history.{key}: {err}"))?
        {
            Value::Nil => Ok(None),
            Value::Boolean(on) => Ok(Some(on)),
            other => Err(format!(
                "history.{key}: expected true or false, got {}",
                other.type_name()
            )),
        }
    };
    if let Some(on) = flag("enabled")? {
        h.enabled = on;
    }
    if let Some(on) = flag("ignore_space")? {
        h.ignore_space = on;
    }
    if let Some(on) = flag("hints")? {
        h.hints = on;
    }
    for (key, field) in [("commands", &mut h.commands), ("dirs", &mut h.dirs)] {
        if let Some(count) = number_field(table, key, &format!("history.{key}"))? {
            if !(1.0..=1_000_000.0).contains(&count) {
                return Err(format!(
                    "history.{key}: must be between 1 and 1000000, got {count}"
                ));
            }
            *field = count as usize;
        }
    }
    Ok(h)
}

fn panels(table: &Table) -> Result<PanelsConfig, String> {
    let mut p = PanelsConfig::default();
    if let Some(dock) = string_field(table, "dock", "panels.dock")? {
        p.dock = match dock.as_str() {
            "left" => DockPlace::Left,
            "right" => DockPlace::Right,
            "bottom" => DockPlace::Bottom,
            other => {
                return Err(format!(
                    "panels.dock: must be \"left\", \"right\", or \"bottom\", got `{other}`"
                ));
            }
        };
    }
    if let Some(size) = number_field(table, "size", "panels.size")? {
        if !(0.1..=0.9).contains(&size) {
            return Err(format!(
                "panels.size: must be between 0.1 and 0.9, got {size}"
            ));
        }
        p.size = size as f32;
    }
    if let Some(open) = table_field(table, "open", "panels.open")? {
        for (i, value) in list(&open) {
            let name = match &value {
                Value::String(name) => name.to_string_lossy(),
                other => {
                    return Err(format!(
                        "panels.open[{i}]: expected a panel name, got {}",
                        other.type_name()
                    ));
                }
            };
            if !PANEL_NAMES.contains(&name.as_str()) {
                return Err(format!(
                    "panels.open[{i}]: no panel `{name}` (there are: {})",
                    PANEL_NAMES.join(", ")
                ));
            }
            // The dock shows one panel at a time: the first one is open.
            p.open.get_or_insert(name);
        }
    }
    Ok(p)
}

fn notifications(table: &Table) -> Result<NotificationConfig, String> {
    let mut n = NotificationConfig::default();
    match table
        .get::<Value>("toasts")
        .map_err(|err| format!("notifications.toasts: {err}"))?
    {
        Value::Nil | Value::Boolean(true) => {}
        Value::Boolean(false) => n.toasts = None,
        Value::String(place) => {
            n.toasts = Some(match place.to_string_lossy().as_str() {
                "bottom_right" => ToastPosition::BottomRight,
                "top_right" => ToastPosition::TopRight,
                "bottom_left" => ToastPosition::BottomLeft,
                "top_left" => ToastPosition::TopLeft,
                "bottom" => ToastPosition::Bottom,
                other => {
                    return Err(format!(
                        "notifications.toasts: must be bottom_right, top_right, bottom_left, top_left, bottom, or false, got `{other}`"
                    ));
                }
            });
        }
        other => {
            return Err(format!(
                "notifications.toasts: expected a place or false, got {}",
                other.type_name()
            ));
        }
    }
    if let Some(count) = number_field(table, "max_visible", "notifications.max_visible")? {
        n.max_visible = count.max(0.0) as usize;
    }
    let os_mode = |text: &str| match text {
        "always" => Ok(OsNotify::Always),
        "when_unfocused" => Ok(OsNotify::WhenUnfocused),
        "never" => Ok(OsNotify::Never),
        other => Err(format!(
            "notifications.os: must be true, false, \"always\", \"when_unfocused\", or a table, got `{other}`"
        )),
    };
    match table
        .get::<Value>("os")
        .map_err(|err| format!("notifications.os: {err}"))?
    {
        Value::Nil | Value::Boolean(false) => {}
        Value::Boolean(true) => n.os = OsNotify::Always,
        Value::String(text) => n.os = os_mode(&text.to_string_lossy())?,
        Value::Table(os) => {
            n.os = match string_field(&os, "when", "notifications.os.when")? {
                Some(text) => os_mode(&text)?,
                None => OsNotify::Always,
            };
            if let Some(levels) = table_field(&os, "levels", "notifications.os.levels")? {
                for (i, value) in list(&levels) {
                    match value {
                        Value::String(level) => n.os_levels.push(level.to_string_lossy()),
                        other => {
                            return Err(format!(
                                "notifications.os.levels[{i}]: expected a string, got {}",
                                other.type_name()
                            ));
                        }
                    }
                }
            }
        }
        other => {
            return Err(format!(
                "notifications.os: expected true, false, a string, or a table, got {}",
                other.type_name()
            ));
        }
    }
    if let Some(seconds) = number_field(table, "long_command", "notifications.long_command")? {
        n.long_command = seconds.max(0.0);
    }
    if let Some(bell) = string_field(table, "bell", "notifications.bell")? {
        n.bell = match bell.as_str() {
            "notify" => true,
            "ignore" => false,
            other => {
                return Err(format!(
                    "notifications.bell: must be \"notify\" or \"ignore\", got `{other}`"
                ));
            }
        };
    }
    if let Some(flash) = bool_field(table, "flash", "notifications.flash")? {
        n.flash = flash;
    }
    Ok(n)
}

fn remote(table: &Table, path: &str) -> Result<Remote, String> {
    let name = string_field(table, "name", &format!("{path}.name"))?
        .ok_or_else(|| format!("{path}.name: missing"))?;
    let host = string_field(table, "host", &format!("{path}.host"))?;
    let command = strings(table, "command", &format!("{path}.command"))?;
    if host.is_none() && command.is_empty() {
        return Err(format!(
            "{path}: give `host` (for ssh) or `command` (a command that runs `ftermctl mux proxy`)"
        ));
    }
    Ok(Remote {
        name,
        host,
        ssh_args: strings(table, "ssh_args", &format!("{path}.ssh_args"))?,
        ftermctl: string_field(table, "ftermctl", &format!("{path}.ftermctl"))?,
        command,
    })
}

/// A list of strings (`{ "a", "b" }`); missing = empty.
fn strings(table: &Table, key: &str, path: &str) -> Result<Vec<String>, String> {
    let Some(items) = table_field(table, key, path)? else {
        return Ok(Vec::new());
    };
    list(&items)
        .into_iter()
        .map(|(i, value)| match value {
            Value::String(text) => Ok(text.to_string_lossy()),
            Value::Integer(n) => Ok(n.to_string()),
            other => Err(format!(
                "{path}[{i}]: expected a string, got {}",
                other.type_name()
            )),
        })
        .collect()
}

fn profile(table: &Table, path: &str) -> Result<Profile, String> {
    let name = string_field(table, "name", &format!("{path}.name"))?
        .ok_or_else(|| format!("{path}.name: missing"))?;
    let wsl = string_field(table, "wsl", &format!("{path}.wsl"))?;
    let command = string_field(table, "command", &format!("{path}.command"))?;
    let (command, mut args) = match (command, &wsl) {
        (Some(command), _) => (command, Vec::new()),
        // `wsl = "Ubuntu"` is enough: fterm knows how to start it.
        (None, Some(distro)) => {
            let p = Profile::wsl(&name, distro);
            (p.command, p.args)
        }
        (None, None) => return Err(format!("{path}.command: missing")),
    };
    if table_field(table, "args", &format!("{path}.args"))?.is_some() {
        args.clear();
    }
    if let Some(list_table) = table_field(table, "args", &format!("{path}.args"))? {
        for (i, value) in list(&list_table) {
            match value {
                Value::String(text) => args.push(text.to_string_lossy()),
                Value::Integer(n) => args.push(n.to_string()),
                Value::Number(n) => args.push(n.to_string()),
                other => {
                    return Err(format!(
                        "{path}.args[{i}]: expected a string, got {}",
                        other.type_name()
                    ));
                }
            }
        }
    }
    let cwd = string_field(table, "cwd", &format!("{path}.cwd"))?
        .map(|dir| expand_home(&dir, home_dir().as_deref()));
    let tab_color = match string_field(table, "tab_color", &format!("{path}.tab_color"))? {
        Some(text) => {
            crate::colors::tab_color(&text).map_err(|err| format!("{path}.tab_color: {err}"))?
        }
        None => None,
    };
    let mut env = Vec::new();
    if let Some(env_table) = table_field(table, "env", &format!("{path}.env"))? {
        for pair in env_table.pairs::<String, Value>() {
            let (key, value) = pair.map_err(|err| format!("{path}.env: {err}"))?;
            let value = match value {
                Value::String(text) => text.to_string_lossy(),
                Value::Integer(n) => n.to_string(),
                Value::Number(n) => n.to_string(),
                Value::Boolean(b) => b.to_string(),
                other => {
                    return Err(format!(
                        "{path}.env.{key}: expected a string, got {}",
                        other.type_name()
                    ));
                }
            };
            env.push((key, value));
        }
        env.sort();
    }
    Ok(Profile {
        name,
        command,
        args,
        cwd,
        env,
        wsl,
        tab_color,
        harmonize: bool_field(table, "harmonize", &format!("{path}.harmonize"))?.unwrap_or(true),
        palette_changes: bool_field(table, "palette_changes", &format!("{path}.palette_changes"))?,
        mux: bool_field(table, "mux", &format!("{path}.mux"))?,
        remote: string_field(table, "remote", &format!("{path}.remote"))?,
        overlay: bool_field(table, "overlay", &format!("{path}.overlay"))?.unwrap_or(false),
        overlay_size: number_field(table, "overlay_size", &format!("{path}.overlay_size"))?
            .map(percent),
        workspace: string_field(table, "workspace", &format!("{path}.workspace"))?
            .map(|w| w.trim().to_owned())
            .filter(|w| !w.is_empty()),
    })
}

/// A size in percent for an overlay or the scratch: 20..100.
fn percent(size: f64) -> u8 {
    size.round().clamp(20.0, 100.0) as u8
}

/// The items of a Lua list, with their 1-based index.
fn list(table: &Table) -> Vec<(usize, Value)> {
    table
        .clone()
        .sequence_values::<Value>()
        .filter_map(Result::ok)
        .enumerate()
        .map(|(i, v)| (i + 1, v))
        .collect()
}

fn table_field(table: &Table, key: &str, path: &str) -> Result<Option<Table>, String> {
    match table
        .get::<Value>(key)
        .map_err(|err| format!("{path}: {err}"))?
    {
        Value::Nil => Ok(None),
        Value::Table(t) => Ok(Some(t)),
        other => Err(format!(
            "{path}: expected a table, got {}",
            other.type_name()
        )),
    }
}

fn number_field(table: &Table, key: &str, path: &str) -> Result<Option<f64>, String> {
    match table
        .get::<Value>(key)
        .map_err(|err| format!("{path}: {err}"))?
    {
        Value::Nil => Ok(None),
        Value::Integer(n) => Ok(Some(n as f64)),
        Value::Number(n) => Ok(Some(n)),
        other => Err(format!(
            "{path}: expected a number, got {}",
            other.type_name()
        )),
    }
}

fn bool_field(table: &Table, key: &str, path: &str) -> Result<Option<bool>, String> {
    match table
        .get::<Value>(key)
        .map_err(|err| format!("{path}: {err}"))?
    {
        Value::Nil => Ok(None),
        Value::Boolean(b) => Ok(Some(b)),
        other => Err(format!(
            "{path}: expected true or false, got {}",
            other.type_name()
        )),
    }
}

fn string_field(table: &Table, key: &str, path: &str) -> Result<Option<String>, String> {
    match table
        .get::<Value>(key)
        .map_err(|err| format!("{path}: {err}"))?
    {
        Value::Nil => Ok(None),
        Value::String(text) => Ok(Some(text.to_string_lossy())),
        other => Err(format!(
            "{path}: expected a string, got {}",
            other.type_name()
        )),
    }
}

pub fn load_file(path: &Path) -> Result<LoadedConfig, String> {
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    load_str(&source, &path.display().to_string())
}

/// `--config PATH` (or `--config=PATH`) on the command line of fterm.
pub fn config_arg(args: &[String]) -> Result<Option<PathBuf>, String> {
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let path = if word == "--config" {
            words.next().map(String::as_str)
        } else if let Some(path) = word.strip_prefix("--config=") {
            Some(path)
        } else {
            continue;
        };
        return match path.filter(|p| !p.is_empty()) {
            Some(path) => Ok(Some(PathBuf::from(path))),
            None => Err("--config needs the path of a config file".to_owned()),
        };
    }
    Ok(None)
}

/// Where the config is: `--config`, then `FTERM_CONFIG`, then `fterm.lua` next to fterm.exe (a portable
/// fterm: the program and its config in one folder), then the config of the user.
pub fn choose_config_path(
    arg: Option<&Path>,
    env: Option<&Path>,
    exe_dir: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
    user: PathBuf,
) -> PathBuf {
    if let Some(path) = arg.or(env) {
        return path.to_path_buf();
    }
    match exe_dir.map(|dir| dir.join("fterm.lua")) {
        Some(next_to_exe) if exists(&next_to_exe) => next_to_exe,
        _ => user,
    }
}

/// Where the config file is: `--config` (`arg`), `FTERM_CONFIG`, `fterm.lua` next to fterm.exe, or
/// the config of the user (see `user_config_path`).
pub fn config_path(arg: Option<&Path>) -> PathBuf {
    let env = std::env::var_os("FTERM_CONFIG").map(PathBuf::from);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    choose_config_path(
        arg,
        env.as_deref(),
        exe_dir.as_deref(),
        Path::is_file,
        user_config_path(),
    )
}

/// `%APPDATA%\fterm\fterm.lua`, or `~/.config/fterm/fterm.lua`.
pub fn user_config_path() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| crate::profiles::home_dir().map(|home| home.join(".config")))
    };
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("fterm")
        .join("fterm.lua")
}

/// The file that "Open config" makes when there is no config yet.
pub const SAMPLE_CONFIG: &str = include_str!("sample.lua");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colors::Rgb;
    use crate::keys::KeyChord;

    fn load(source: &str) -> LoadedConfig {
        load_str(source, "test.lua").unwrap_or_else(|err| panic!("{err}"))
    }

    fn error(source: &str) -> String {
        match load_str(source, "test.lua") {
            Ok(_) => panic!("no error"),
            Err(err) => err,
        }
    }

    #[test]
    fn copy_mode_keys_from_the_config() {
        use crate::copy_keys::{CopyBinding, CopyCommand, CopyKey, JumpKind, Lookup, parse_keys};
        let plain = load("return {}");
        let cm = &plain.config.copy_mode;
        assert!(cm.exit_after_copy && cm.status_bar);
        assert_eq!(
            cm.keys.lookup(&[CopyKey::char('G')]),
            Lookup::Found(CopyBinding::Command(CopyCommand::Bottom))
        );
        let loaded = load(
            r#"return { copy_mode = { exit_after_copy = false, status_bar = false, keys = {
                { key = "x", action = "copy_line" },
                { key = "q", action = "none" },
                { key = "g g", action = "top" },
                { key = "ctrl+o", action = function(fterm, cm) fterm.copy(cm.selection) end },
                { key = "z", action = { jump = "f", char = "(" } },
            } } }"#,
        );
        let cm = &loaded.config.copy_mode;
        assert!(!cm.exit_after_copy && !cm.status_bar);
        let get = |text: &str| cm.keys.lookup(&parse_keys(text).unwrap());
        assert_eq!(
            get("x"),
            Lookup::Found(CopyBinding::Command(CopyCommand::CopyLine))
        );
        assert_eq!(get("q"), Lookup::None);
        assert_eq!(get("g"), Lookup::Prefix);
        assert!(matches!(get("ctrl+o"), Lookup::Found(CopyBinding::Lua(_))));
        assert_eq!(
            get("z"),
            Lookup::Found(CopyBinding::Command(CopyCommand::JumpTo(
                JumpKind::Forward,
                '('
            )))
        );
    }

    #[test]
    fn copy_mode_errors_name_the_field() {
        assert!(error(r#"return { copy_mode = 5 }"#).contains("copy_mode"));
        let bad_key =
            error(r#"return { copy_mode = { keys = { { key = "shift+v", action = "up" } } } }"#);
        assert!(bad_key.contains("copy_mode.keys[1].key"), "{bad_key}");
        let bad_action =
            error(r#"return { copy_mode = { keys = { { key = "v", action = "fly" } } } }"#);
        assert!(
            bad_action.contains("copy_mode.keys[1].action"),
            "{bad_action}"
        );
        assert!(
            bad_action.contains("select_word"),
            "lists the names: {bad_action}"
        );
        let bad_jump = error(
            r#"return { copy_mode = { keys = { { key = "v", action = { jump = "x", char = "a" } } } } }"#,
        );
        assert!(bad_jump.contains("jump"), "{bad_jump}");
        assert!(
            error(r#"return { copy_mode = { exit_after_copy = "yes" } }"#)
                .contains("copy_mode.exit_after_copy")
        );
    }

    #[test]
    fn a_lua_copy_action_gets_the_text_and_can_drive_copy_mode() {
        use crate::copy_keys::{CopyBinding, CopyCommand, CopyKey, JumpKind, Lookup};
        let loaded = load(
            r#"return { copy_mode = { keys = { { key = "x", action = function(fterm, cm)
                fterm.copy_mode("select_word")
                fterm.copy_mode({ jump = "t", char = ")" })
                fterm.copy(cm.word .. "|" .. cm.line .. "|" .. cm.mode .. "|" .. cm.row .. ":" .. cm.column
                    .. "|" .. tostring(cm.selection) .. "|" .. tostring(cm.search))
                fterm.copy_mode("exit")
            end } } } }"#,
        );
        let Lookup::Found(CopyBinding::Lua(index)) =
            loaded.config.copy_mode.keys.lookup(&[CopyKey::char('x')])
        else {
            panic!("no Lua action");
        };
        let input = CopyModeIn {
            selection: None,
            line: "fn main() {".into(),
            word: Some("main".into()),
            row: 12,
            column: 3,
            mode: "normal".into(),
            search: Some("main".into()),
        };
        assert_eq!(
            loaded.call_copy(index, &input).unwrap(),
            [
                ApiCall::CopyMode(CopyCommand::SelectWord),
                ApiCall::CopyMode(CopyCommand::JumpTo(JumpKind::TillForward, ')')),
                ApiCall::Copy("main|fn main() {|normal|12:3|nil|main".into()),
                ApiCall::CopyMode(CopyCommand::Exit),
            ]
        );
        let bad = load(
            r#"return { copy_mode = { keys = { { key = "x", action = function(fterm, cm)
                fterm.copy_mode("fly")
            end } } } }"#,
        );
        let Lookup::Found(CopyBinding::Lua(index)) =
            bad.config.copy_mode.keys.lookup(&[CopyKey::char('x')])
        else {
            panic!("no Lua action");
        };
        assert!(bad.call_copy(index, &input).unwrap_err().contains("fly"));
    }

    #[test]
    fn an_empty_table_gives_the_defaults() {
        let config = load("return {}").config;
        assert_eq!(config.font_size, 14.0);
        assert_eq!(config.padding, 6.0);
        assert_eq!(config.scrollback, 10_000);
        assert!(config.profiles.is_empty());
        assert!(
            config
                .keys
                .get(&KeyChord::parse("ctrl+shift+t").unwrap())
                .is_some()
        );
    }

    #[test]
    fn shell_integration_can_be_turned_off() {
        assert!(load("return {}").config.shell_integration);
        assert!(
            !load("return { shell_integration = false }")
                .config
                .shell_integration
        );
        assert!(error(r#"return { shell_integration = "yes" }"#).contains("shell_integration"));
    }

    #[test]
    fn the_sample_config_loads() {
        let loaded = load(SAMPLE_CONFIG);
        assert!(loaded.config.font_size > 0.0);
    }

    #[test]
    fn full_config() {
        let config = load(
            r##"
            return {
              font = { size = 18 },
              padding = 10,
              scrollback = 50000,
              braille_style = "pixels",
              colors = {
                background = "#000000",
                cursor = "#fff",
                ansi = { "#010101", "#020202" },
                bright = { [8] = "#080808" },
              },
              default_profile = "Claude",
              profiles = {
                { name = "Claude", command = "claude", args = { "--continue" }, cwd = "C:/code", env = { A = "1" } },
                { name = "Ollama", command = "ollama", args = { "run", "llama3" } },
              },
            }
            "##,
        )
        .config;
        assert_eq!(config.font_size, 18.0);
        assert_eq!(config.padding, 10.0);
        assert_eq!(config.scrollback, 50_000);
        assert_eq!(config.braille_style, BrailleStyle::Pixels);
        assert_eq!(config.colors.background, Some(Rgb { r: 0, g: 0, b: 0 }));
        assert_eq!(
            config.colors.cursor,
            Some(Rgb {
                r: 255,
                g: 255,
                b: 255
            })
        );
        assert_eq!(config.colors.ansi[1], Some(Rgb { r: 2, g: 2, b: 2 }));
        assert_eq!(config.colors.ansi[2], None);
        assert_eq!(config.colors.bright[7], Some(Rgb { r: 8, g: 8, b: 8 }));
        assert_eq!(config.default_profile.as_deref(), Some("Claude"));
        assert_eq!(config.profiles.len(), 2);
        let claude = &config.profiles[0];
        assert_eq!(claude.args, ["--continue"]);
        assert_eq!(claude.cwd.as_deref(), Some(Path::new("C:/code")));
        assert_eq!(claude.env, [("A".to_owned(), "1".to_owned())]);
    }

    #[test]
    fn keys_and_commands() {
        let loaded = load(
            r#"
            return {
              keys = {
                { key = "ctrl+alt+c", action = { spawn = "Claude" } },
                { key = "ctrl+alt+o", action = { spawn = "Ollama", split = "right" } },
                { key = "ctrl+shift+w", action = "none" },
                { key = "ctrl+shift+t", action = "split_down" },
                { key = "ctrl+alt+h", action = function(fterm) fterm.send_text("hi") end },
              },
              commands = {
                { name = "Git status", action = function(fterm) fterm.send_text("git status\r") end },
                { name = "New Claude", action = { spawn = "Claude", split = "down" } },
              },
            }
            "#,
        );
        let keys = &loaded.config.keys;
        let get = |text: &str| keys.get(&KeyChord::parse(text).unwrap()).cloned();
        assert_eq!(
            get("ctrl+alt+c"),
            Some(Action::Spawn {
                profile: Some("Claude".into()),
                place: SpawnWhere::Tab
            })
        );
        assert_eq!(
            get("ctrl+alt+o"),
            Some(Action::Spawn {
                profile: Some("Ollama".into()),
                place: SpawnWhere::SplitRight
            })
        );
        assert_eq!(get("ctrl+shift+w"), None);
        assert_eq!(
            get("ctrl+shift+t"),
            Some(Action::Builtin(BuiltinAction::SplitDown))
        );
        let Some(Action::Lua(index)) = get("ctrl+alt+h") else {
            panic!("not a Lua action");
        };
        assert_eq!(
            loaded.call(index).unwrap(),
            [ApiCall::SendText("hi".into())]
        );

        assert_eq!(loaded.config.commands.len(), 2);
        assert_eq!(loaded.config.commands[0].name, "Git status");
        let Action::Lua(index) = loaded.config.commands[0].action else {
            panic!("not a Lua action");
        };
        assert_eq!(
            loaded.call(index).unwrap(),
            [ApiCall::SendText("git status\r".into())]
        );
    }

    #[test]
    fn lua_functions_can_color_a_tab() {
        let loaded = load(
            r##"
            return { keys = { { key = "f6", action = function(fterm)
              fterm.set_tab_color("#f38ba8")
              fterm.set_tab_color("none", 4)
            end }, { key = "f7", action = function(fterm)
              fterm.set_tab_color("red")
            end } } }
            "##,
        );
        let get = |key: &str| {
            let Some(Action::Lua(index)) = loaded
                .config
                .keys
                .get(&KeyChord::parse(key).unwrap())
                .cloned()
            else {
                panic!("no {key}");
            };
            loaded.call(index)
        };
        assert_eq!(
            get("f6").unwrap(),
            [
                ApiCall::SetTabColor {
                    pane: None,
                    color: Some([0xf3, 0x8b, 0xa8])
                },
                ApiCall::SetTabColor {
                    pane: Some(4),
                    color: None
                },
            ]
        );
        let err = get("f7").unwrap_err();
        assert!(err.contains("#rrggbb"), "{err}");
    }

    #[test]
    fn lua_functions_can_use_the_whole_api() {
        let loaded = load(
            r#"
            return { keys = { { key = "f5", action = function(fterm)
              fterm.spawn("Claude", { split = "right" })
              fterm.spawn()
              fterm.notify("hello")
              fterm.copy("text")
              fterm.action("zoom")
            end } } }
            "#,
        );
        let Some(Action::Lua(index)) = loaded
            .config
            .keys
            .get(&KeyChord::parse("f5").unwrap())
            .cloned()
        else {
            panic!("no f5");
        };
        assert_eq!(
            loaded.call(index).unwrap(),
            [
                ApiCall::Spawn {
                    profile: Some("Claude".into()),
                    place: SpawnWhere::SplitRight
                },
                ApiCall::Spawn {
                    profile: None,
                    place: SpawnWhere::Tab
                },
                ApiCall::Notify {
                    title: String::new(),
                    body: "hello".into(),
                    level: "info".into()
                },
                ApiCall::Copy("text".into()),
                ApiCall::Action(BuiltinAction::Zoom),
            ]
        );
        // Each call starts with an empty queue.
        assert_eq!(loaded.call(index).unwrap().len(), 5);
    }

    #[test]
    fn errors_in_lua_functions_are_reported() {
        let loaded =
            load(r#"return { keys = { { key = "f6", action = function() error("boom") end } } }"#);
        let Some(Action::Lua(index)) = loaded
            .config
            .keys
            .get(&KeyChord::parse("f6").unwrap())
            .cloned()
        else {
            panic!("no f6");
        };
        assert!(loaded.call(index).unwrap_err().contains("boom"));
        assert!(loaded.call(99).is_err());
    }

    #[test]
    fn helpers_and_loops_work() {
        // The point of a script: functions, tables, and loops.
        let config = load(
            r#"
            local function tool(name, cmd) return { name = name, command = cmd } end
            local profiles = {}
            for _, t in ipairs({ {"A", "a"}, {"B", "b"} }) do
              table.insert(profiles, tool(t[1], t[2]))
            end
            local big = true
            return { font = { size = if big then 20 else 12 }, profiles = profiles }
            "#,
        )
        .config;
        assert_eq!(config.font_size, 20.0);
        assert_eq!(config.profiles.len(), 2);
    }

    #[test]
    fn remote_machines_and_their_connect_commands() {
        use super::Remote;
        let loaded = load(
            r#"return {
              remotes = {
                { name = "prod", host = "me@prod.example.com" },
                { name = "build", host = "build", ssh_args = { "-p", "2222" }, ftermctl = "~/bin/ftermctl" },
                { name = "wsl", command = { "wsl", "-d", "Ubuntu", "--", "ftermctl", "mux", "proxy" } },
              },
              profiles = { { name = "prod htop", remote = "prod", command = "htop" } },
            }"#,
        );
        let r = &loaded.config.remotes;
        assert_eq!(r.len(), 3);
        assert_eq!(
            r[0].connect_command(),
            [
                "ssh",
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "me@prod.example.com",
                "ftermctl",
                "mux",
                "proxy"
            ]
        );
        assert_eq!(
            r[1].connect_command(),
            [
                "ssh",
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-p",
                "2222",
                "build",
                "~/bin/ftermctl",
                "mux",
                "proxy"
            ]
        );
        assert_eq!(r[2].connect_command()[0], "wsl");
        assert_eq!(loaded.config.profiles[0].remote.as_deref(), Some("prod"));
        assert!(error(r#"return { remotes = { { host = "x" } } }"#).contains("remotes[1].name"));
        assert!(error(r#"return { remotes = { { name = "x" } } }"#).contains("host"));
        // An ad-hoc host ("Connect over SSH…") is a remote too.
        assert_eq!(
            Remote::ssh("me@h").connect_command(),
            [
                "ssh",
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "me@h",
                "ftermctl",
                "mux",
                "proxy"
            ]
        );
    }

    #[test]
    fn panes_can_live_in_the_mux_server() {
        assert!(!load("return {}").config.mux, "off by default");
        let loaded = load(
            r#"return { mux = true, profiles = { { name = "a", command = "a", mux = false },
                                                 { name = "b", command = "b" } } }"#,
        );
        assert!(loaded.config.mux);
        assert_eq!(loaded.config.profiles[0].mux, Some(false));
        assert_eq!(loaded.config.profiles[1].mux, None);
        assert!(error("return { mux = 1 }").contains("mux"));
    }

    #[test]
    fn a_profile_can_open_its_tabs_in_a_workspace() {
        let loaded = load(
            r#"return { profiles = { { name = "Claude", command = "claude", workspace = "agents" },
                                     { name = "sh", command = "sh" } } }"#,
        );
        assert_eq!(
            loaded.config.profiles[0].workspace.as_deref(),
            Some("agents")
        );
        assert_eq!(loaded.config.profiles[1].workspace, None);
    }

    #[test]
    fn braille_in_panes_is_round_dots_by_default() {
        assert_eq!(load("return {}").config.braille_style, BrailleStyle::Dots);
        assert_eq!(
            load(r#"return { braille_style = "pixels" }"#)
                .config
                .braille_style,
            BrailleStyle::Pixels
        );
    }

    #[test]
    fn panes_off_the_screen_redraw_once_a_second() {
        assert_eq!(load("return {}").config.background_redraws, 1.0);
        assert_eq!(
            load("return { background_redraws = 4 }")
                .config
                .background_redraws,
            4.0
        );
        // At least one every 10 s, at most 60 a second.
        assert_eq!(
            load("return { background_redraws = 0 }")
                .config
                .background_redraws,
            0.1
        );
        assert_eq!(
            load("return { background_redraws = 500 }")
                .config
                .background_redraws,
            60.0
        );
    }

    #[test]
    fn the_command_of_the_window_overlay() {
        assert_eq!(load("return {}").config.window_overlay, None);
        assert_eq!(
            load(r#"return { window_overlay = "far" }"#)
                .config
                .window_overlay
                .as_deref(),
            Some("far")
        );
        assert_eq!(
            load(r#"return { window_overlay = "  " }"#)
                .config
                .window_overlay,
            None
        );
    }

    #[test]
    fn the_sizes_of_overlays_and_the_scratch() {
        let config = load("return {}").config;
        assert_eq!((config.overlay_size, config.scratch_size), (80, 50));
        let config = load("return { overlay_size = 100, scratch_size = 35 }").config;
        assert_eq!((config.overlay_size, config.scratch_size), (100, 35));
        // Out of 20..100: the nearest end.
        let config = load("return { overlay_size = 5, scratch_size = 300 }").config;
        assert_eq!((config.overlay_size, config.scratch_size), (20, 100));
    }

    #[test]
    fn the_dashboard_has_agents_and_busy_panes_by_default() {
        let dash = load("return {}").config.dashboard;
        assert_eq!(dash.columns, 0);
        assert_eq!(
            dash.tiles,
            [
                DashTile::Panes(PaneFilter::Agents),
                DashTile::Panes(PaneFilter::Busy)
            ]
        );
    }

    #[test]
    fn dashboard_tiles_from_the_config() {
        let loaded = load(
            r#"return { dashboard = { columns = 3, tiles = {
                { panes = "all" },
                { panes = { program = "cargo", workspace = "Work" } },
                { title = "PRs", command = "gh pr list", every = 30 },
                { title = "Log", command = "tail -n 5 x.log" },
                { title = "Publish", buttons = {
                    { label = "Staging", run = "gh workflow run p.yml", confirm = true },
                    { label = "See it", run = "gh run list", show = "overlay" },
                    { label = "Tab", action = "new_tab" },
                } },
            } } }"#,
        );
        let dash = &loaded.config.dashboard;
        assert_eq!(dash.columns, 3);
        assert_eq!(dash.tiles[0], DashTile::Panes(PaneFilter::All));
        assert_eq!(
            dash.tiles[1],
            DashTile::Panes(PaneFilter::Match(PaneMatch {
                program: Some("cargo".into()),
                workspace: Some("work".into()),
                ..Default::default()
            })),
            "matches have no case"
        );
        let DashTile::Command {
            title,
            command,
            every,
            show,
            actions,
            waiting,
            argv,
        } = &dash.tiles[2]
        else {
            panic!("{:?}", dash.tiles[2]);
        };
        assert_eq!(
            (title.as_str(), command.as_str(), *every),
            ("PRs", "gh pr list", 30)
        );
        assert_eq!(
            (*show, actions.len(), *waiting),
            (None, 0, false),
            "a plain list"
        );
        assert_eq!(*argv, None, "a text: the shell runs it");
        assert!(
            matches!(&dash.tiles[3], DashTile::Command { every: 60, .. }),
            "60 s by default"
        );
        let DashTile::Buttons { title, buttons } = &dash.tiles[4] else {
            panic!("{:?}", dash.tiles[4]);
        };
        assert_eq!(title, "Publish");
        assert_eq!(buttons.len(), 3);
        assert_eq!(buttons[0].label, "Staging");
        assert!(buttons[0].confirm);
        assert_eq!(
            buttons[0].press,
            Press::Run {
                command: "gh workflow run p.yml".into(),
                overlay: false
            }
        );
        assert_eq!(
            buttons[1].press,
            Press::Run {
                command: "gh run list".into(),
                overlay: true
            }
        );
        assert_eq!(
            buttons[2].press,
            Press::Action(Action::Builtin(BuiltinAction::NewTab))
        );
        assert!(!buttons[2].confirm);
    }

    #[test]
    fn the_agents_panel_settings() {
        let agents = load("return {}").config.agents;
        assert!(agents.history);
        assert_eq!(agents.show, AgentsConfig::default().show);
        assert!(agents.shows("question") && agents.shows("cost"));
        let agents =
            load(r#"return { agents = { history = false, show = { "question", "todos" } } }"#)
                .config
                .agents;
        assert!(!agents.history);
        assert!(agents.shows("todos") && !agents.shows("cost"));
        assert!(error(r#"return { agents = { show = { "money" } } }"#).contains("agents.show[1]"));
    }

    #[test]
    fn a_list_tile_with_actions_on_its_items() {
        let loaded = load(
            r#"return { dashboard = { tiles = {
                { title = "PRs", command = "gh search prs", show = 2, waiting_when = "output", actions = {
                    { key = "enter", label = "open", open = "{1}" },
                    { key = "a", label = "approve", run = { "gh", "pr", "review", "--approve", "{1}" }, confirm = true },
                    { key = "v", label = "view", show = "overlay",
                      run = { { "gh", "pr", "view", "{1}" }, { "gh", "pr", "diff", "{1}" } } },
                    { key = "c", label = "comment", action = function(fterm, item) fterm.notify(item.line) end },
                } },
            } } }"#,
        );
        let DashTile::Command {
            show,
            actions,
            waiting,
            ..
        } = &loaded.config.dashboard.tiles[0]
        else {
            panic!("{:?}", loaded.config.dashboard.tiles[0]);
        };
        assert_eq!(*show, Some(2));
        assert!(*waiting);
        assert_eq!(actions.len(), 4);
        assert_eq!(actions[0].key, ItemKey::Enter);
        assert_eq!(actions[0].does, ItemDo::Open("{1}".into()));
        assert_eq!(actions[1].key, ItemKey::Char('a'));
        assert!(actions[1].confirm);
        assert_eq!(
            actions[1].does,
            ItemDo::Run {
                steps: vec![vec![
                    "gh".into(),
                    "pr".into(),
                    "review".into(),
                    "--approve".into(),
                    "{1}".into()
                ]],
                overlay: false
            }
        );
        assert_eq!(
            actions[2].does,
            ItemDo::Run {
                steps: vec![
                    vec!["gh".into(), "pr".into(), "view".into(), "{1}".into()],
                    vec!["gh".into(), "pr".into(), "diff".into(), "{1}".into()],
                ],
                overlay: true
            }
        );
        let ItemDo::Action(Action::Lua(index)) = actions[3].does else {
            panic!("{:?}", actions[3].does);
        };
        // The function gets the item: its line and its fields.
        let calls = loaded
            .call_with_item(index, "u\tt", &["u".into(), "t".into()])
            .unwrap();
        assert_eq!(
            calls,
            [ApiCall::Notify {
                title: String::new(),
                body: "u\tt".into(),
                level: "info".into()
            }]
        );
    }

    #[test]
    fn a_command_tile_can_skip_the_shell() {
        let loaded = load(
            r#"return { dashboard = { tiles = {
                { title = "PRs", command = { "gh", "search", "prs", "--template", '{{"\t"}}' } },
            } } }"#,
        );
        let DashTile::Command { command, argv, .. } = &loaded.config.dashboard.tiles[0] else {
            panic!();
        };
        assert_eq!(argv.as_ref().unwrap()[4], "{{\"\t\"}}", "the quotes stay");
        assert!(command.starts_with("gh search prs"));
        assert!(
            error(r#"return { dashboard = { tiles = { { command = { { "a" }, { "b" } } } } } }"#)
                .contains("one program")
        );
    }

    #[test]
    fn bad_item_actions_say_where() {
        for (action, want) in [
            // A command as one string goes through the shell: a field from the output could run as a command.
            (
                r#"{ key = "a", label = "x", run = "gh pr review {1}" }"#,
                "a list of arguments",
            ),
            (r#"{ key = "a", label = "x", run = { } }"#, "empty"),
            (r#"{ key = "ab", label = "x", open = "{1}" }"#, ".key"),
            (r#"{ key = "a", open = "{1}" }"#, ".label"),
            (
                r#"{ key = "a", label = "x" }"#,
                "needs `open`, `run`, or `action`",
            ),
        ] {
            let source = format!(
                r#"return {{ dashboard = {{ tiles = {{ {{ command = "x", actions = {{ {action} }} }} }} }} }}"#
            );
            let err = error(&source);
            assert!(
                err.contains("dashboard.tiles[1].actions[1]") && err.contains(want),
                "{action}: {err}"
            );
        }
        assert!(
            error(
                r#"return { dashboard = { tiles = { { command = "x", waiting_when = "y" } } } }"#
            )
            .contains("waiting_when")
        );
    }

    #[test]
    fn lua_can_run_programs_and_open_links_through_fterm() {
        let loaded = load(
            r#"return { keys = {
                { key = "f6", action = function(fterm)
                    fterm.run({ "gh", "pr", "list" })
                    fterm.run({ { "a" }, { "b", "c" } }, { show = "overlay" })
                    fterm.open("https://example.com")
                end },
            } }"#,
        );
        let Some(Action::Lua(index)) = loaded
            .config
            .keys
            .get(&KeyChord::parse("f6").unwrap())
            .cloned()
        else {
            panic!("no f6");
        };
        let calls = loaded.call(index).unwrap();
        assert_eq!(
            calls,
            [
                ApiCall::Run {
                    steps: vec![vec!["gh".into(), "pr".into(), "list".into()]],
                    overlay: false
                },
                ApiCall::Run {
                    steps: vec![vec!["a".into()], vec!["b".into(), "c".into()]],
                    overlay: true
                },
                ApiCall::Open("https://example.com".into()),
            ]
        );
    }

    #[test]
    fn bad_dashboard_tiles_say_where() {
        for (source, want) in [
            (
                r#"return { dashboard = { tiles = { { panes = "some" } } } }"#,
                "dashboard.tiles[1].panes",
            ),
            (
                r#"return { dashboard = { tiles = { { title = "x" } } } }"#,
                "dashboard.tiles[1]",
            ),
            (
                r#"return { dashboard = { tiles = { { title = "x", command = "a", every = 1 } } } }"#,
                "dashboard.tiles[1].every",
            ),
            (
                r#"return { dashboard = { tiles = { { buttons = { { label = "x" } } } } } }"#,
                "dashboard.tiles[1].buttons[1]",
            ),
            (
                r#"return { dashboard = { tiles = { { buttons = { { run = "x" } } } } } }"#,
                "dashboard.tiles[1].buttons[1].label",
            ),
        ] {
            let err = error(source);
            assert!(err.contains(want), "{source}: {err}");
        }
    }

    #[test]
    fn the_place_of_companions() {
        let config = load("return {}").config;
        assert_eq!(
            (config.companion_side, config.companion_size),
            (CompanionSide::Bottom, 40)
        );
        let config = load(r#"return { companion_side = "right", companion_size = 30 }"#).config;
        assert_eq!(
            (config.companion_side, config.companion_size),
            (CompanionSide::Right, 30)
        );
        // Out of 10..90: the nearest end.
        assert_eq!(
            load("return { companion_size = 99 }").config.companion_size,
            90
        );
        assert_eq!(
            load("return { companion_size = 1 }").config.companion_size,
            10
        );
        assert!(error(r#"return { companion_side = "top" }"#).contains("companion_side"));
    }

    #[test]
    fn a_profile_can_open_as_an_overlay() {
        let loaded = load(
            r#"return { profiles = { { name = "yazi", command = "yazi", overlay = true, overlay_size = 100 },
                                     { name = "shell", command = "pwsh" } } }"#,
        );
        let (yazi, shell) = (&loaded.config.profiles[0], &loaded.config.profiles[1]);
        assert!(yazi.overlay && !shell.overlay);
        assert_eq!((yazi.overlay_size, shell.overlay_size), (Some(100), None));
    }

    #[test]
    fn pictures_are_pixels_by_default() {
        assert_eq!(load("return {}").config.pictures, Pictures::Pixels);
        for (text, want) in [
            ("braille", Pictures::Braille),
            ("off", Pictures::Off),
            ("pixels", Pictures::Pixels),
        ] {
            let loaded = load(&format!("return {{ pictures = \"{text}\" }}"));
            assert_eq!(loaded.config.pictures, want, "{text}");
        }
        assert!(error(r#"return { pictures = "sixel" }"#).contains("pictures"));
    }

    #[test]
    fn the_language_of_the_ui() {
        assert_eq!(load("return {}").config.language, None, "English");
        assert_eq!(
            load(r#"return { language = "ru" }"#)
                .config
                .language
                .as_deref(),
            Some("ru")
        );
        assert!(error("return { language = 1 }").contains("language"));
    }

    #[test]
    fn when_a_plan_of_plan_mode_opens_a_review() {
        assert_eq!(load("return {}").config.plan_review, PlanReview::Ask);
        assert_eq!(
            load(r#"return { plan_review = "always" }"#)
                .config
                .plan_review,
            PlanReview::Always
        );
        assert_eq!(
            load(r#"return { plan_review = "never" }"#)
                .config
                .plan_review,
            PlanReview::Never
        );
        assert!(error(r#"return { plan_review = "yes" }"#).contains("plan_review"));
    }

    #[test]
    fn the_plan_review_modes_go_round() {
        assert_eq!(PlanReview::Ask.next(), PlanReview::Always);
        assert_eq!(PlanReview::Always.next(), PlanReview::Never);
        assert_eq!(PlanReview::Never.next(), PlanReview::Ask);
        assert_eq!(PlanReview::Never.name(), "never");
    }

    #[test]
    fn programs_may_or_may_not_change_the_palette() {
        let loaded = load(
            r#"return {
              palette_changes = false,
              profiles = { { name = "far", command = "far", palette_changes = true },
                           { name = "sh", command = "sh" } },
            }"#,
        );
        assert!(!loaded.config.palette_changes);
        assert_eq!(loaded.config.profiles[0].palette_changes, Some(true));
        assert_eq!(
            loaded.config.profiles[1].palette_changes, None,
            "the global value"
        );
        assert!(
            load("return {}").config.palette_changes,
            "allowed by default"
        );
        assert!(error(r#"return { palette_changes = "no" }"#).contains("palette_changes"));
    }

    #[test]
    fn the_config_and_profiles_can_change_the_harmonize() {
        let loaded = load(
            r#"return {
              harmonize = { strength = 0.7 },
              profiles = { { name = "btop", command = "btop", harmonize = false },
                           { name = "sh", command = "sh" } },
            }"#,
        );
        let h = loaded.config.harmonize;
        assert_eq!((h.strength, h.min_contrast), (Some(0.7), None));
        assert!(
            !loaded.config.profiles[0].harmonize,
            "a program that needs its exact colors"
        );
        assert!(loaded.config.profiles[1].harmonize);
        assert_eq!(load("return {}").config.harmonize.strength, None);
        assert!(error(r#"return { harmonize = { strength = 3 } }"#).contains("harmonize.strength"));
    }

    #[test]
    fn the_theme_by_name_by_system_or_inline() {
        use crate::theme::ThemeChoice;
        assert_eq!(load("return {}").config.theme, ThemeChoice::Default);
        assert_eq!(
            load(r#"return { theme = "Nord" }"#).config.theme,
            ThemeChoice::Named("Nord".into())
        );
        assert_eq!(
            load(r#"return { theme = { light = "Catppuccin Latte", dark = "Nord" } }"#)
                .config
                .theme,
            ThemeChoice::System {
                light: "Catppuccin Latte".into(),
                dark: "Nord".into()
            }
        );
        // Only one of them: the other is the built-in one.
        assert_eq!(
            load(r#"return { theme = { dark = "Nord" } }"#).config.theme,
            ThemeChoice::System {
                light: "Catppuccin Latte".into(),
                dark: "Nord".into()
            }
        );
        // A theme in Lua: the same keys as the JSON.
        let lua = load(
            r##"return { theme = {
              name = "Mine",
              terminal = { background = "#101010", ansi = { "#000000", "#cc0000", "#00cc00", "#cccc00",
                                                            "#0000cc", "#cc00cc", "#00cccc", "#cccccc" } },
              ui = { accent = "#123456" },
            } }"##,
        );
        let ThemeChoice::Inline(theme) = lua.config.theme else {
            panic!("not inline: {:?}", lua.config.theme);
        };
        assert_eq!(theme.name, "Mine");
        assert_eq!(theme.ui.accent.r, 0x12);
        assert_eq!(theme.terminal.ansi[1].r, 0xcc);
        assert_eq!(theme.ui.surface_active.r, 0x10, "made from the background");
        // A theme as JSON text.
        let json =
            load(r##"return { theme = [[ { "name": "Json", "ui": { "accent": "#654321" } } ]] }"##);
        let ThemeChoice::Inline(theme) = json.config.theme else {
            panic!("not inline");
        };
        assert_eq!((theme.name.as_str(), theme.ui.accent.r), ("Json", 0x65));
    }

    #[test]
    fn a_bad_theme_says_where() {
        assert!(error(r#"return { theme = 5 }"#).contains("theme"));
        let bad = error(r#"return { theme = { ui = { accent = "blue" } } }"#);
        assert!(bad.contains("theme") && bad.contains("ui.accent"), "{bad}");
        let bad = error(r#"return { theme = "{ nope" }"#);
        assert!(bad.contains("theme") && bad.contains("JSON"), "{bad}");
        let bad = error(r#"return { theme = { light = 5 } }"#);
        assert!(bad.contains("theme.light"), "{bad}");
    }

    #[test]
    fn a_profile_can_color_its_tabs() {
        let loaded = load(
            r##"return { profiles = { { name = "Prod", command = "ssh", tab_color = "#f38ba8" } } }"##,
        );
        assert_eq!(
            loaded.config.profiles[0].tab_color,
            Some([0xf3, 0x8b, 0xa8])
        );
        let plain = load(r#"return { profiles = { { name = "x", command = "cmd.exe" } } }"#);
        assert_eq!(plain.config.profiles[0].tab_color, None);
        let bad = error(
            r#"return { profiles = { { name = "x", command = "cmd.exe", tab_color = "red" } } }"#,
        );
        assert!(bad.contains("profiles[1].tab_color"), "{bad}");
    }

    #[test]
    fn a_wsl_profile_needs_no_command() {
        let loaded = load(r#"return { profiles = { { name = "Dev", wsl = "Ubuntu" } } }"#);
        let dev = &loaded.config.profiles[0];
        assert_eq!(dev.command, "wsl.exe");
        assert_eq!(dev.args, ["-d", "Ubuntu", "--cd", "~"]);
        assert_eq!(dev.wsl.as_deref(), Some("Ubuntu"));
        let plain = load(r#"return { profiles = { { name = "x", command = "cmd.exe" } } }"#);
        assert_eq!(plain.config.profiles[0].wsl, None);
    }

    #[test]
    fn type_errors_say_where() {
        assert!(error(r#"return { font = { size = "big" } }"#).contains("font.size"));
        assert!(error(r#"return { padding = {} }"#).contains("padding"));
        assert!(
            error(r#"return { colors = { background = "blue" } }"#).contains("colors.background")
        );
        assert!(
            error(r#"return { profiles = { { name = "x" } } }"#).contains("profiles[1].command")
        );
        assert!(error(r#"return { braille_style = "round" }"#).contains("braille_style"));
        let keys = error(r#"return { keys = { { key = "ctrl+nope", action = "new_tab" } } }"#);
        assert!(keys.contains("keys[1]") && keys.contains("nope"), "{keys}");
        assert!(error(r#"return { keys = { { key = "f1", action = "fly" } } }"#).contains("fly"));
        assert!(error(r#"return { font = { size = -3 } }"#).contains("font.size"));
    }

    #[test]
    fn syntax_errors_have_the_line() {
        let err = error("return {\n  font = { size = 14 \n}");
        assert!(err.contains("test.lua"), "{err}");
        assert!(err.contains(':'), "{err}");
        assert!(error("return 5").contains("table"));
    }

    #[test]
    fn the_sandbox_has_no_files_or_programs() {
        let loaded = load(
            // `loadstring` and `require` stay: they only read Lua code, they do not run programs.
            r#"return { padding = if io == nil and os.execute == nil and os.remove == nil then 1 else 2 }"#,
        );
        assert_eq!(loaded.config.padding, 1.0);
    }

    #[test]
    fn defaults_have_no_lua_functions() {
        let loaded = LoadedConfig::defaults();
        assert_eq!(loaded.config.font_size, 14.0);
        assert!(loaded.call(0).is_err());
    }

    #[test]
    fn notification_defaults() {
        let n = load("return {}").config.notifications;
        assert_eq!(n.toasts, Some(ToastPosition::BottomRight));
        assert_eq!(n.os, OsNotify::Never, "OS notifications are off by default");
        assert_eq!(n.max_visible, 4);
        assert_eq!(n.long_command, 10.0);
        assert!(!n.bell);
    }

    #[test]
    fn notification_settings() {
        let n = load(
            r#"return { notifications = {
              toasts = "top_left", max_visible = 2, os = "when_unfocused",
              long_command = 30, bell = "notify", flash = false,
            } }"#,
        )
        .config
        .notifications;
        assert_eq!(n.toasts, Some(ToastPosition::TopLeft));
        assert_eq!(n.max_visible, 2);
        assert_eq!(n.os, OsNotify::WhenUnfocused);
        assert_eq!(n.long_command, 30.0);
        assert!(n.bell && !n.flash);

        let off = load("return { notifications = { toasts = false, os = true } }")
            .config
            .notifications;
        assert_eq!(off.toasts, None);
        assert_eq!(off.os, OsNotify::Always);

        let levels = load(
            r#"return { notifications = { os = { when = "always", levels = { "attention", "error" } } } }"#,
        )
        .config
        .notifications;
        assert_eq!(levels.os, OsNotify::Always);
        assert_eq!(levels.os_levels, ["attention", "error"]);

        assert!(
            error(r#"return { notifications = { toasts = "middle" } }"#)
                .contains("notifications.toasts")
        );
        assert!(
            error(r#"return { notifications = { os = "sometimes" } }"#)
                .contains("notifications.os")
        );
    }

    fn input(body: &str) -> NotifyIn {
        NotifyIn {
            title: "T".into(),
            body: body.into(),
            level: "info".into(),
            source: "terminal".into(),
            pane: Some(3),
        }
    }

    #[test]
    fn without_on_notification_everything_stays() {
        let loaded = load("return {}");
        assert_eq!(
            loaded.filter_notification(&input("x")).unwrap(),
            NotifyOut::Keep {
                title: "T".into(),
                body: "x".into(),
                level: "info".into(),
                os: None,
                toast: None
            }
        );
    }

    #[test]
    fn on_notification_can_drop_change_and_route() {
        let loaded = load(
            r#"return { on_notification = function(n, fterm)
              if n.body:find("spam") then return nil end
              if n.source == "terminal" and n.pane == 3 then
                n.level = "attention"; n.os = true; n.toast = false
                n.title = n.title .. "!"
              end
              return n
            end }"#,
        );
        assert_eq!(
            loaded
                .filter_notification(&input("some spam here"))
                .unwrap(),
            NotifyOut::Drop
        );
        assert_eq!(
            loaded.filter_notification(&input("ok")).unwrap(),
            NotifyOut::Keep {
                title: "T!".into(),
                body: "ok".into(),
                level: "attention".into(),
                os: Some(true),
                toast: Some(false)
            }
        );
    }

    #[test]
    fn on_notification_errors_are_reported() {
        let loaded = load(r#"return { on_notification = function(n) error("bad filter") end }"#);
        assert!(
            loaded
                .filter_notification(&input("x"))
                .unwrap_err()
                .contains("bad filter")
        );
    }

    fn agent(state: &str) -> AgentIn {
        AgentIn {
            pane: 2,
            state: state.into(),
            previous: Some("working".into()),
            message: "Allow Bash?".into(),
            name: "claude".into(),
        }
    }

    fn title_in() -> TitleIn {
        TitleIn {
            tab: 2,
            tabs: 3,
            title: "claude".into(),
            agent: Some("working".into()),
            waiting: 1,
            failed: 0,
            default: "[2/3] claude — ⏳ 1 waiting".into(),
        }
    }

    #[test]
    fn without_window_title_the_default_is_used() {
        assert_eq!(load("return {}").window_title(&title_in()).unwrap(), None);
    }

    #[test]
    fn window_title_makes_the_title() {
        let loaded = load(
            r#"return { window_title = function(t)
              if t.agent == "working" then return "◐ " .. t.title .. " (" .. t.tab .. "/" .. t.tabs .. ")" end
              if t.waiting > 0 then return t.default end
            end }"#,
        );
        assert_eq!(
            loaded.window_title(&title_in()).unwrap().as_deref(),
            Some("◐ claude (2/3)")
        );
        let idle = TitleIn {
            agent: None,
            ..title_in()
        };
        assert_eq!(
            loaded.window_title(&idle).unwrap().as_deref(),
            Some("[2/3] claude — ⏳ 1 waiting")
        );
        let quiet = TitleIn {
            agent: None,
            waiting: 0,
            ..title_in()
        };
        assert_eq!(
            loaded.window_title(&quiet).unwrap(),
            None,
            "nil = the default"
        );
    }

    fn ai_in() -> AiRequestIn {
        AiRequestIn {
            question: "why?".into(),
            text: "<terminal>token=abc</terminal>\n\nwhy?".into(),
            provider: "anthropic".into(),
            model: "claude-haiku-4-5-20251001".into(),
        }
    }

    fn saved() -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "saved": 5,
            "active_tab": 0,
            "tabs": [
                { "active": 0, "layout": { "pane": { "cwd": "C:/work", "program": "pwsh" } } },
                { "active": 0, "layout": { "pane": { "cwd": "C:/tmp/x", "program": "pwsh", "ran": "npm run dev" } } }
            ]
        })
    }

    #[test]
    fn without_on_restore_the_session_comes_back_as_it_is() {
        assert_eq!(
            load("return {}").on_restore(&saved()).unwrap(),
            Some(saved())
        );
    }

    #[test]
    fn on_restore_can_change_the_session() {
        let loaded = load(
            r#"return { on_restore = function(s)
              local keep = {}
              for _, tab in ipairs(s.tabs) do
                if not tab.layout.pane.cwd:find("tmp") then table.insert(keep, tab) end
              end
              s.tabs = keep
              return s
            end }"#,
        );
        let out = loaded.on_restore(&saved()).unwrap().unwrap();
        assert_eq!(out["tabs"].as_array().map(Vec::len), Some(1));
        assert_eq!(out["tabs"][0]["layout"]["pane"]["cwd"], "C:/work");
        assert_eq!(out["saved"], 5);
    }

    #[test]
    fn on_restore_can_stop_it() {
        let loaded = load("return { on_restore = function(s) return #s.tabs < 2 end }");
        assert_eq!(loaded.on_restore(&saved()).unwrap(), None);
        let loaded = load("return { on_restore = function(s) return 42 end }");
        assert!(loaded.on_restore(&saved()).is_err());
    }

    #[test]
    fn without_on_ai_request_the_text_goes_as_it_is() {
        assert_eq!(
            load("return {}").on_ai_request(&ai_in()).unwrap(),
            Some(ai_in().text)
        );
    }

    #[test]
    fn on_ai_request_can_stop_or_change_the_text() {
        let loaded = load(
            r#"return { on_ai_request = function(r)
              if r.question == "secret" then return false end
              if r.provider == "anthropic" and r.model:find("haiku") then
                r.text = r.text:gsub("token=%w+", "token=***")
                return r
              end
            end }"#,
        );
        assert_eq!(
            loaded.on_ai_request(&ai_in()).unwrap().as_deref(),
            Some("<terminal>token=***</terminal>\n\nwhy?")
        );
        let secret = AiRequestIn {
            question: "secret".into(),
            ..ai_in()
        };
        assert_eq!(loaded.on_ai_request(&secret).unwrap(), None);
    }

    #[test]
    fn ai_defaults() {
        let ai = load("return {}").config.ai;
        assert_eq!(ai.provider, "anthropic");
        assert_eq!(ai.max_tokens, 2048);
        let p = ai.current().unwrap();
        assert_eq!(
            (p.name.as_str(), p.kind, p.model.as_str()),
            ("anthropic", AiKind::Anthropic, "claude-haiku-4-5-20251001")
        );
        assert!(p.needs_key);
        let ollama = ai.providers.iter().find(|p| p.name == "ollama").unwrap();
        assert_eq!(
            (ollama.kind, ollama.url.as_str()),
            (AiKind::OpenAi, "http://localhost:11434/v1")
        );
        assert!(!ollama.needs_key, "a local server needs no key");
        assert!(ai.providers.iter().any(|p| p.name == "openrouter"));
    }

    #[test]
    fn ai_from_the_config() {
        let ai = load(
            r#"return { ai = {
              provider = "ollama",
              system = "Answer in Russian.",
              max_tokens = 500,
              providers = {
                ollama = { model = "qwen2.5-coder" },
                anthropic = { model = "claude-sonnet-5" },
                lmstudio = { kind = "openai", url = "http://localhost:1234/v1", model = "local", key = false },
              },
            } }"#,
        )
        .config
        .ai;
        assert_eq!(ai.provider, "ollama");
        assert_eq!(ai.system, "Answer in Russian.");
        assert_eq!(ai.max_tokens, 500);
        let ollama = ai.current().unwrap();
        assert_eq!(ollama.model, "qwen2.5-coder");
        assert_eq!(
            ollama.url, "http://localhost:11434/v1",
            "the rest of the preset stays"
        );
        let anthropic = ai.providers.iter().find(|p| p.name == "anthropic").unwrap();
        assert_eq!(anthropic.model, "claude-sonnet-5");
        let lm = ai.providers.iter().find(|p| p.name == "lmstudio").unwrap();
        assert_eq!(
            (lm.kind, lm.url.as_str(), lm.needs_key),
            (AiKind::OpenAi, "http://localhost:1234/v1", false)
        );
    }

    #[test]
    fn a_model_for_commands() {
        assert_eq!(load("return {}").config.ai.command_model, None);
        let ai = load(r#"return { ai = { command_model = "claude-sonnet-5" } }"#)
            .config
            .ai;
        assert_eq!(ai.command_model.as_deref(), Some("claude-sonnet-5"));
    }

    #[test]
    fn ai_for_api_clients() {
        assert!(
            !load("return {}").config.ai.api_access,
            "off: questions cost money"
        );
        assert!(
            load("return { ai = { api_access = true } }")
                .config
                .ai
                .api_access
        );
        let Err(err) = load_str(r#"return { ai = { api_access = "yes" } }"#, "t") else {
            panic!("a string must fail");
        };
        assert!(err.contains("ai.api_access"), "{err}");
    }

    #[test]
    fn bad_ai_config() {
        for (source, part) in [
            (r#"return { ai = { provider = "nope" } }"#, "ai.provider"),
            (
                r#"return { ai = { providers = { x = { kind = "openai" } } } }"#,
                "ai.providers.x",
            ),
            (
                r#"return { ai = { providers = { x = { kind = "smoke", url = "u", model = "m" } } } }"#,
                "ai.providers.x.kind",
            ),
        ] {
            let Err(err) = load_str(source, "t") else {
                panic!("{source} must fail");
            };
            assert!(err.contains(part), "{err}");
        }
    }

    #[test]
    fn api_config() {
        let a = load("return {}").config.api;
        assert!(
            a.enabled && a.ask,
            "on, and it asks before another pane is used"
        );
        let a = load("return { api = { enabled = false, ask = false } }")
            .config
            .api;
        assert!(!a.enabled && !a.ask);
        assert!(load_str("return { api = { ask = 1 } }", "t").is_err());
    }

    #[test]
    fn restore_values() {
        assert_eq!(load("return {}").config.restore, Restore::Ask);
        for (text, value) in [
            ("ask", Restore::Ask),
            ("always", Restore::Always),
            ("never", Restore::Never),
        ] {
            let source = format!("return {{ restore = \"{text}\" }}");
            assert_eq!(load(&source).config.restore, value);
        }
        assert!(load_str(r#"return { restore = "maybe" }"#, "t").is_err());
    }

    #[test]
    fn restore_history_lines() {
        assert_eq!(load("return {}").config.restore_history, 200);
        assert_eq!(
            load("return { restore_history = 0 }")
                .config
                .restore_history,
            0
        );
        assert_eq!(
            load("return { restore_history = 1000 }")
                .config
                .restore_history,
            1000
        );
        assert_eq!(
            load("return { restore_history = -5 }")
                .config
                .restore_history,
            0
        );
    }

    #[test]
    fn the_gpu_backend_and_power() {
        let gpu = load("return {}").config.gpu;
        assert_eq!(
            gpu,
            GpuConfig {
                backend: GpuBackend::Auto,
                power: GpuPower::High
            }
        );
        for (text, value) in [
            ("auto", GpuBackend::Auto),
            ("dx12", GpuBackend::Dx12),
            ("vulkan", GpuBackend::Vulkan),
            ("gl", GpuBackend::Gl),
            ("metal", GpuBackend::Metal),
        ] {
            let source = format!("return {{ gpu = {{ backend = \"{text}\" }} }}");
            assert_eq!(load(&source).config.gpu.backend, value, "{text}");
        }
        assert_eq!(
            load(r#"return { gpu = { power = "low" } }"#)
                .config
                .gpu
                .power,
            GpuPower::Low
        );
        let err = load_str(r#"return { gpu = { backend = "directx" } }"#, "t")
            .err()
            .unwrap();
        assert!(err.contains("gpu.backend") && err.contains("gl"), "{err}");
        assert!(load_str(r#"return { gpu = { power = "max" } }"#, "t").is_err());
    }

    #[test]
    fn the_config_path_from_the_command_line() {
        let args = |line: &str| {
            line.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(config_arg(&args("")), Ok(None));
        assert_eq!(
            config_arg(&args("--config D:/f/my.lua")),
            Ok(Some(PathBuf::from("D:/f/my.lua")))
        );
        assert_eq!(
            config_arg(&args("--config=D:/f/my.lua")),
            Ok(Some(PathBuf::from("D:/f/my.lua")))
        );
        assert!(config_arg(&args("--config")).is_err(), "no path");
        assert!(config_arg(&args("--config=")).is_err(), "an empty path");
        // Other words are not for this (for example a path that Windows gives to the program).
        assert_eq!(config_arg(&args("C:/x --other")), Ok(None));
    }

    #[test]
    fn the_data_folder_from_the_config() {
        assert_eq!(load("return {}").config.data_dir, None);
        assert_eq!(
            load(r#"return { data_dir = "data" }"#)
                .config
                .data_dir
                .as_deref(),
            Some("data")
        );
        assert!(load_str("return { data_dir = 5 }", "t").is_err());
    }

    #[test]
    fn the_config_path_in_order() {
        let user = PathBuf::from("C:/Users/me/AppData/Roaming/fterm/fterm.lua");
        let exe = Path::new("D:/tools/fterm");
        let there = |p: &Path| p == Path::new("D:/tools/fterm").join("fterm.lua");
        let none = |_: &Path| false;
        let arg = Path::new("E:/a.lua");
        let env = Path::new("E:/b.lua");
        assert_eq!(
            choose_config_path(Some(arg), Some(env), Some(exe), there, user.clone()),
            arg
        );
        assert_eq!(
            choose_config_path(None, Some(env), Some(exe), there, user.clone()),
            env
        );
        // A portable fterm: its config is next to it.
        assert_eq!(
            choose_config_path(None, None, Some(exe), there, user.clone()),
            exe.join("fterm.lua")
        );
        assert_eq!(
            choose_config_path(None, None, Some(exe), none, user.clone()),
            user
        );
        assert_eq!(
            choose_config_path(None, None, None, there, user.clone()),
            user
        );
    }

    #[test]
    fn rerun_values() {
        let config = load("return {}").config;
        assert_eq!(config.restore_programs, Rerun::Prompt);
        assert_eq!(config.restore_agents, Rerun::Prompt);
        for (text, value) in [
            ("prompt", Rerun::Prompt),
            ("run", Rerun::Run),
            ("never", Rerun::Never),
        ] {
            let source =
                format!("return {{ restore_programs = \"{text}\", restore_agents = \"{text}\" }}");
            let config = load(&source).config;
            assert_eq!(config.restore_programs, value);
            assert_eq!(config.restore_agents, value);
        }
        assert!(load_str(r#"return { restore_agents = "yes" }"#, "t").is_err());
    }

    #[test]
    fn confirm_close_values() {
        assert_eq!(
            load("return {}").config.confirm_close,
            ConfirmClose::Running
        );
        for (text, value) in [
            ("running", ConfirmClose::Running),
            ("always", ConfirmClose::Always),
            ("never", ConfirmClose::Never),
        ] {
            let source = format!("return {{ confirm_close = \"{text}\" }}");
            assert_eq!(load(&source).config.confirm_close, value);
        }
        let Err(err) = load_str(r#"return { confirm_close = "maybe" }"#, "t") else {
            panic!("a bad value");
        };
        assert!(err.contains("confirm_close"), "{err}");
    }

    fn close_in() -> CloseIn {
        CloseIn {
            tabs: 2,
            panes: 3,
            running: vec![(2, "cargo".into())],
            agents: vec![(1, "claude".into(), "working".into())],
        }
    }

    #[test]
    fn without_on_close_window_the_rule_decides() {
        assert_eq!(
            load("return {}").on_close_window(&close_in()).unwrap(),
            None
        );
    }

    #[test]
    fn on_close_window_sees_what_runs() {
        let loaded = load(
            r#"return { on_close_window = function(info)
              for _, a in ipairs(info.agents) do
                if a.state == "working" and a.tab == 1 and a.name == "claude" then return false end
              end
              if info.tabs == 2 and info.panes == 3 and info.running[1].program == "cargo" then return true end
            end }"#,
        );
        assert_eq!(loaded.on_close_window(&close_in()).unwrap(), Some(false));
        let no_agents = CloseIn {
            agents: vec![],
            ..close_in()
        };
        assert_eq!(loaded.on_close_window(&no_agents).unwrap(), Some(true));
        let other = CloseIn {
            tabs: 5,
            agents: vec![],
            ..close_in()
        };
        assert_eq!(
            loaded.on_close_window(&other).unwrap(),
            None,
            "nil = the rule"
        );
    }

    #[test]
    fn history_defaults_and_fields() {
        let h = load("return {}").config.history;
        assert_eq!(h, HistoryConfig::default());
        assert!(h.enabled && h.ignore_space && !h.hints);
        assert_eq!((h.commands, h.dirs), (10_000, 500));
        let h = load(
            r#"return { history = { enabled = false, commands = 50, dirs = 20, ignore_space = false, hints = true } }"#,
        )
        .config
        .history;
        assert!(!h.enabled && !h.ignore_space && h.hints);
        assert_eq!((h.commands, h.dirs), (50, 20));
        let Err(err) = load_str("return { history = { commands = 0 } }", "t") else {
            panic!("0 commands is not a history");
        };
        assert!(err.contains("history.commands"), "{err}");
    }

    fn history_in(cmd: &str) -> HistoryIn {
        HistoryIn {
            cmd: cmd.into(),
            cwd: Some("C:/work".into()),
            exit: Some(0),
            shell: "pwsh".into(),
        }
    }

    #[test]
    fn without_on_history_the_command_is_saved() {
        let out = load("return {}").on_history(&history_in("ls")).unwrap();
        assert_eq!(out, Some("ls".to_owned()));
    }

    #[test]
    fn on_history_can_skip_and_change() {
        let loaded = load(
            r#"return { on_history = function(h)
              if h.cmd:find("token") then return false end
              if h.cwd == "C:/work" and h.shell == "pwsh" and h.exit == 0 then
                h.cmd = h.cmd:gsub("password=%S+", "password=***")
                return h
              end
            end }"#,
        );
        assert_eq!(
            loaded.on_history(&history_in("curl --token x")).unwrap(),
            None
        );
        assert_eq!(
            loaded.on_history(&history_in("db password=abc")).unwrap(),
            Some("db password=***".to_owned())
        );
        // nil = keep it as it is.
        let other = HistoryIn {
            exit: Some(1),
            ..history_in("make")
        };
        assert_eq!(loaded.on_history(&other).unwrap(), Some("make".to_owned()));
    }

    #[test]
    fn panels_default_is_a_closed_right_dock() {
        let p = load("return {}").config.panels;
        assert_eq!(p, PanelsConfig::default());
        assert_eq!(p.dock, DockPlace::Right);
        assert_eq!(p.open, None);
        assert!((p.size - 0.28).abs() < 1e-6);
    }

    #[test]
    fn panels_from_the_config() {
        let p = load(r#"return { panels = { dock = "bottom", size = 0.4, open = { "agents" } } }"#)
            .config
            .panels;
        assert_eq!(p.dock, DockPlace::Bottom);
        assert!((p.size - 0.4).abs() < 1e-6);
        assert_eq!(p.open.as_deref(), Some("agents"));
        let p = load(r#"return { panels = { dock = "left", open = {} } }"#)
            .config
            .panels;
        assert_eq!((p.dock, p.open), (DockPlace::Left, None));
    }

    #[test]
    fn bad_panels_are_errors() {
        for (source, part) in [
            (r#"return { panels = { dock = "top" } }"#, "panels.dock"),
            (r#"return { panels = { size = 2 } }"#, "panels.size"),
            (
                r#"return { panels = { open = { "chat" } } }"#,
                "panels.open",
            ),
        ] {
            let Err(err) = load_str(source, "t") else {
                panic!("{source} must fail");
            };
            assert!(err.contains(part), "{err}");
        }
    }

    #[test]
    fn without_on_agent_the_notification_stays() {
        let out = load("return {}").on_agent(&agent("waiting")).unwrap();
        assert_eq!(
            out,
            AgentOut {
                notify: true,
                calls: vec![]
            }
        );
    }

    #[test]
    fn on_agent_sees_the_state_and_can_act() {
        let loaded = load(
            r#"return { on_agent = function(a, fterm)
              if a.state == "waiting" and a.previous == "working" and a.pane == 2 then
                fterm.notify{ title = a.name .. ": " .. a.message, level = "warning" }
                return false
              end
            end }"#,
        );
        let out = loaded.on_agent(&agent("waiting")).unwrap();
        assert!(!out.notify, "false = no built-in notification");
        assert_eq!(
            out.calls,
            vec![ApiCall::Notify {
                title: "claude: Allow Bash?".into(),
                body: String::new(),
                level: "warning".into()
            }]
        );
        let out = loaded.on_agent(&agent("done")).unwrap();
        assert_eq!(
            out,
            AgentOut {
                notify: true,
                calls: vec![]
            },
            "nil = keep"
        );
    }

    fn row_in() -> AgentRowIn {
        AgentRowIn {
            pane: 3,
            name: "claude".into(),
            state: "working".into(),
            question: None,
            todos_done: 1,
            todos_total: 3,
            todo_current: Some("Writing tests".into()),
            cost: Some(6.2),
            context: Some(81.0),
            files: 2,
            last: Some("edited app.rs".into()),
        }
    }

    #[test]
    fn on_agent_row_makes_the_row() {
        let loaded = load(
            r#"return { on_agent_row = function(a)
                local right = nil
                if a.context and a.context > 80 then right = "ctx " .. math.floor(a.context) .. "%!" end
                return { title = a.name .. " · " .. a.todos.done .. "/" .. a.todos.total, right = right }
            end }"#,
        );
        let out = loaded.on_agent_row(&row_in()).unwrap().unwrap();
        assert_eq!(out.title.as_deref(), Some("claude · 1/3"));
        assert_eq!(out.right.as_deref(), Some("ctx 81%!"));
        assert_eq!(out.detail, None, "a missing part = the default");
        // nil = the default row; no function = no change.
        let none = load("return { on_agent_row = function(a) return nil end }");
        assert_eq!(none.on_agent_row(&row_in()).unwrap(), None);
        assert_eq!(load("return {}").on_agent_row(&row_in()).unwrap(), None);
    }

    #[test]
    fn on_agent_event_sees_every_step() {
        let loaded = load(
            r#"return { on_agent_event = function(e, fterm)
                if e.kind == "usage" and e.cost > 5 then fterm.notify("expensive: " .. e.pane) end
                if e.kind == "tool" and e.tool == "Bash" then fterm.notify("ran " .. e.command) end
            end }"#,
        );
        let usage = AgentEventIn {
            pane: 3,
            kind: "usage".into(),
            cost: Some(6.0),
            ..Default::default()
        };
        assert_eq!(
            loaded.on_agent_event(&usage).unwrap(),
            [ApiCall::Notify {
                title: String::new(),
                body: "expensive: 3".into(),
                level: "info".into()
            }]
        );
        let tool = AgentEventIn {
            pane: 3,
            kind: "tool".into(),
            tool: Some("Bash".into()),
            command: Some("cargo test".into()),
            ..Default::default()
        };
        assert_eq!(loaded.on_agent_event(&tool).unwrap().len(), 1);
        assert!(load("return {}").on_agent_event(&tool).unwrap().is_empty());
        assert!(error("return { on_agent_event = 5 }").contains("on_agent_event"));
    }

    #[test]
    fn on_agent_must_be_a_function() {
        let Err(err) = load_str("return { on_agent = 5 }", "t") else {
            panic!("a number is not a function");
        };
        assert!(err.contains("on_agent"));
    }

    #[test]
    fn fterm_notify_takes_a_string_or_a_table() {
        let loaded = load(
            r#"return { keys = { { key = "f7", action = function(fterm)
              fterm.notify("just text")
              fterm.notify({ title = "Build", body = "done", level = "success" })
            end } } }"#,
        );
        let Some(Action::Lua(index)) = loaded
            .config
            .keys
            .get(&KeyChord::parse("f7").unwrap())
            .cloned()
        else {
            panic!("no f7");
        };
        assert_eq!(
            loaded.call(index).unwrap(),
            [
                ApiCall::Notify {
                    title: String::new(),
                    body: "just text".into(),
                    level: "info".into()
                },
                ApiCall::Notify {
                    title: "Build".into(),
                    body: "done".into(),
                    level: "success".into()
                },
            ]
        );
    }
}
