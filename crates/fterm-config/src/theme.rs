//! Themes: the colors of the terminal and of the whole UI (tab bar, dock, toasts, boxes, frames).
//!
//! A theme is JSON (a file in a `themes` folder, a string, or a Lua table with the same keys).
//! The UI colors are roles (`accent`, `surface`, `text_dim`, ...). A role that the theme does not give
//! is made from the terminal colors, so a theme with only the 16 colors colors the whole UI.
//! A Windows Terminal color scheme (`black` ... `brightWhite`) is a theme too.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::colors::{Rgb, parse_color};

/// The colors of the terminal grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalColors {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    /// Black, red, green, yellow, blue, magenta, cyan, white.
    pub ansi: [Rgb; 8],
    pub bright: [Rgb; 8],
}

macro_rules! ui_roles {
    ($($name:ident),* $(,)?) => {
        /// The colors of the UI, by role.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct UiColors {
            $(pub $name: Rgb,)*
        }

        impl UiColors {
            /// The names of the roles, as in the JSON.
            pub const KEYS: &'static [&'static str] = &[$(stringify!($name)),*];

            /// The role with this name.
            pub fn slot(&mut self, key: &str) -> Option<&mut Rgb> {
                match key {
                    $(stringify!($name) => Some(&mut self.$name),)*
                    _ => None,
                }
            }
        }
    };
}

ui_roles!(
    surface,
    surface_active,
    overlay,
    selected,
    accent,
    text,
    text_dim,
    text_ghost,
    scrollbar,
    copy_cursor,
    code_bg,
    input_bg,
    chip_bg,
    info,
    success,
    warning,
    error,
    attention,
    agent_working,
    agent_waiting,
    agent_done,
    agent_error,
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: String,
    pub terminal: TerminalColors,
    pub ui: UiColors,
}

/// The themes in fterm itself: (file name, JSON).
const BUILTIN: &[(&str, &str)] = &[
    (
        "catppuccin-mocha",
        include_str!("../../../assets/themes/catppuccin-mocha.json"),
    ),
    (
        "catppuccin-latte",
        include_str!("../../../assets/themes/catppuccin-latte.json"),
    ),
];

/// The name of the default theme.
pub const DEFAULT_THEME: &str = "Catppuccin Mocha";

impl Default for Theme {
    fn default() -> Self {
        Self::builtin(DEFAULT_THEME).expect("the default theme is built in")
    }
}

impl Theme {
    /// A theme from JSON: the fterm format or a Windows Terminal scheme.
    pub fn from_json(value: &Value) -> Result<Theme, String> {
        let object = value.as_object().ok_or("a theme must be a JSON object")?;
        if WT_KEYS.iter().any(|(key, _)| object.contains_key(*key)) {
            return from_windows_terminal(object);
        }
        let mut name = String::new();
        let mut terminal = default_terminal();
        let mut ui_value = None;
        for (key, value) in object {
            match key.as_str() {
                "name" => {
                    name = value.as_str().ok_or("name: expected a string")?.to_owned();
                }
                "terminal" => terminal = terminal_colors(value, terminal)?,
                "ui" => ui_value = Some(value),
                other => {
                    return Err(format!(
                        "`{other}` is not a theme key: use name, terminal, ui"
                    ));
                }
            }
        }
        let mut ui = derive_ui(&terminal);
        if let Some(value) = ui_value {
            let given = value.as_object().ok_or("ui: expected an object")?;
            // `attention` follows the accent, unless the theme gives it.
            if given.contains_key("accent") && !given.contains_key("attention") {
                ui.attention = color_at(&given["accent"], "ui.accent")?;
            }
            for (key, value) in given {
                let path = format!("ui.{key}");
                let slot = ui.slot(key).ok_or_else(|| {
                    format!(
                        "`{path}` is not a UI role: use one of {}",
                        UiColors::KEYS.join(", ")
                    )
                })?;
                *slot = color_at(value, &path)?;
            }
        }
        Ok(Theme { name, terminal, ui })
    }

    /// A built-in theme by its name or its file name (case does not matter).
    pub fn builtin(name: &str) -> Option<Theme> {
        let wanted = simple(name);
        BUILTIN.iter().find_map(|(file, text)| {
            let theme = builtin_theme(text);
            (simple(file) == wanted || simple(&theme.name) == wanted).then_some(theme)
        })
    }
}

fn builtin_theme(text: &str) -> Theme {
    let value: Value = serde_json::from_str(text).expect("a built-in theme is JSON");
    Theme::from_json(&value).expect("a built-in theme is good")
}

/// A name to compare: lower case, `-` and `_` are spaces.
fn simple(name: &str) -> String {
    name.trim().to_lowercase().replace(['-', '_'], " ")
}

/// The names of the built-in themes.
pub fn builtin_names() -> Vec<String> {
    BUILTIN
        .iter()
        .map(|(_, text)| builtin_theme(text).name)
        .collect()
}

/// The terminal colors of the default theme. Kept here, not from the JSON, so a theme with no terminal
/// part does not need to read the built-in file.
fn default_terminal() -> TerminalColors {
    let hex = |v: u32| Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    };
    let ansi = [
        0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
    ];
    let bright = [
        0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
    ];
    TerminalColors {
        background: hex(0x1e1e2e),
        foreground: hex(0xcdd6f4),
        cursor: hex(0xf5e0dc),
        selection: hex(0x585b70),
        ansi: ansi.map(hex),
        bright: bright.map(hex),
    }
}

fn color_at(value: &Value, path: &str) -> Result<Rgb, String> {
    let text = value
        .as_str()
        .ok_or_else(|| format!("{path}: expected a color like \"#1e1e2e\""))?;
    parse_color(text).map_err(|err| format!("{path}: {err}"))
}

fn colors_8(value: &Value, path: &str) -> Result<[Rgb; 8], String> {
    let list = value
        .as_array()
        .filter(|l| l.len() == 8)
        .ok_or_else(|| {
            format!("{path}: expected a list of 8 colors (black, red, green, yellow, blue, magenta, cyan, white)")
        })?;
    let mut out = [Rgb { r: 0, g: 0, b: 0 }; 8];
    for (i, item) in list.iter().enumerate() {
        out[i] = color_at(item, &format!("{path}[{}]", i + 1))?;
    }
    Ok(out)
}

/// The `terminal` part: given colors on top of `base`.
fn terminal_colors(value: &Value, base: TerminalColors) -> Result<TerminalColors, String> {
    let object = value.as_object().ok_or("terminal: expected an object")?;
    let mut t = base;
    for (key, value) in object {
        let path = format!("terminal.{key}");
        match key.as_str() {
            "background" => t.background = color_at(value, &path)?,
            "foreground" => t.foreground = color_at(value, &path)?,
            "cursor" => t.cursor = color_at(value, &path)?,
            "selection" => t.selection = color_at(value, &path)?,
            "ansi" => t.ansi = colors_8(value, &path)?,
            "bright" => t.bright = colors_8(value, &path)?,
            _ => {
                return Err(format!(
                    "`{path}` is not known: use background, foreground, cursor, selection, ansi, bright"
                ));
            }
        }
    }
    Ok(t)
}

/// The color keys of a Windows Terminal scheme: (key, index in ansi + bright).
const WT_KEYS: &[(&str, usize)] = &[
    ("black", 0),
    ("red", 1),
    ("green", 2),
    ("yellow", 3),
    ("blue", 4),
    ("purple", 5),
    ("cyan", 6),
    ("white", 7),
    ("brightBlack", 8),
    ("brightRed", 9),
    ("brightGreen", 10),
    ("brightYellow", 11),
    ("brightBlue", 12),
    ("brightPurple", 13),
    ("brightCyan", 14),
    ("brightWhite", 15),
];

fn from_windows_terminal(object: &serde_json::Map<String, Value>) -> Result<Theme, String> {
    let mut t = default_terminal();
    let get = |key: &str| object.get(key).map(|v| color_at(v, key)).transpose();
    for (key, index) in WT_KEYS {
        if let Some(color) = get(key)? {
            if *index < 8 {
                t.ansi[*index] = color;
            } else {
                t.bright[index - 8] = color;
            }
        }
    }
    if let Some(c) = get("background")? {
        t.background = c;
    }
    if let Some(c) = get("foreground")? {
        t.foreground = c;
    }
    t.cursor = get("cursorColor")?.unwrap_or(t.foreground);
    t.selection =
        get("selectionBackground")?.unwrap_or_else(|| mix(t.background, t.foreground, 0.3));
    // Other keys of Windows Terminal schemes are not colors of fterm; they are skipped.
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(Theme {
        name,
        terminal: t,
        ui: derive_ui(&t),
    })
}

/// The UI roles made from the terminal colors.
pub fn derive_ui(t: &TerminalColors) -> UiColors {
    let (bg, fg) = (t.background, t.foreground);
    let black = Rgb { r: 0, g: 0, b: 0 };
    let [_, red, green, yellow, blue, magenta, _, _] = t.ansi;
    UiColors {
        surface: mix(bg, black, 0.2),
        surface_active: bg,
        overlay: mix(bg, fg, 0.12),
        selected: mix(bg, fg, 0.2),
        accent: magenta,
        text: fg,
        text_dim: mix(fg, bg, 0.35),
        text_ghost: mix(fg, bg, 0.55),
        scrollbar: mix(fg, bg, 0.45),
        copy_cursor: yellow,
        code_bg: mix(bg, black, 0.35),
        input_bg: mix(bg, fg, 0.06),
        chip_bg: mix(bg, fg, 0.16),
        info: blue,
        success: green,
        warning: yellow,
        error: red,
        attention: magenta,
        agent_working: blue,
        agent_waiting: yellow,
        agent_done: green,
        agent_error: red,
    }
}

/// `a` mixed with `b`: `t` = 0 is `a`, 1 is `b`.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let one = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Rgb {
        r: one(a.r, b.r),
        g: one(a.g, b.g),
        b: one(a.b, b.b),
    }
}

/// The `.json` files in `dirs`, sorted by name.
fn theme_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            })
            .collect();
        files.sort();
        out.extend(files);
    }
    out
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Finds a theme: a built-in one, or a `.json` file in `dirs` (by the file name or its `name`).
pub fn find_theme(name: &str, dirs: &[PathBuf]) -> Result<Theme, String> {
    if let Some(theme) = Theme::builtin(name) {
        return Ok(theme);
    }
    let wanted = simple(name);
    let files = theme_files(dirs);
    // The file name first: it needs no reading of other files.
    if let Some(path) = files.iter().find(|p| simple(&file_stem(p)) == wanted) {
        return load_file(path);
    }
    for path in &files {
        if let Ok(theme) = load_file(path)
            && simple(&theme.name) == wanted
        {
            return Ok(theme);
        }
    }
    Err(format!(
        "no theme `{name}`; the themes are: {}",
        list_themes(dirs).join(", ")
    ))
}

/// The names of all themes: the built-in ones, then the good files in `dirs`.
pub fn list_themes(dirs: &[PathBuf]) -> Vec<String> {
    let mut names = builtin_names();
    for path in theme_files(dirs) {
        if let Ok(theme) = load_file(&path)
            && !names.iter().any(|n| simple(n) == simple(&theme.name))
        {
            names.push(theme.name);
        }
    }
    names
}

/// A theme from a file. A theme with no `name` gets the file name.
pub fn load_file(path: &Path) -> Result<Theme, String> {
    let where_ = path.display();
    let text = std::fs::read_to_string(path).map_err(|err| format!("{where_}: {err}"))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|err| format!("{where_}: bad JSON: {err}"))?;
    let mut theme = Theme::from_json(&value).map_err(|err| format!("{where_}: {err}"))?;
    if theme.name.is_empty() {
        theme.name = file_stem(path);
    }
    Ok(theme)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rgb(hex: &str) -> Rgb {
        parse_color(hex).unwrap()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fterm-theme-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_default_is_mocha_with_the_colors_of_today() {
        let t = Theme::default();
        assert_eq!(t.name, "Catppuccin Mocha");
        assert_eq!(t.terminal.background, rgb("#1e1e2e"));
        assert_eq!(t.terminal.ansi[1], rgb("#f38ba8"));
        assert_eq!(t.terminal.bright[7], rgb("#a6adc8"));
        assert_eq!(t.ui.surface, rgb("#181825"));
        assert_eq!(t.ui.accent, rgb("#cba6f7"));
        assert_eq!(t.ui.text_dim, rgb("#9399b2"));
        assert_eq!(t.ui.agent_error, rgb("#f04a4a"));
    }

    #[test]
    fn the_built_in_themes_give_every_role() {
        for (file, text) in BUILTIN {
            let value: Value = serde_json::from_str(text).unwrap();
            let ui = value["ui"].as_object().unwrap();
            for key in UiColors::KEYS {
                assert!(ui.contains_key(*key), "{file}: no ui.{key}");
            }
        }
        assert_eq!(builtin_names(), ["Catppuccin Mocha", "Catppuccin Latte"]);
        let latte = Theme::builtin("catppuccin latte").unwrap();
        assert_eq!(latte.terminal.background, rgb("#eff1f5"));
        assert_eq!(
            Theme::builtin("Catppuccin-Latte"),
            Some(latte),
            "the file name works too"
        );
        assert_eq!(Theme::builtin("nope"), None);
    }

    #[test]
    fn missing_roles_come_from_the_terminal_colors() {
        let ansi = [
            "#000000", "#cc0000", "#00cc00", "#cccc00", "#0000cc", "#cc00cc", "#00cccc", "#cccccc",
        ];
        let t = Theme::from_json(&json!({
            "name": "Mine",
            "terminal": { "background": "#101010", "foreground": "#f0f0f0", "ansi": ansi, "bright": ansi },
        }))
        .unwrap();
        assert_eq!(t.name, "Mine");
        let ui = t.ui;
        assert_eq!(ui.surface_active, rgb("#101010"));
        assert_eq!(ui.text, rgb("#f0f0f0"));
        assert_eq!(ui.accent, rgb("#cc00cc"), "magenta");
        assert_eq!(ui.attention, ui.accent);
        assert_eq!(ui.info, rgb("#0000cc"));
        assert_eq!(ui.success, rgb("#00cc00"));
        assert_eq!(ui.warning, rgb("#cccc00"));
        assert_eq!(ui.error, rgb("#cc0000"));
        assert_eq!(ui.agent_waiting, rgb("#cccc00"));
        assert_eq!(ui.copy_cursor, rgb("#cccc00"));
        // Panels are a bit darker than the background; boxes and rows a bit lighter (toward the text).
        assert_eq!(ui.surface, mix(rgb("#101010"), rgb("#000000"), 0.2));
        assert_eq!(ui.overlay, mix(rgb("#101010"), rgb("#f0f0f0"), 0.12));
        assert!(ui.selected.r > ui.overlay.r && ui.overlay.r > ui.surface_active.r);
        assert!(ui.text_dim.r < ui.text.r && ui.text_ghost.r < ui.text_dim.r);
    }

    #[test]
    fn a_role_from_the_theme_wins_and_the_rest_is_made() {
        let t = Theme::from_json(&json!({ "ui": { "accent": "#123456" } })).unwrap();
        assert_eq!(t.ui.accent, rgb("#123456"));
        // No terminal colors: they are the default ones.
        assert_eq!(t.terminal, Theme::default().terminal);
        assert_eq!(t.ui.text, rgb("#cdd6f4"));
        assert_eq!(
            t.ui.attention,
            rgb("#123456"),
            "attention follows the accent"
        );
        assert_eq!(t.name, "", "no name in the JSON");
    }

    #[test]
    fn mix_goes_from_a_to_b() {
        let (a, b) = (rgb("#000000"), rgb("#ffffff"));
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, 0.5), rgb("#808080"));
    }

    #[test]
    fn a_windows_terminal_scheme_is_a_theme() {
        let t = Theme::from_json(&json!({
            "name": "Nord",
            "background": "#2E3440", "foreground": "#D8DEE9",
            "cursorColor": "#ECEFF4", "selectionBackground": "#434C5E",
            "black": "#3B4252", "red": "#BF616A", "green": "#A3BE8C", "yellow": "#EBCB8B",
            "blue": "#81A1C1", "purple": "#B48EAD", "cyan": "#88C0D0", "white": "#E5E9F0",
            "brightBlack": "#4C566A", "brightRed": "#BF616A", "brightGreen": "#A3BE8C",
            "brightYellow": "#EBCB8B", "brightBlue": "#81A1C1", "brightPurple": "#B48EAD",
            "brightCyan": "#8FBCBB", "brightWhite": "#ECEFF4"
        }))
        .unwrap();
        assert_eq!(t.name, "Nord");
        assert_eq!(t.terminal.background, rgb("#2e3440"));
        assert_eq!(t.terminal.cursor, rgb("#eceff4"));
        assert_eq!(t.terminal.selection, rgb("#434c5e"));
        assert_eq!(t.terminal.ansi[5], rgb("#b48ead"));
        assert_eq!(t.terminal.bright[6], rgb("#8fbcbb"));
        assert_eq!(
            t.ui.accent,
            rgb("#b48ead"),
            "the UI is made from the scheme"
        );
        // No cursor color: the text color.
        let t = Theme::from_json(
            &json!({ "background": "#000000", "foreground": "#eeeeee", "black": "#000000" }),
        )
        .unwrap();
        assert_eq!(t.terminal.cursor, rgb("#eeeeee"));
    }

    #[test]
    fn errors_say_where() {
        let err = |v: Value| Theme::from_json(&v).unwrap_err();
        assert!(err(json!({ "ui": { "accent": "blue" } })).contains("ui.accent"));
        let typo = err(json!({ "ui": { "acent": "#123456" } }));
        assert!(
            typo.contains("ui.acent") && typo.contains("accent"),
            "{typo}"
        );
        assert!(err(json!({ "terminal": { "ansi": ["#000000"] } })).contains("terminal.ansi"));
        assert!(err(json!({ "terminal": { "background": 5 } })).contains("terminal.background"));
        assert!(err(json!({ "colours": {} })).contains("colours"));
        assert!(err(json!([1, 2])).contains("object"));
    }

    #[test]
    fn themes_from_files() {
        let dir = temp_dir("files");
        std::fs::write(
            dir.join("nord.json"),
            r##"{ "name": "Nord", "background": "#2e3440", "foreground": "#d8dee9", "black": "#3b4252" }"##,
        )
        .unwrap();
        std::fs::write(
            dir.join("my-dark.json"),
            r##"{ "name": "My Dark", "ui": { "accent": "#ff0000" } }"##,
        )
        .unwrap();
        std::fs::write(dir.join("broken.json"), "{ nope").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a theme").unwrap();
        let dirs = vec![dir.clone(), dir.join("missing")];

        assert_eq!(
            find_theme("nord", &dirs).unwrap().name,
            "Nord",
            "by the file name"
        );
        assert_eq!(
            find_theme("My Dark", &dirs).unwrap().ui.accent,
            rgb("#ff0000"),
            "by its name"
        );
        assert_eq!(
            find_theme("catppuccin latte", &dirs).unwrap().name,
            "Catppuccin Latte"
        );
        let broken = find_theme("broken", &dirs).unwrap_err();
        assert!(broken.contains("broken.json"), "{broken}");
        let missing = find_theme("Solarized", &dirs).unwrap_err();
        assert!(
            missing.contains("Solarized") && missing.contains("Nord"),
            "{missing}"
        );

        let names = list_themes(&dirs);
        assert_eq!(names[..2], ["Catppuccin Mocha", "Catppuccin Latte"]);
        assert!(names.contains(&"Nord".to_owned()) && names.contains(&"My Dark".to_owned()));
        assert!(
            !names
                .iter()
                .any(|n| n.contains("broken") || n.contains("notes"))
        );

        // A file with no `name`: its file name.
        std::fs::write(dir.join("plain.json"), r##"{ "ui": {} }"##).unwrap();
        assert_eq!(load_file(&dir.join("plain.json")).unwrap().name, "plain");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
