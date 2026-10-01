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
    /// The Lua function `on_notification` (its number), if there is one.
    pub on_notification: Option<usize>,
    /// The Lua function `on_agent` (its number), if there is one.
    pub on_agent: Option<usize>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            padding: 6.0,
            scrollback: 10_000,
            braille_style: BrailleStyle::Pixels,
            colors: ColorConfig::default(),
            default_profile: None,
            profiles: Vec::new(),
            keys: Keymap::with_defaults(),
            commands: Vec::new(),
            shell_integration: true,
            notifications: NotificationConfig::default(),
            panels: PanelsConfig::default(),
            on_notification: None,
            on_agent: None,
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
    let command = string_field(table, "command", &format!("{path}.command"))?
        .ok_or_else(|| format!("{path}.command: missing"))?;
    let mut args = Vec::new();
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

/// Where the config file is: `FTERM_CONFIG`, or `%APPDATA%\fterm\fterm.lua`, or `~/.config/fterm/fterm.lua`.
pub fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("FTERM_CONFIG") {
        return PathBuf::from(path);
    }
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
