//! Key bindings: "ctrl+shift+t" → an action.

use std::collections::HashMap;

/// Modifier keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// The Windows key (Command on macOS).
    pub logo: bool,
}

/// A key by its place on the keyboard (US names), so bindings work on every layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A letter (`a`..`z`), a digit, or a symbol key (`=` `-` `[` `]` `;` `'` `,` `.` `/` `\` `` ` ``).
    Char(char),
    Tab,
    Enter,
    Escape,
    Space,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    F(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub mods: Mods,
    pub key: Key,
}

impl KeyChord {
    /// Reads `ctrl+shift+t`, `alt+shift+=`, `ctrl+tab`, `f11`, `shift+pageup`, ...
    pub fn parse(text: &str) -> Result<Self, String> {
        let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let clean = clean.to_lowercase();
        if clean.is_empty() {
            return Err("empty key".to_owned());
        }
        // The last part is the key. `+` itself is written as `plus`, so "ctrl++" is an error.
        let parts: Vec<&str> = clean.split('+').collect();
        let (key_name, mod_names) = parts.split_last().expect("split gives at least one part");
        let mut mods = Mods::default();
        for name in mod_names {
            match *name {
                "ctrl" | "control" => mods.ctrl = true,
                "shift" => mods.shift = true,
                "alt" | "option" => mods.alt = true,
                "win" | "super" | "cmd" | "logo" => mods.logo = true,
                "" => return Err(format!("`{text}`: a `+` without a key (write `plus`)")),
                other => return Err(format!("`{text}`: unknown modifier `{other}`")),
            }
        }
        let key = parse_key(key_name).ok_or_else(|| {
            if key_name.is_empty() {
                format!("`{text}`: no key after the modifiers")
            } else {
                format!("`{text}`: unknown key `{key_name}`")
            }
        })?;
        Ok(Self { mods, key })
    }

    /// For people: "Ctrl+Shift+T".
    pub fn display(&self) -> String {
        let mut out = String::new();
        for (on, name) in [
            (self.mods.ctrl, "Ctrl+"),
            (self.mods.alt, "Alt+"),
            (self.mods.shift, "Shift+"),
            (self.mods.logo, "Win+"),
        ] {
            if on {
                out.push_str(name);
            }
        }
        let key = match self.key {
            Key::Char(c) => c.to_ascii_uppercase().to_string(),
            Key::F(n) => format!("F{n}"),
            Key::PageUp => "PageUp".into(),
            Key::PageDown => "PageDown".into(),
            other => format!("{other:?}"),
        };
        out.push_str(&key);
        out
    }
}

fn parse_key(name: &str) -> Option<Key> {
    let key = match name {
        "tab" => Key::Tab,
        "enter" | "return" => Key::Enter,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" | "ins" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "plus" => Key::Char('='),
        "minus" => Key::Char('-'),
        _ => {
            if let Some(number) = name.strip_prefix('f')
                && let Ok(n) = number.parse::<u8>()
            {
                return (1..=24).contains(&n).then_some(Key::F(n));
            }
            let mut chars = name.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || "=-[];',./\\`".contains(c);
            return ok.then_some(Key::Char(c));
        }
    };
    Some(key)
}

/// Actions of fterm itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinAction {
    NewTab,
    ClosePane,
    NextTab,
    PrevTab,
    /// Tab number, from 0.
    SelectTab(usize),
    LastTab,
    MoveTabLeft,
    MoveTabRight,
    RenameTab,
    SplitRight,
    SplitDown,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
    Zoom,
    Copy,
    Paste,
    CopyMode,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
    CommandPalette,
    ReloadConfig,
    OpenConfig,
    /// Put the Claude Code hooks for agent badges into the clipboard.
    CopyClaudeHooks,
    /// Open or close the dock with the service panels.
    ToggleDock,
    PanelEvents,
    PanelAgents,
    /// Move the keyboard between the terminal and the dock.
    FocusDock,
    /// The command history popup (like Alt+F8 in Far).
    HistoryCommands,
    /// The folder history popup (like Alt+F12 in Far).
    HistoryDirs,
    /// API clients may (not) read and type into the active pane.
    ToggleRemoteControl,
    /// The AI panel with the keyboard.
    PanelAi,
    /// Ask for the API key of the AI provider and save it in the key store.
    SetAiKey,
    /// Send the last command, its exit code, and its output to the AI: "why did it fail?".
    ExplainError,
    /// Open the AI panel with the selected text as context.
    AskAiSelection,
    /// The task typed in the prompt becomes a command (Phase 9).
    TextToCommand,
    /// Open the tabs of the last saved session again (Phase 9b).
    RestoreSession,
    /// The list of saved sessions.
    Sessions,
    /// Save this window as a named session.
    SaveSessionAs,
    /// A Braille scene pane next to the active pane (Phase 11).
    NewScene,
    /// Write the fterm skill for Claude Code (~/.claude/skills/fterm/SKILL.md).
    InstallClaudeSkill,
    /// Full screen with no window frame (like Alt+Enter in WezTerm).
    ToggleFullscreen,
    /// The list of themes; Enter uses one.
    ChooseTheme,
    /// Put the fterm hooks into the Claude Code settings (after a question).
    InstallClaudeHooks,
    /// The colors that programs chose, as they are, or fitted to the theme (`harmonize`).
    ToggleOriginalColors,
    /// The colors of programs one step closer to the theme (`harmonize` strength + 0.1).
    HarmonizeMore,
    /// One step back to their own colors (strength - 0.1).
    HarmonizeLess,
}

impl BuiltinAction {
    /// All actions with one value (for the palette and the docs). `SelectTab` is there as tab 1.
    pub const ALL: [BuiltinAction; 54] = [
        Self::NewTab,
        Self::ClosePane,
        Self::NextTab,
        Self::PrevTab,
        Self::SelectTab(0),
        Self::LastTab,
        Self::MoveTabLeft,
        Self::MoveTabRight,
        Self::RenameTab,
        Self::SplitRight,
        Self::SplitDown,
        Self::FocusLeft,
        Self::FocusRight,
        Self::FocusUp,
        Self::FocusDown,
        Self::ResizeLeft,
        Self::ResizeRight,
        Self::ResizeUp,
        Self::ResizeDown,
        Self::Zoom,
        Self::Copy,
        Self::Paste,
        Self::CopyMode,
        Self::ScrollPageUp,
        Self::ScrollPageDown,
        Self::ScrollTop,
        Self::ScrollBottom,
        Self::CommandPalette,
        Self::ReloadConfig,
        Self::OpenConfig,
        Self::CopyClaudeHooks,
        Self::ToggleDock,
        Self::PanelEvents,
        Self::PanelAgents,
        Self::FocusDock,
        Self::HistoryCommands,
        Self::HistoryDirs,
        Self::ToggleRemoteControl,
        Self::PanelAi,
        Self::SetAiKey,
        Self::ExplainError,
        Self::AskAiSelection,
        Self::TextToCommand,
        Self::RestoreSession,
        Self::Sessions,
        Self::SaveSessionAs,
        Self::NewScene,
        Self::InstallClaudeSkill,
        Self::ToggleFullscreen,
        Self::ChooseTheme,
        Self::InstallClaudeHooks,
        Self::ToggleOriginalColors,
        Self::HarmonizeMore,
        Self::HarmonizeLess,
    ];

    /// The name in the config, for example `new_tab` or `select_tab_3`.
    pub fn name(self) -> String {
        let name = match self {
            Self::NewTab => "new_tab",
            Self::ClosePane => "close_pane",
            Self::NextTab => "next_tab",
            Self::PrevTab => "prev_tab",
            Self::SelectTab(i) => return format!("select_tab_{}", i + 1),
            Self::LastTab => "last_tab",
            Self::MoveTabLeft => "move_tab_left",
            Self::MoveTabRight => "move_tab_right",
            Self::RenameTab => "rename_tab",
            Self::SplitRight => "split_right",
            Self::SplitDown => "split_down",
            Self::FocusLeft => "focus_left",
            Self::FocusRight => "focus_right",
            Self::FocusUp => "focus_up",
            Self::FocusDown => "focus_down",
            Self::ResizeLeft => "resize_left",
            Self::ResizeRight => "resize_right",
            Self::ResizeUp => "resize_up",
            Self::ResizeDown => "resize_down",
            Self::Zoom => "zoom",
            Self::Copy => "copy",
            Self::Paste => "paste",
            Self::CopyMode => "copy_mode",
            Self::ScrollPageUp => "scroll_page_up",
            Self::ScrollPageDown => "scroll_page_down",
            Self::ScrollTop => "scroll_top",
            Self::ScrollBottom => "scroll_bottom",
            Self::CommandPalette => "command_palette",
            Self::ReloadConfig => "reload_config",
            Self::OpenConfig => "open_config",
            Self::CopyClaudeHooks => "copy_claude_hooks",
            Self::ToggleDock => "toggle_dock",
            Self::PanelEvents => "panel_events",
            Self::PanelAgents => "panel_agents",
            Self::FocusDock => "focus_dock",
            Self::HistoryCommands => "history_commands",
            Self::HistoryDirs => "history_dirs",
            Self::ToggleRemoteControl => "toggle_remote_control",
            Self::PanelAi => "panel_ai",
            Self::SetAiKey => "set_ai_key",
            Self::ExplainError => "explain_error",
            Self::AskAiSelection => "ask_ai_selection",
            Self::TextToCommand => "text_to_command",
            Self::RestoreSession => "restore_session",
            Self::Sessions => "sessions",
            Self::SaveSessionAs => "save_session_as",
            Self::NewScene => "new_scene",
            Self::InstallClaudeSkill => "install_claude_skill",
            Self::ToggleFullscreen => "toggle_fullscreen",
            Self::ChooseTheme => "choose_theme",
            Self::InstallClaudeHooks => "install_claude_hooks",
            Self::ToggleOriginalColors => "toggle_original_colors",
            Self::HarmonizeMore => "harmonize_more",
            Self::HarmonizeLess => "harmonize_less",
        };
        name.to_owned()
    }

    pub fn from_name(name: &str) -> Option<Self> {
        if let Some(number) = name.strip_prefix("select_tab_") {
            let n: usize = number.parse().ok()?;
            return (n >= 1).then(|| Self::SelectTab(n - 1));
        }
        Self::ALL
            .into_iter()
            .find(|action| !matches!(action, Self::SelectTab(_)) && action.name() == name)
    }

    /// Words for people, for the palette: "New tab", "Split right".
    pub fn label(self) -> String {
        match self {
            Self::SelectTab(i) => format!("Go to tab {}", i + 1),
            Self::ClosePane => "Close pane".to_owned(),
            Self::PrevTab => "Previous tab".to_owned(),
            Self::Zoom => "Zoom pane".to_owned(),
            Self::CommandPalette => "Command palette".to_owned(),
            Self::OpenConfig => "Open config file".to_owned(),
            Self::CopyClaudeHooks => "Copy Claude Code hooks (settings.json)".to_owned(),
            Self::ToggleDock => "Show or hide the dock".to_owned(),
            Self::PanelEvents => "Events panel".to_owned(),
            Self::PanelAgents => "Agents panel".to_owned(),
            Self::FocusDock => "Focus the dock or the terminal".to_owned(),
            Self::HistoryCommands => "Command history".to_owned(),
            Self::HistoryDirs => "Folder history".to_owned(),
            Self::ToggleRemoteControl => "Remote control on or off for this pane".to_owned(),
            Self::PanelAi => "AI panel".to_owned(),
            Self::SetAiKey => "Set the AI key".to_owned(),
            Self::ExplainError => "Explain the last error (AI)".to_owned(),
            Self::AskAiSelection => "Ask AI about the selection".to_owned(),
            Self::TextToCommand => "Text to command (AI)".to_owned(),
            Self::RestoreSession => "Restore the last session".to_owned(),
            Self::Sessions => "Sessions".to_owned(),
            Self::SaveSessionAs => "Save session as…".to_owned(),
            Self::NewScene => "New Braille scene (split right)".to_owned(),
            Self::InstallClaudeSkill => "Install the fterm skill for Claude Code".to_owned(),
            Self::ToggleFullscreen => "Full screen on or off".to_owned(),
            Self::ChooseTheme => "Theme…".to_owned(),
            Self::InstallClaudeHooks => "Install Claude Code hooks (agent states)".to_owned(),
            Self::ToggleOriginalColors => "Original colors of programs on or off".to_owned(),
            Self::HarmonizeMore => {
                "Harmonize: more (program colors closer to the theme)".to_owned()
            }
            Self::HarmonizeLess => {
                "Harmonize: less (program colors closer to their own)".to_owned()
            }
            other => {
                let name = other.name().replace('_', " ");
                let mut chars = name.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                    None => name,
                }
            }
        }
    }
}

/// Where a spawned profile goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnWhere {
    Tab,
    SplitRight,
    SplitDown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Builtin(BuiltinAction),
    /// Start a profile (`None` = the default profile).
    Spawn {
        profile: Option<String>,
        place: SpawnWhere,
    },
    /// A Lua function from the config (its number in the config).
    Lua(usize),
}

/// Keys and their actions: the defaults, then the user's bindings.
#[derive(Clone, Debug)]
pub struct Keymap {
    map: HashMap<KeyChord, Action>,
    /// The order of the keys, so `key_for` always gives the same answer.
    order: Vec<KeyChord>,
}

/// The default keys (the same as in `docs/KEYS.md`).
pub const DEFAULT_KEYS: &[(&str, &str)] = &[
    ("ctrl+shift+t", "new_tab"),
    ("ctrl+shift+w", "close_pane"),
    ("ctrl+tab", "next_tab"),
    ("ctrl+pagedown", "next_tab"),
    ("ctrl+shift+tab", "prev_tab"),
    ("ctrl+pageup", "prev_tab"),
    ("ctrl+shift+1", "select_tab_1"),
    ("ctrl+shift+2", "select_tab_2"),
    ("ctrl+shift+3", "select_tab_3"),
    ("ctrl+shift+4", "select_tab_4"),
    ("ctrl+shift+5", "select_tab_5"),
    ("ctrl+shift+6", "select_tab_6"),
    ("ctrl+shift+7", "select_tab_7"),
    ("ctrl+shift+8", "select_tab_8"),
    ("ctrl+shift+9", "last_tab"),
    ("ctrl+shift+pageup", "move_tab_left"),
    ("ctrl+shift+pagedown", "move_tab_right"),
    ("ctrl+shift+r", "rename_tab"),
    ("alt+shift+=", "split_right"),
    ("alt+shift+-", "split_down"),
    ("alt+left", "focus_left"),
    ("alt+right", "focus_right"),
    ("alt+up", "focus_up"),
    ("alt+down", "focus_down"),
    ("alt+shift+left", "resize_left"),
    ("alt+shift+right", "resize_right"),
    ("alt+shift+up", "resize_up"),
    ("alt+shift+down", "resize_down"),
    ("ctrl+shift+z", "zoom"),
    ("ctrl+shift+c", "copy"),
    ("ctrl+shift+v", "paste"),
    ("shift+insert", "paste"),
    ("ctrl+insert", "copy"),
    ("ctrl+shift+space", "copy_mode"),
    ("shift+pageup", "scroll_page_up"),
    ("shift+pagedown", "scroll_page_down"),
    ("shift+home", "scroll_top"),
    ("shift+end", "scroll_bottom"),
    ("ctrl+shift+p", "command_palette"),
    ("ctrl+shift+,", "open_config"),
    ("ctrl+shift+f5", "reload_config"),
    ("ctrl+shift+b", "toggle_dock"),
    ("ctrl+shift+e", "panel_events"),
    ("ctrl+shift+a", "panel_agents"),
    ("ctrl+shift+o", "focus_dock"),
    ("ctrl+shift+i", "panel_ai"),
    ("ctrl+shift+x", "explain_error"),
    ("ctrl+shift+g", "text_to_command"),
    ("ctrl+shift+s", "sessions"),
    ("alt+f8", "history_commands"),
    ("alt+f12", "history_dirs"),
    ("alt+enter", "toggle_fullscreen"),
    ("ctrl+shift+]", "harmonize_more"),
    ("ctrl+shift+[", "harmonize_less"),
];

impl Keymap {
    pub fn with_defaults() -> Self {
        let mut keys = Self {
            map: HashMap::new(),
            order: Vec::new(),
        };
        for (chord, name) in DEFAULT_KEYS {
            let chord = KeyChord::parse(chord).expect("default keys are valid");
            let action = BuiltinAction::from_name(name).expect("default actions are valid");
            keys.bind(chord, Some(Action::Builtin(action)));
        }
        keys
    }

    /// Adds or changes a binding. `None` removes the key.
    pub fn bind(&mut self, chord: KeyChord, action: Option<Action>) {
        match action {
            Some(action) => {
                if self.map.insert(chord, action).is_none() {
                    self.order.push(chord);
                }
            }
            None => {
                self.map.remove(&chord);
                self.order.retain(|c| *c != chord);
            }
        }
    }

    pub fn get(&self, chord: &KeyChord) -> Option<&Action> {
        self.map.get(chord)
    }

    /// The first key for an action (to show it in the palette), like "Ctrl+Shift+T".
    pub fn key_for(&self, action: &Action) -> Option<String> {
        self.order
            .iter()
            .find(|chord| self.map.get(chord) == Some(action))
            .map(KeyChord::display)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_to_command_action() {
        let keys = Keymap::with_defaults();
        assert_eq!(
            keys.get(&chord("ctrl+shift+g")),
            Some(&Action::Builtin(BuiltinAction::TextToCommand))
        );
        assert_eq!(BuiltinAction::TextToCommand.name(), "text_to_command");
        assert_eq!(BuiltinAction::TextToCommand.label(), "Text to command (AI)");
    }

    #[test]
    fn session_actions() {
        let keys = Keymap::with_defaults();
        assert_eq!(
            keys.get(&chord("ctrl+shift+s")),
            Some(&Action::Builtin(BuiltinAction::Sessions))
        );
        assert_eq!(
            BuiltinAction::InstallClaudeSkill.name(),
            "install_claude_skill"
        );
        assert_eq!(
            BuiltinAction::InstallClaudeSkill.label(),
            "Install the fterm skill for Claude Code"
        );
        assert_eq!(BuiltinAction::NewScene.name(), "new_scene");
        assert_eq!(
            BuiltinAction::NewScene.label(),
            "New Braille scene (split right)"
        );
        assert_eq!(BuiltinAction::SaveSessionAs.name(), "save_session_as");
        assert_eq!(BuiltinAction::SaveSessionAs.label(), "Save session as…");
        assert_eq!(BuiltinAction::Sessions.label(), "Sessions");
    }

    #[test]
    fn config_keys() {
        let keys = Keymap::with_defaults();
        assert_eq!(
            keys.get(&chord("ctrl+shift+f5")),
            Some(&Action::Builtin(BuiltinAction::ReloadConfig))
        );
        assert_eq!(
            keys.get(&chord("ctrl+shift+,")),
            Some(&Action::Builtin(BuiltinAction::OpenConfig))
        );
    }

    #[test]
    fn ai_context_actions() {
        let keys = Keymap::with_defaults();
        assert_eq!(
            keys.get(&chord("ctrl+shift+x")),
            Some(&Action::Builtin(BuiltinAction::ExplainError))
        );
        assert_eq!(
            BuiltinAction::ExplainError.label(),
            "Explain the last error (AI)"
        );
        assert_eq!(BuiltinAction::AskAiSelection.name(), "ask_ai_selection");
        assert_eq!(
            BuiltinAction::AskAiSelection.label(),
            "Ask AI about the selection"
        );
    }

    #[test]
    fn ai_actions_and_keys() {
        let keys = Keymap::with_defaults();
        assert_eq!(
            keys.get(&chord("ctrl+shift+i")),
            Some(&Action::Builtin(BuiltinAction::PanelAi))
        );
        assert_eq!(BuiltinAction::SetAiKey.name(), "set_ai_key");
        assert_eq!(BuiltinAction::SetAiKey.label(), "Set the AI key");
    }

    #[test]
    fn remote_control_action() {
        let action = BuiltinAction::from_name("toggle_remote_control").unwrap();
        assert_eq!(action.label(), "Remote control on or off for this pane");
    }

    #[test]
    fn history_actions_and_keys() {
        let keys = Keymap::with_defaults();
        assert_eq!(
            keys.get(&chord("alt+f8")),
            Some(&Action::Builtin(BuiltinAction::HistoryCommands))
        );
        assert_eq!(
            keys.get(&chord("alt+f12")),
            Some(&Action::Builtin(BuiltinAction::HistoryDirs))
        );
        assert_eq!(BuiltinAction::HistoryCommands.name(), "history_commands");
        assert_eq!(BuiltinAction::HistoryDirs.label(), "Folder history");
    }

    #[test]
    fn dock_actions_and_keys() {
        for name in ["toggle_dock", "panel_events", "panel_agents", "focus_dock"] {
            let action = BuiltinAction::from_name(name).unwrap();
            assert_eq!(action.name(), name);
        }
        let keys = Keymap::with_defaults();
        assert_eq!(
            keys.get(&chord("ctrl+shift+e")),
            Some(&Action::Builtin(BuiltinAction::PanelEvents))
        );
        assert_eq!(
            keys.get(&chord("ctrl+shift+a")),
            Some(&Action::Builtin(BuiltinAction::PanelAgents))
        );
    }

    #[test]
    fn harmonize_more_and_less() {
        let keys = Keymap::with_defaults();
        assert_eq!(
            keys.get(&chord("ctrl+shift+]")),
            Some(&Action::Builtin(BuiltinAction::HarmonizeMore))
        );
        assert_eq!(
            keys.get(&chord("ctrl+shift+[")),
            Some(&Action::Builtin(BuiltinAction::HarmonizeLess))
        );
        assert_eq!(BuiltinAction::HarmonizeMore.name(), "harmonize_more");
        assert_eq!(
            BuiltinAction::from_name("harmonize_less"),
            Some(BuiltinAction::HarmonizeLess)
        );
        assert!(
            BuiltinAction::HarmonizeMore
                .label()
                .starts_with("Harmonize: more")
        );
        assert!(BuiltinAction::ALL.contains(&BuiltinAction::HarmonizeLess));
    }

    #[test]
    fn original_colors_action() {
        let action = BuiltinAction::from_name("toggle_original_colors").unwrap();
        assert_eq!(action, BuiltinAction::ToggleOriginalColors);
        assert_eq!(action.label(), "Original colors of programs on or off");
        assert!(BuiltinAction::ALL.contains(&action));
    }

    #[test]
    fn install_claude_hooks_action() {
        let action = BuiltinAction::from_name("install_claude_hooks").unwrap();
        assert_eq!(action, BuiltinAction::InstallClaudeHooks);
        assert_eq!(action.label(), "Install Claude Code hooks (agent states)");
        assert!(BuiltinAction::ALL.contains(&action));
    }

    #[test]
    fn choose_theme_action() {
        let action = BuiltinAction::from_name("choose_theme").unwrap();
        assert_eq!(action, BuiltinAction::ChooseTheme);
        assert_eq!(action.label(), "Theme…");
        assert!(BuiltinAction::ALL.contains(&action));
    }

    #[test]
    fn fullscreen_action_and_key() {
        // Like WezTerm: Alt+Enter, and the window has no frame then.
        let action = BuiltinAction::from_name("toggle_fullscreen").unwrap();
        assert_eq!(action, BuiltinAction::ToggleFullscreen);
        assert_eq!(action.label(), "Full screen on or off");
        assert!(BuiltinAction::ALL.contains(&action));
        assert_eq!(
            Keymap::with_defaults().get(&chord("alt+enter")),
            Some(&Action::Builtin(BuiltinAction::ToggleFullscreen))
        );
    }

    #[test]
    fn copy_claude_hooks_action() {
        let action = BuiltinAction::from_name("copy_claude_hooks").unwrap();
        assert_eq!(action.name(), "copy_claude_hooks");
        assert_eq!(action.label(), "Copy Claude Code hooks (settings.json)");
        assert!(BuiltinAction::ALL.contains(&action));
    }

    fn chord(text: &str) -> KeyChord {
        KeyChord::parse(text).unwrap()
    }

    const CTRL_SHIFT: Mods = Mods {
        ctrl: true,
        shift: true,
        alt: false,
        logo: false,
    };

    #[test]
    fn parse_letters_and_mods() {
        assert_eq!(
            chord("ctrl+shift+t"),
            KeyChord {
                mods: CTRL_SHIFT,
                key: Key::Char('t')
            }
        );
        // Case and spaces do not matter, and the order of mods does not matter.
        assert_eq!(chord("Shift + Ctrl + T"), chord("ctrl+shift+t"));
        assert_eq!(chord("control+shift+t"), chord("ctrl+shift+t"));
        assert!(chord("win+e").mods.logo);
    }

    #[test]
    fn parse_named_keys() {
        assert_eq!(chord("ctrl+tab").key, Key::Tab);
        assert_eq!(chord("shift+pageup").key, Key::PageUp);
        assert_eq!(chord("shift+pgdn").key, Key::PageDown);
        assert_eq!(chord("alt+left").key, Key::Left);
        assert_eq!(chord("f11").key, Key::F(11));
        assert_eq!(chord("ctrl+shift+space").key, Key::Space);
        assert_eq!(chord("esc").key, Key::Escape);
    }

    #[test]
    fn parse_symbols_and_digits() {
        assert_eq!(chord("alt+shift+=").key, Key::Char('='));
        assert_eq!(chord("alt+shift+plus").key, Key::Char('='));
        assert_eq!(chord("alt+shift+-").key, Key::Char('-'));
        assert_eq!(chord("alt+shift+minus").key, Key::Char('-'));
        assert_eq!(chord("ctrl+shift+1").key, Key::Char('1'));
        assert_eq!(chord("ctrl+,").key, Key::Char(','));
    }

    #[test]
    fn parse_errors() {
        for bad in ["", "ctrl+shift", "ctrl+foo", "f99", "hyper+t", "ctrl++"] {
            assert!(KeyChord::parse(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn action_names_go_both_ways() {
        for action in BuiltinAction::ALL {
            assert_eq!(
                BuiltinAction::from_name(&action.name()),
                Some(action),
                "{action:?}"
            );
            assert!(!action.label().is_empty());
        }
        assert_eq!(
            BuiltinAction::from_name("select_tab_3"),
            Some(BuiltinAction::SelectTab(2))
        );
        assert_eq!(BuiltinAction::from_name("select_tab_0"), None);
        assert_eq!(BuiltinAction::from_name("fly"), None);
        assert_eq!(BuiltinAction::NewTab.name(), "new_tab");
        assert_eq!(BuiltinAction::SplitRight.label(), "Split right");
    }

    #[test]
    fn defaults_have_the_known_keys() {
        let keys = Keymap::with_defaults();
        let get = |text: &str| keys.get(&chord(text)).cloned();
        assert_eq!(
            get("ctrl+shift+t"),
            Some(Action::Builtin(BuiltinAction::NewTab))
        );
        assert_eq!(
            get("ctrl+shift+w"),
            Some(Action::Builtin(BuiltinAction::ClosePane))
        );
        assert_eq!(
            get("alt+shift+="),
            Some(Action::Builtin(BuiltinAction::SplitRight))
        );
        assert_eq!(
            get("ctrl+shift+3"),
            Some(Action::Builtin(BuiltinAction::SelectTab(2)))
        );
        assert_eq!(
            get("ctrl+shift+p"),
            Some(Action::Builtin(BuiltinAction::CommandPalette))
        );
        assert_eq!(
            get("shift+insert"),
            Some(Action::Builtin(BuiltinAction::Paste))
        );
        // The Windows pair: Ctrl+Ins copies, Shift+Ins pastes.
        assert_eq!(
            get("ctrl+insert"),
            Some(Action::Builtin(BuiltinAction::Copy))
        );
        assert_eq!(get("ctrl+t"), None);
        // Every default key string is valid.
        for (key, action) in DEFAULT_KEYS {
            assert!(KeyChord::parse(key).is_ok(), "{key}");
            assert!(BuiltinAction::from_name(action).is_some(), "{action}");
        }
    }

    #[test]
    fn user_binding_wins_and_none_removes() {
        let mut keys = Keymap::with_defaults();
        let spawn = Action::Spawn {
            profile: Some("Claude".into()),
            place: SpawnWhere::Tab,
        };
        keys.bind(chord("ctrl+shift+t"), Some(spawn.clone()));
        assert_eq!(keys.get(&chord("ctrl+shift+t")), Some(&spawn));
        keys.bind(chord("ctrl+shift+w"), None);
        assert_eq!(keys.get(&chord("ctrl+shift+w")), None);
    }

    #[test]
    fn key_for_shows_a_readable_key() {
        let keys = Keymap::with_defaults();
        assert_eq!(
            keys.key_for(&Action::Builtin(BuiltinAction::NewTab))
                .as_deref(),
            Some("Ctrl+Shift+T")
        );
        assert_eq!(
            keys.key_for(&Action::Builtin(BuiltinAction::SplitRight))
                .as_deref(),
            Some("Alt+Shift+=")
        );
        assert_eq!(keys.key_for(&Action::Lua(7)), None);
    }
}
