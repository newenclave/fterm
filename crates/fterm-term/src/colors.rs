//! Colors: the palette and the rules that turn a cell into real fg/bg colors.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::{COUNT, Colors};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

/// Full color table with the same slots as alacritty:
/// 0..16 named, 16..256 indexed, then foreground, background, cursor, dim colors, bright/dim foreground.
#[derive(Clone, Debug)]
pub struct Palette {
    colors: [Rgb; COUNT],
}

/// Catppuccin Mocha: 8 normal and 8 bright colors.
const ANSI: [u32; 16] = [
    0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, //
    0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
];
const FOREGROUND: u32 = 0xcdd6f4;
const BACKGROUND: u32 = 0x1e1e2e;
const CURSOR: u32 = 0xf5e0dc;

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
        Self { colors }
    }
}

impl Palette {
    /// Returns the color in `index`. A color set by the app (OSC 4/10/11) wins.
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

/// Final colors of one cell, after bold, dim, inverse, and hidden.
pub fn cell_colors(
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
}
