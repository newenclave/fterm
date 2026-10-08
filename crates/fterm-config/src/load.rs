//! Loading `fterm.lua`: run the Luau script and read the table it returns.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{Function, Lua, Table, Value};

use crate::colors::{ColorConfig, Rgb, parse_color};
use crate::keys::{Action, BuiltinAction, KeyChord, Keymap, SpawnWhere};
use crate::profiles::{Profile, expand_home, home_dir};

/// How Braille chars are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BrailleStyle {
    #[default]
    Pixels,
    Dots,
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
    /// The profile for new tabs and splits. `None` = the first profile.
    pub default_profile: Option<String>,
    /// Profiles from the config. Empty = fterm finds them itself.
    pub profiles: Vec<Profile>,
    pub keys: Keymap,
    pub commands: Vec<UserCommand>,
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
            braille_style: BrailleStyle::Pixels,
            colors: ColorConfig::default(),
            theme: crate::theme::ThemeChoice::Default,
            harmonize: HarmonizeConfig::default(),
            palette_changes: true,
            plan_review: PlanReview::default(),
            language: None,
            default_profile: None,
            profiles: Vec::new(),
            keys: Keymap::with_defaults(),
            commands: Vec::new(),
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
        "action",
        lua.create_function(move |_, name: String| {
            let action = BuiltinAction::from_name(&name)
                .ok_or_else(|| mlua::Error::runtime(format!("unknown action `{name}`")))?;
            q.borrow_mut().push(ApiCall::Action(action));
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
        if let Some(colors) = table_field(root, "colors", "colors")? {
            config.colors = self.colors(&colors)?;
        }
        config.theme = theme_choice(root)?;
        if let Some(on) = bool_field(root, "palette_changes", "palette_changes")? {
            config.palette_changes = on;
        }
        config.language = string_field(root, "language", "language")?;
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
    })
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
              braille_style = "dots",
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
        assert_eq!(config.braille_style, BrailleStyle::Dots);
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
