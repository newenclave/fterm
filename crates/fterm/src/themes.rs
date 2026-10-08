//! The theme in use: which theme (the config, the system light or dark mode, or a change while fterm runs),
//! and its colors for the terminal palette and the renderer.

use std::path::{Path, PathBuf};

use fterm_config::colors::{ColorConfig, Rgb};
use fterm_config::theme::{Theme, ThemeChoice};
use fterm_render::theme::UiColors;
use fterm_term::alacritty_terminal::vte::ansi::Rgb as TermRgb;
use fterm_term::colors::ColorOverrides;

/// A theme chosen while fterm runs (the palette or the API). It wins over the config until the config changes.
#[derive(Clone, Debug, PartialEq)]
pub enum Override {
    Named(String),
    Inline(Box<Theme>),
}

/// The folders with theme files: `themes` next to the config file, and `themes` in the data folder.
pub fn theme_dirs(config_file: &Path, data_dir: Option<&Path>) -> Vec<PathBuf> {
    asset_dirs(config_file, data_dir, "themes")
}

/// The folders `name` (for example `themes` or `l10n`) next to the config file and in the data folder.
pub fn asset_dirs(config_file: &Path, data_dir: Option<&Path>, name: &str) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = config_file
        .parent()
        .map(|dir| dir.join(name))
        .into_iter()
        .collect();
    if let Some(data) = data_dir {
        let folder = data.join(name);
        if !dirs.contains(&folder) {
            dirs.push(folder);
        }
    }
    dirs
}

/// The theme to use: a theme chosen while fterm runs, else the config. `dark` = the system is in dark mode.
pub fn pick(
    choice: &ThemeChoice,
    over: Option<&Override>,
    dark: bool,
    find: impl Fn(&str) -> Result<Theme, String>,
) -> Result<Theme, String> {
    match over {
        Some(Override::Named(name)) => return find(name),
        Some(Override::Inline(theme)) => return Ok((**theme).clone()),
        None => {}
    }
    match choice {
        ThemeChoice::Default => Ok(Theme::default()),
        ThemeChoice::Named(name) => find(name),
        ThemeChoice::Inline(theme) => Ok((**theme).clone()),
        ThemeChoice::System { light, dark: night } => find(if dark { night } else { light }),
    }
}

fn term_rgb(c: Rgb) -> TermRgb {
    TermRgb {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

/// The rows of the theme list: the theme in use says so.
pub fn theme_rows(names: &[String], current: &str) -> Vec<crate::history_popup::PopupRow> {
    names
        .iter()
        .map(|name| crate::history_popup::PopupRow {
            text: name.clone(),
            hint: if name.eq_ignore_ascii_case(current) {
                fterm_config::tr!("popup.in_use")
            } else {
                String::new()
            },
            key: name.clone(),
            ..Default::default()
        })
        .collect()
}

/// The terminal colors of `theme`, with the `colors` of the config on top.
pub fn palette_colors(theme: &Theme, colors: &ColorConfig) -> ColorOverrides {
    let t = &theme.terminal;
    let pick = |own: Option<Rgb>, theirs: Rgb| Some(term_rgb(own.unwrap_or(theirs)));
    let mut ansi = [None; 8];
    let mut bright = [None; 8];
    for i in 0..8 {
        ansi[i] = pick(colors.ansi[i], t.ansi[i]);
        bright[i] = pick(colors.bright[i], t.bright[i]);
    }
    ColorOverrides {
        background: pick(colors.background, t.background),
        foreground: pick(colors.foreground, t.foreground),
        cursor: pick(colors.cursor, t.cursor),
        selection: pick(colors.selection, t.selection),
        ansi,
        bright,
    }
}

/// How the colors of programs fit the theme: the theme's values, changed by the config.
pub fn harmonize_settings(
    theme: &Theme,
    config: &fterm_config::load::HarmonizeConfig,
) -> fterm_term::harmonize::Settings {
    fterm_term::harmonize::Settings {
        strength: config.strength.unwrap_or(theme.harmonize.strength),
        min_contrast: config.min_contrast.unwrap_or(theme.harmonize.min_contrast),
    }
}

/// The strength one step (0.1) up (`dir` > 0) or down, in 0..1.
pub fn step_strength(current: f32, dir: i32) -> f32 {
    let tenths = current * 10.0;
    // A value between steps goes to the next whole step; a value on a step goes one further.
    let next = if dir > 0 {
        (tenths + 0.001).floor() + 1.0
    } else {
        (tenths - 0.001).ceil() - 1.0
    };
    next.clamp(0.0, 10.0).round() / 10.0
}

/// The UI colors of `theme` for the renderer.
pub fn ui_colors(theme: &Theme) -> UiColors {
    let u = &theme.ui;
    UiColors {
        surface: term_rgb(u.surface),
        surface_active: term_rgb(u.surface_active),
        overlay: term_rgb(u.overlay),
        selected: term_rgb(u.selected),
        accent: term_rgb(u.accent),
        text: term_rgb(u.text),
        text_dim: term_rgb(u.text_dim),
        text_ghost: term_rgb(u.text_ghost),
        scrollbar: term_rgb(u.scrollbar),
        copy_cursor: term_rgb(u.copy_cursor),
        code_bg: term_rgb(u.code_bg),
        input_bg: term_rgb(u.input_bg),
        chip_bg: term_rgb(u.chip_bg),
        info: term_rgb(u.info),
        success: term_rgb(u.success),
        warning: term_rgb(u.warning),
        error: term_rgb(u.error),
        attention: term_rgb(u.attention),
        agent_working: term_rgb(u.agent_working),
        agent_waiting: term_rgb(u.agent_waiting),
        agent_done: term_rgb(u.agent_done),
        agent_error: term_rgb(u.agent_error),
    }
}

#[cfg(test)]
mod tests {
    use fterm_term::alacritty_terminal::term::color::Colors;
    use fterm_term::alacritty_terminal::vte::ansi::NamedColor;
    use fterm_term::colors::Palette;

    use super::*;

    fn named(name: &str) -> Result<Theme, String> {
        Ok(Theme {
            name: name.to_owned(),
            ..Theme::default()
        })
    }

    #[test]
    fn the_default_theme_is_the_look_of_today() {
        // The built-in Mocha JSON and the defaults of the renderer and the terminal must not drift apart.
        let theme = Theme::default();
        assert_eq!(ui_colors(&theme), UiColors::default());
        let from_theme = Palette::with_colors(&palette_colors(&theme, &ColorConfig::default()));
        let today = Palette::default();
        let none = Colors::default();
        for i in 0..16 {
            assert_eq!(from_theme.get(i, &none), today.get(i, &none), "color {i}");
        }
        for named in [
            NamedColor::Foreground,
            NamedColor::Background,
            NamedColor::Cursor,
        ] {
            let i = named as usize;
            assert_eq!(from_theme.get(i, &none), today.get(i, &none), "{named:?}");
        }
        assert_eq!(from_theme.selection, today.selection);
    }

    #[test]
    fn the_ui_colors_follow_the_theme() {
        let mut theme = Theme::default();
        theme.ui.accent = Rgb { r: 1, g: 2, b: 3 };
        theme.ui.agent_error = Rgb { r: 4, g: 5, b: 6 };
        let ui = ui_colors(&theme);
        assert_eq!(ui.accent, TermRgb { r: 1, g: 2, b: 3 });
        assert_eq!(ui.agent_error, TermRgb { r: 4, g: 5, b: 6 });
    }

    #[test]
    fn colors_of_the_config_change_the_theme() {
        let theme = Theme::builtin("Catppuccin Latte").unwrap();
        let colors = ColorConfig {
            background: Some(Rgb { r: 9, g: 9, b: 9 }),
            ..ColorConfig::default()
        };
        let out = palette_colors(&theme, &colors);
        assert_eq!(out.background, Some(TermRgb { r: 9, g: 9, b: 9 }));
        // The rest is the theme.
        assert_eq!(out.foreground, Some(term_rgb(theme.terminal.foreground)));
        assert_eq!(out.ansi[1], Some(term_rgb(theme.terminal.ansi[1])));
        assert_eq!(out.bright[7], Some(term_rgb(theme.terminal.bright[7])));
    }

    #[test]
    fn which_theme() {
        let name = |choice: &ThemeChoice, over: Option<&Override>, dark: bool| {
            pick(choice, over, dark, named).unwrap().name
        };
        assert_eq!(name(&ThemeChoice::Default, None, true), "Catppuccin Mocha");
        assert_eq!(name(&ThemeChoice::Named("Nord".into()), None, true), "Nord");
        let system = ThemeChoice::System {
            light: "Day".into(),
            dark: "Night".into(),
        };
        assert_eq!(name(&system, None, true), "Night");
        assert_eq!(name(&system, None, false), "Day");
        let inline = Theme {
            name: "Inline".into(),
            ..Theme::default()
        };
        assert_eq!(
            name(&ThemeChoice::Inline(Box::new(inline.clone())), None, true),
            "Inline"
        );
        // A theme chosen while fterm runs wins.
        let over = Override::Named("Picked".into());
        assert_eq!(name(&system, Some(&over), true), "Picked");
        let over = Override::Inline(Box::new(inline));
        assert_eq!(name(&ThemeChoice::Default, Some(&over), true), "Inline");
        // An error from finding the theme comes back.
        let err = pick(&ThemeChoice::Named("x".into()), None, true, |_| {
            Err("no theme `x`".into())
        });
        assert_eq!(err.unwrap_err(), "no theme `x`");
    }

    #[test]
    fn the_rows_of_the_theme_list() {
        let names = vec!["Catppuccin Mocha".to_owned(), "Nord".to_owned()];
        let rows = theme_rows(&names, "nord");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text, "Catppuccin Mocha");
        assert_eq!(rows[0].hint, "");
        assert_eq!(rows[1].hint, "in use", "case does not matter");
        assert_eq!(rows[1].key, "Nord");
    }

    #[test]
    fn the_harmonize_of_the_theme_and_the_config() {
        use fterm_config::load::HarmonizeConfig;
        let mut theme = Theme::default();
        let off = harmonize_settings(&theme, &HarmonizeConfig::default());
        assert_eq!(
            (off.strength, off.min_contrast),
            (0.0, 3.0),
            "off by default"
        );
        theme.harmonize.strength = 0.8;
        let on = harmonize_settings(&theme, &HarmonizeConfig::default());
        assert_eq!(on.strength, 0.8, "the theme says");
        let config = HarmonizeConfig {
            strength: Some(0.3),
            min_contrast: Some(4.5),
        };
        let mine = harmonize_settings(&theme, &config);
        assert_eq!(
            (mine.strength, mine.min_contrast),
            (0.3, 4.5),
            "the config wins"
        );
    }

    #[test]
    fn a_strength_goes_up_and_down_in_steps() {
        assert_eq!(step_strength(0.0, 1), 0.1);
        assert_eq!(step_strength(0.8, 1), 0.9);
        assert_eq!(step_strength(1.0, 1), 1.0, "at most 1");
        assert_eq!(step_strength(0.1, -1), 0.0);
        assert_eq!(step_strength(0.0, -1), 0.0, "at least 0");
        // A value between steps goes to the next whole step.
        assert_eq!(step_strength(0.65, 1), 0.7);
        assert_eq!(step_strength(0.65, -1), 0.6);
    }

    #[test]
    fn the_theme_folders() {
        let config = Path::new("cfg").join("fterm.lua");
        assert_eq!(
            theme_dirs(&config, Some(Path::new("data"))),
            [
                Path::new("cfg").join("themes"),
                Path::new("data").join("themes")
            ]
        );
        assert_eq!(theme_dirs(&config, None), [Path::new("cfg").join("themes")]);
    }
}
