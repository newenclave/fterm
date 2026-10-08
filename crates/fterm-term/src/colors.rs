//! Colors: the palette and the rules that turn a cell into real fg/bg colors.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::{COUNT, Colors};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

/// Full color table with the same slots as alacritty:
/// 0..16 named, 16..256 indexed, then foreground, background, cursor, dim colors, bright/dim foreground.
#[derive(Clone, Debug)]
pub struct Palette {
    colors: [Rgb; COUNT],
    /// Background of selected cells.
    pub selection: Rgb,
    /// Fits the colors that programs choose to the theme (`None` = they stay as they are).
    harmonizer: Option<crate::harmonize::Harmonizer>,
}

/// Colors from the user config. `None` = keep the built-in color.
#[derive(Clone, Copy, Debug, Default)]
pub struct ColorOverrides {
    pub background: Option<Rgb>,
    pub foreground: Option<Rgb>,
    pub cursor: Option<Rgb>,
    pub selection: Option<Rgb>,
    pub ansi: [Option<Rgb>; 8],
    pub bright: [Option<Rgb>; 8],
}

/// Catppuccin Mocha: 8 normal and 8 bright colors.
const ANSI: [u32; 16] = [
    0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, //
    0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
];
const FOREGROUND: u32 = 0xcdd6f4;
const BACKGROUND: u32 = 0x1e1e2e;
const CURSOR: u32 = 0xf5e0dc;
const SELECTION: u32 = 0x585b70;

/// How much darker a DIM color is.
const DIM_FACTOR: f32 = 0.66;

fn hex(v: u32) -> Rgb {
    Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    }
}

fn dim(c: Rgb) -> Rgb {
    c * DIM_FACTOR
}

impl Default for Palette {
    fn default() -> Self {
        let mut colors = [Rgb::default(); COUNT];
        for (slot, v) in colors.iter_mut().zip(ANSI) {
            *slot = hex(v);
        }
        // 6x6x6 color cube.
        let step = |i: usize| if i == 0 { 0 } else { (55 + i * 40) as u8 };
        for i in 0..216 {
            colors[16 + i] = Rgb {
                r: step(i / 36),
                g: step(i / 6 % 6),
                b: step(i % 6),
            };
        }
        // 24 grays.
        for i in 0..24 {
            let v = (8 + i * 10) as u8;
            colors[232 + i] = Rgb { r: v, g: v, b: v };
        }
        colors[NamedColor::Foreground as usize] = hex(FOREGROUND);
        colors[NamedColor::Background as usize] = hex(BACKGROUND);
        colors[NamedColor::Cursor as usize] = hex(CURSOR);
        for i in 0..8 {
            colors[NamedColor::DimBlack as usize + i] = dim(colors[i]);
        }
        colors[NamedColor::BrightForeground as usize] = hex(FOREGROUND);
        colors[NamedColor::DimForeground as usize] = dim(hex(FOREGROUND));
        Self {
            colors,
            selection: hex(SELECTION),
            harmonizer: None,
        }
    }
}

impl Palette {
    /// The built-in palette with the user's colors on top. Dim colors follow their base colors.
    pub fn with_colors(colors: &ColorOverrides) -> Self {
        let mut palette = Self::default();
        for (i, color) in colors.ansi.iter().enumerate() {
            if let Some(c) = color {
                palette.colors[i] = *c;
                palette.colors[NamedColor::DimBlack as usize + i] = dim(*c);
            }
        }
        for (i, color) in colors.bright.iter().enumerate() {
            if let Some(c) = color {
                palette.colors[8 + i] = *c;
            }
        }
        if let Some(fg) = colors.foreground {
            palette.colors[NamedColor::Foreground as usize] = fg;
            palette.colors[NamedColor::BrightForeground as usize] = fg;
            palette.colors[NamedColor::DimForeground as usize] = dim(fg);
        }
        if let Some(bg) = colors.background {
            palette.colors[NamedColor::Background as usize] = bg;
        }
        if let Some(cursor) = colors.cursor {
            palette.colors[NamedColor::Cursor as usize] = cursor;
        }
        if let Some(selection) = colors.selection {
            palette.selection = selection;
        }
        palette
    }

    /// Returns the color in `index`. A color set by the app (OSC 4/10/11) wins.
    /// Fits the colors that programs choose to this palette. Strength 0 = they stay as they are.
    pub fn set_harmonize(&mut self, settings: crate::harmonize::Settings) {
        let mut sixteen = [Rgb::default(); 16];
        sixteen.copy_from_slice(&self.colors[..16]);
        let background = self.colors[NamedColor::Background as usize];
        self.harmonizer = crate::harmonize::Harmonizer::new(settings, background, &sixteen);
    }

    pub fn get(&self, index: usize, overrides: &Colors) -> Rgb {
        overrides[index].unwrap_or(self.colors[index])
    }

    fn fg(&self, color: Color, flags: Flags, overrides: &Colors) -> Rgb {
        let bold = flags.contains(Flags::BOLD);
        let dimmed = flags.contains(Flags::DIM) && !bold;
        match color {
            Color::Spec(rgb) if dimmed => dim(rgb),
            Color::Spec(rgb) => rgb,
            Color::Named(named) => {
                let named = if bold && (named as usize) < 8 {
                    named.to_bright()
                } else if dimmed {
                    named.to_dim()
                } else {
                    named
                };
                self.get(named as usize, overrides)
            }
            Color::Indexed(i) => {
                let i = if bold && i < 8 { i + 8 } else { i };
                let rgb = self.get(i as usize, overrides);
                if dimmed { dim(rgb) } else { rgb }
            }
        }
    }

    fn bg(&self, color: Color, overrides: &Colors) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Named(named) => self.get(named as usize, overrides),
            Color::Indexed(i) => self.get(i as usize, overrides),
        }
    }
}

/// Final colors of one cell; colors that programs choose fit the theme (when the palette says so).
/// See `cell_colors_with`.
pub fn cell_colors(
    fg: Color,
    bg: Color,
    flags: Flags,
    palette: &Palette,
    overrides: &Colors,
) -> (Rgb, Rgb) {
    cell_colors_with(fg, bg, flags, palette, overrides, true)
}

/// Final colors of one cell, after bold, dim, inverse, and hidden. With `harmonize`, the colors that the
/// program chose (truecolor and the 256-color cube) fit the theme, and text keeps a minimum contrast.
pub fn cell_colors_with(
    fg: Color,
    bg: Color,
    flags: Flags,
    palette: &Palette,
    overrides: &Colors,
    harmonize: bool,
) -> (Rgb, Rgb) {
    let harmonizer = palette.harmonizer.as_ref().filter(|_| harmonize);
    let Some(h) = harmonizer else {
        return cell_colors_plain(fg, bg, flags, palette, overrides);
    };
    // Only the colors that the program chose: the first 16 are the theme already.
    // The colors that the program chose: its own (truecolor, the 256-color cube), or palette colors that
    // it changed (OSC 4, 10, 11; Far Manager sets the old console colors). The theme's own stay.
    let changed = |i: usize| overrides[i].is_some();
    let bright = |i: usize| {
        if flags.contains(Flags::BOLD) && i < 8 {
            i + 8
        } else {
            i
        }
    };
    let foreign_fg = match fg {
        Color::Spec(_) => true,
        Color::Indexed(i) => i >= 16 || changed(bright(i as usize)),
        Color::Named(n) => changed(bright(n as usize)),
    };
    let foreign_bg = match bg {
        Color::Spec(_) => true,
        Color::Indexed(i) => i >= 16 || changed(i as usize),
        Color::Named(n) => changed(n as usize),
    };
    let mut fg_rgb = palette.fg(fg, flags, overrides);
    let mut bg_rgb = palette.bg(bg, overrides);
    if foreign_fg {
        fg_rgb = h.color(fg_rgb);
    }
    if foreign_bg {
        bg_rgb = h.color(bg_rgb);
    }
    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg_rgb, &mut bg_rgb);
    }
    if flags.contains(Flags::HIDDEN) {
        return (bg_rgb, bg_rgb);
    }
    (h.readable(fg_rgb, bg_rgb), bg_rgb)
}

fn cell_colors_plain(
    fg: Color,
    bg: Color,
    flags: Flags,
    palette: &Palette,
    overrides: &Colors,
) -> (Rgb, Rgb) {
    let mut fg = palette.fg(fg, flags, overrides);
    let mut bg = palette.bg(bg, overrides);
    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if flags.contains(Flags::HIDDEN) {
        fg = bg;
    }
    (fg, bg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    fn colors(fg: Color, bg: Color, flags: Flags) -> (Rgb, Rgb) {
        cell_colors(fg, bg, flags, &Palette::default(), &Colors::default())
    }

    const FG: Color = Color::Named(NamedColor::Foreground);
    const BG: Color = Color::Named(NamedColor::Background);

    #[test]
    fn cube_starts_at_16_and_ends_at_231() {
        let p = Palette::default();
        let o = Colors::default();
        assert_eq!(p.get(16, &o), rgb(0, 0, 0));
        assert_eq!(p.get(231, &o), rgb(255, 255, 255));
        // Index 196 is pure red in the 6x6x6 cube.
        assert_eq!(p.get(196, &o), rgb(255, 0, 0));
        // The cube steps are 0, 95, 135, 175, 215, 255.
        assert_eq!(p.get(16 + 36 + 6 + 1, &o), rgb(95, 95, 95));
    }

    #[test]
    fn grays_are_232_to_255() {
        let p = Palette::default();
        let o = Colors::default();
        assert_eq!(p.get(232, &o), rgb(8, 8, 8));
        assert_eq!(p.get(255, &o), rgb(238, 238, 238));
    }

    #[test]
    fn truecolor_is_kept() {
        let (fg, bg) = colors(
            Color::Spec(rgb(1, 2, 3)),
            Color::Spec(rgb(4, 5, 6)),
            Flags::empty(),
        );
        assert_eq!(fg, rgb(1, 2, 3));
        assert_eq!(bg, rgb(4, 5, 6));
    }

    #[test]
    fn indexed_color_uses_the_table() {
        let (fg, _) = colors(Color::Indexed(196), BG, Flags::empty());
        assert_eq!(fg, rgb(255, 0, 0));
    }

    #[test]
    fn inverse_swaps_fg_and_bg() {
        let p = Palette::default();
        let o = Colors::default();
        let (fg, bg) = colors(FG, BG, Flags::INVERSE);
        assert_eq!(fg, p.get(NamedColor::Background as usize, &o));
        assert_eq!(bg, p.get(NamedColor::Foreground as usize, &o));
    }

    #[test]
    fn hidden_text_uses_the_bg_color() {
        let (fg, bg) = colors(FG, BG, Flags::HIDDEN);
        assert_eq!(fg, bg);
    }

    #[test]
    fn bold_named_color_becomes_bright() {
        let p = Palette::default();
        let o = Colors::default();
        let (fg, _) = colors(Color::Named(NamedColor::Red), BG, Flags::BOLD);
        assert_eq!(fg, p.get(NamedColor::BrightRed as usize, &o));
        let (fg, _) = colors(Color::Indexed(1), BG, Flags::BOLD);
        assert_eq!(fg, p.get(NamedColor::BrightRed as usize, &o));
    }

    #[test]
    fn program_colors_fit_the_theme_when_asked() {
        use crate::harmonize::{Settings, contrast};
        let o = Colors::default();
        let red = Color::Spec(rgb(255, 0, 0));
        let mut p = Palette::default();
        let (plain, _) = cell_colors(red, BG, Flags::empty(), &p, &o);
        assert_eq!(plain, rgb(255, 0, 0), "no harmonizer: as it is");
        p.set_harmonize(Settings {
            strength: 1.0,
            min_contrast: 3.0,
        });
        let (fit, _) = cell_colors(red, BG, Flags::empty(), &p, &o);
        assert_ne!(fit, rgb(255, 0, 0));
        assert!(fit.r > fit.g && fit.r > fit.b, "still red: {fit:?}");
        // The palette colors are the theme already.
        let (named, _) = cell_colors(Color::Named(NamedColor::Red), BG, Flags::empty(), &p, &o);
        assert_eq!(named, p.get(NamedColor::Red as usize, &o));
        // The 256-color cube is the program's own too.
        let (cube, _) = cell_colors(Color::Indexed(196), BG, Flags::empty(), &p, &o);
        assert_ne!(cube, rgb(255, 0, 0));
        // Off for one pane.
        let (raw, _) = cell_colors_with(red, BG, Flags::empty(), &p, &o, false);
        assert_eq!(raw, rgb(255, 0, 0));
        // Text that is hard to read gets the minimum contrast.
        let (dark, back) = cell_colors(
            Color::Spec(rgb(0x30, 0x30, 0x50)),
            BG,
            Flags::empty(),
            &p,
            &o,
        );
        assert!(contrast(dark, back) >= 3.0, "{dark:?}");
        // Hidden text stays hidden.
        let (fg, bg) = cell_colors(red, BG, Flags::HIDDEN, &p, &o);
        assert_eq!(fg, bg);
        // Strength 0 takes it away.
        p.set_harmonize(Settings::default());
        let (plain, _) = cell_colors(red, BG, Flags::empty(), &p, &o);
        assert_eq!(plain, rgb(255, 0, 0));
    }

    #[test]
    fn a_palette_that_a_program_changed_fits_the_theme_too() {
        use crate::harmonize::Settings;
        let mut p = Palette::default();
        p.set_harmonize(Settings {
            strength: 1.0,
            min_contrast: 3.0,
        });
        // Like Far: the program sets blue (index 4) to the old console navy (OSC 4).
        let navy = rgb(0x00, 0x00, 0x80);
        let mut changed = Colors::default();
        changed[4] = Some(navy);
        let blue = Color::Named(NamedColor::Blue);
        let (_, bg) = cell_colors(FG, blue, Flags::empty(), &p, &changed);
        assert_ne!(bg, navy, "the program's navy fits the theme");
        assert!(bg.b > bg.r && bg.b > bg.g, "still blue: {bg:?}");
        let (_, bg) = cell_colors(FG, Color::Indexed(4), Flags::empty(), &p, &changed);
        assert_ne!(bg, navy, "by number too");
        // A color the program did not change is the theme's own.
        let o = Colors::default();
        let (_, bg) = cell_colors(FG, blue, Flags::empty(), &p, &o);
        assert_eq!(bg, p.get(NamedColor::Blue as usize, &o));
        // No harmonizer: the program's color as it set it.
        let plain = Palette::default();
        let (_, bg) = cell_colors(FG, blue, Flags::empty(), &plain, &changed);
        assert_eq!(bg, navy);
    }

    #[test]
    fn dim_color_is_darker() {
        let (normal, _) = colors(Color::Spec(rgb(200, 100, 50)), BG, Flags::empty());
        let (dim, _) = colors(Color::Spec(rgb(200, 100, 50)), BG, Flags::DIM);
        assert!(dim.r < normal.r && dim.g < normal.g && dim.b < normal.b);
        let (named, _) = colors(Color::Named(NamedColor::Red), BG, Flags::empty());
        let (named_dim, _) = colors(Color::Named(NamedColor::Red), BG, Flags::DIM);
        assert!(named_dim.r < named.r);
    }

    #[test]
    fn app_color_override_wins() {
        let mut o = Colors::default();
        o[NamedColor::Red as usize] = Some(rgb(9, 9, 9));
        o[NamedColor::Background as usize] = Some(rgb(7, 7, 7));
        let (fg, bg) = cell_colors(
            Color::Named(NamedColor::Red),
            BG,
            Flags::empty(),
            &Palette::default(),
            &o,
        );
        assert_eq!(fg, rgb(9, 9, 9));
        assert_eq!(bg, rgb(7, 7, 7));
    }

    #[test]
    fn user_colors_replace_the_built_in_ones() {
        let o = Colors::default();
        let red = rgb(200, 10, 10);
        let user = ColorOverrides {
            background: Some(rgb(1, 2, 3)),
            foreground: Some(rgb(250, 250, 250)),
            cursor: Some(rgb(9, 9, 9)),
            selection: Some(rgb(7, 7, 7)),
            ansi: [None, Some(red), None, None, None, None, None, None],
            bright: [
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(rgb(255, 255, 254)),
            ],
        };
        let p = Palette::with_colors(&user);
        assert_eq!(p.get(NamedColor::Background as usize, &o), rgb(1, 2, 3));
        assert_eq!(
            p.get(NamedColor::Foreground as usize, &o),
            rgb(250, 250, 250)
        );
        assert_eq!(
            p.get(NamedColor::BrightForeground as usize, &o),
            rgb(250, 250, 250)
        );
        assert_eq!(p.get(NamedColor::Cursor as usize, &o), rgb(9, 9, 9));
        assert_eq!(p.selection, rgb(7, 7, 7));
        assert_eq!(p.get(1, &o), red);
        assert_eq!(p.get(15, &o), rgb(255, 255, 254));
        // The dim red and the dim foreground follow the new colors.
        assert!(p.get(NamedColor::DimRed as usize, &o).r < red.r);
        assert!(p.get(NamedColor::DimRed as usize, &o).r > 100);
        assert!(p.get(NamedColor::DimForeground as usize, &o).r < 250);
        // Colors that the user did not set stay.
        let built_in = Palette::default();
        assert_eq!(p.get(2, &o), built_in.get(2, &o));
    }

    #[test]
    fn no_user_colors_is_the_default_palette() {
        let o = Colors::default();
        let p = Palette::with_colors(&ColorOverrides::default());
        let d = Palette::default();
        for i in 0..COUNT {
            assert_eq!(p.get(i, &o), d.get(i, &o), "slot {i}");
        }
        assert_eq!(p.selection, d.selection);
    }
}
