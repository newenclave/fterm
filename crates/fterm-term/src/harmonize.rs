//! Colors that programs choose themselves (truecolor and the 256-color cube) made to fit the theme,
//! and kept readable.
//!
//! A color is taken apart in Oklab (a color space where distances match what the eye sees):
//! - its hue goes toward the nearest color of the theme (a red stays red, but the theme's red);
//! - its chroma goes toward the chroma of that theme color (a gray theme makes everything gray);
//! - its lightness is kept as a distance from the background: programs expect a dark background,
//!   so on a light theme the distance goes the other way (light gray text becomes dark gray).
//!
//! `strength` (0..1) says how far. At the end, text gets at least `min_contrast` against its background.

use std::collections::HashMap;
use std::sync::Mutex;

use alacritty_terminal::vte::ansi::Rgb;

/// The settings of a theme (or of the config).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// 0 = the colors of programs as they are, 1 = fully in the style of the theme.
    pub strength: f32,
    /// The lowest contrast of text against its background (WCAG ratio, 1..21).
    pub min_contrast: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            strength: 0.0,
            min_contrast: 3.0,
        }
    }
}

/// Makes foreign colors fit one theme. Make a new one when the theme changes.
#[derive(Debug)]
pub struct Harmonizer {
    settings: Settings,
    /// The theme background in Oklab, and if it is dark.
    background: Lab,
    dark: bool,
    /// The colored colors of the theme (no grays): (hue, chroma).
    hues: Vec<(f32, f32)>,
    cache: Mutex<HashMap<u32, Rgb>>,
}

impl Clone for Harmonizer {
    fn clone(&self) -> Self {
        Self {
            settings: self.settings,
            background: self.background,
            dark: self.dark,
            hues: self.hues.clone(),
            cache: Mutex::new(HashMap::new()),
        }
    }
}

/// The lightness of the dark background that programs expect.
const PROGRAM_BACKGROUND: f32 = 0.2;
/// Below this chroma a color counts as a gray.
const GRAY: f32 = 0.03;
/// At this chroma (or more) a program color takes the full chroma of the theme color.
const VIVID: f32 = 0.1;

impl Harmonizer {
    /// `None` when `strength` is 0: nothing to do.
    pub fn new(settings: Settings, background: Rgb, colors16: &[Rgb; 16]) -> Option<Self> {
        if settings.strength <= 0.0 {
            return None;
        }
        let background = oklab(background);
        // The colored colors: red .. cyan, normal and bright.
        let hues = (1..=6)
            .chain(9..=14)
            .map(|i| oklab(colors16[i]).lch())
            .filter(|(_, c, _)| *c >= GRAY)
            .map(|(_, c, h)| (h, c))
            .collect();
        Some(Self {
            settings: Settings {
                strength: settings.strength.min(1.0),
                min_contrast: settings.min_contrast.clamp(1.0, 21.0),
            },
            dark: background.l < 0.5,
            background,
            hues,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// A color that a program chose, in the style of the theme.
    pub fn color(&self, c: Rgb) -> Rgb {
        let key = u32::from(c.r) << 16 | u32::from(c.g) << 8 | u32::from(c.b);
        if let Some(found) = self.cache.lock().ok().and_then(|m| m.get(&key).copied()) {
            return found;
        }
        let out = self.compute(c);
        if let Ok(mut cache) = self.cache.lock() {
            // A program with very many colors (a picture): start the cache again.
            if cache.len() > 8192 {
                cache.clear();
            }
            cache.insert(key, out);
        }
        out
    }

    fn compute(&self, c: Rgb) -> Rgb {
        let s = self.settings.strength;
        let (l, chroma, hue) = oklab(c).lch();
        // The lightness as a distance from the background that the program expects.
        let distance = l - PROGRAM_BACKGROUND;
        let target_l = if self.dark {
            self.background.l + distance
        } else {
            self.background.l - distance
        };
        let (target_h, target_c) = if chroma < GRAY {
            // A gray stays a gray.
            (hue, 0.0)
        } else {
            match self
                .hues
                .iter()
                .min_by(|a, b| angle(a.0, hue).total_cmp(&angle(b.0, hue)))
            {
                Some(&(h, theme_c)) => (h, theme_c * (chroma / VIVID).min(1.0)),
                // A theme with no colors (only grays): no colors.
                None => (hue, 0.0),
            }
        };
        let l = lerp(l, target_l.clamp(0.0, 1.0), s);
        let chroma = lerp(chroma, target_c, s);
        let hue = hue + signed_angle(hue, target_h) * s;
        from_lch(l, chroma, hue)
    }

    /// `fg` moved in lightness until it has `min_contrast` against `bg`.
    pub fn readable(&self, fg: Rgb, bg: Rgb) -> Rgb {
        let want = self.settings.min_contrast;
        if contrast(fg, bg) >= want {
            return fg;
        }
        let (mut l, chroma, hue) = oklab(fg).lch();
        // Away from the background: lighter on a dark one, darker on a light one.
        let step = if luminance(bg) < 0.18 { 0.02 } else { -0.02 };
        let mut out = fg;
        for _ in 0..60 {
            l = (l + step).clamp(0.0, 1.0);
            out = from_lch(l, chroma, hue);
            if contrast(out, bg) >= want || l <= 0.0 || l >= 1.0 {
                break;
            }
        }
        out
    }
}

/// The WCAG contrast ratio of two colors: 1 (the same) .. 21 (black and white).
pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The relative luminance (WCAG).
fn luminance(c: Rgb) -> f32 {
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The angle from `a` to `b` (radians), the short way round: -π..π.
fn signed_angle(a: f32, b: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let d = (b - a).rem_euclid(TAU);
    if d > PI { d - TAU } else { d }
}

fn angle(a: f32, b: f32) -> f32 {
    signed_angle(a, b).abs()
}

fn linear(c: u8) -> f32 {
    let v = f32::from(c) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

#[derive(Clone, Copy, Debug)]
struct Lab {
    l: f32,
    a: f32,
    b: f32,
}

impl Lab {
    /// Lightness, chroma, hue (radians).
    fn lch(self) -> (f32, f32, f32) {
        (self.l, self.a.hypot(self.b), self.b.atan2(self.a))
    }
}

fn oklab(c: Rgb) -> Lab {
    let (r, g, b) = (linear(c.r), linear(c.g), linear(c.b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    Lab {
        l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        a: 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        b: 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    }
}

/// Linear RGB of an Oklab color (may be out of 0..1).
fn linear_rgb(lab: Lab) -> (f32, f32, f32) {
    let l = (lab.l + 0.396_337_78 * lab.a + 0.215_803_76 * lab.b).powi(3);
    let m = (lab.l - 0.105_561_346 * lab.a - 0.063_854_17 * lab.b).powi(3);
    let s = (lab.l - 0.089_484_18 * lab.a - 1.291_485_5 * lab.b).powi(3);
    (
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    )
}

/// An sRGB color from lightness, chroma, and hue. A color out of sRGB keeps its lightness and hue
/// and loses chroma until it fits.
fn from_lch(l: f32, chroma: f32, hue: f32) -> Rgb {
    let make = |c: f32| {
        linear_rgb(Lab {
            l,
            a: c * hue.cos(),
            b: c * hue.sin(),
        })
    };
    let fits = |(r, g, b): (f32, f32, f32)| {
        let ok = |v: f32| (-0.001..=1.001).contains(&v);
        ok(r) && ok(g) && ok(b)
    };
    let mut rgb = make(chroma);
    if !fits(rgb) {
        let (mut low, mut high) = (0.0, chroma);
        for _ in 0..12 {
            let mid = (low + high) / 2.0;
            if fits(make(mid)) {
                low = mid;
            } else {
                high = mid;
            }
        }
        rgb = make(low);
    }
    Rgb {
        r: to_srgb(rgb.0),
        g: to_srgb(rgb.1),
        b: to_srgb(rgb.2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(v: u32) -> Rgb {
        Rgb {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        }
    }

    fn sixteen(list: [u32; 16]) -> [Rgb; 16] {
        list.map(rgb)
    }

    const MOCHA_BG: u32 = 0x1e1e2e;
    const MOCHA: [u32; 16] = [
        0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, //
        0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
    ];
    const LATTE_BG: u32 = 0xeff1f5;
    const LATTE: [u32; 16] = [
        0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be, //
        0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc,
    ];
    const GRAPHITE_BG: u32 = 0x1c1c1c;
    const GRAPHITE: [u32; 16] = [
        0x303030, 0x9a9a9a, 0xb0b0b0, 0xc4c4c4, 0x8a8a8a, 0xa0a0a0, 0xb8b8b8, 0xd0d0d0, //
        0x4a4a4a, 0xaaaaaa, 0xc0c0c0, 0xd4d4d4, 0x9a9a9a, 0xb0b0b0, 0xc8c8c8, 0xe8e8e8,
    ];

    fn full() -> Settings {
        Settings {
            strength: 1.0,
            min_contrast: 3.0,
        }
    }

    fn mocha(settings: Settings) -> Harmonizer {
        Harmonizer::new(settings, rgb(MOCHA_BG), &sixteen(MOCHA)).unwrap()
    }

    fn spread(c: Rgb) -> u8 {
        c.r.max(c.g).max(c.b) - c.r.min(c.g).min(c.b)
    }

    #[test]
    fn no_strength_is_no_work() {
        let none = Harmonizer::new(Settings::default(), rgb(MOCHA_BG), &sixteen(MOCHA));
        assert!(none.is_none());
    }

    #[test]
    fn a_red_stays_red_but_the_theme_red() {
        let h = mocha(full());
        let red = h.color(rgb(0xff0000));
        assert!(red.r > red.g && red.r > red.b, "still red: {red:?}");
        // Pure red is very saturated; the Mocha red is pastel: less saturated now.
        assert!(spread(red) < 255, "softer: {red:?}");
        let green = h.color(rgb(0x00c000));
        assert!(
            green.g > green.r && green.g > green.b,
            "still green: {green:?}"
        );
    }

    #[test]
    fn grays_stay_gray() {
        let h = mocha(full());
        let gray = h.color(rgb(0x808080));
        assert!(spread(gray) <= 6, "no color for a gray: {gray:?}");
    }

    #[test]
    fn a_gray_theme_makes_everything_gray() {
        let h = Harmonizer::new(full(), rgb(GRAPHITE_BG), &sixteen(GRAPHITE)).unwrap();
        for c in [0xff0000, 0x00ff00, 0x3a7bd5, 0xffcc00] {
            let out = h.color(rgb(c));
            assert!(spread(out) <= 8, "{c:06x} -> {out:?}");
        }
    }

    #[test]
    fn a_light_theme_turns_light_text_dark() {
        let h = Harmonizer::new(full(), rgb(LATTE_BG), &sixteen(LATTE)).unwrap();
        let bg = rgb(LATTE_BG);
        // Light gray text, made for a dark background.
        let text = h.color(rgb(0xd0d0d0));
        assert!(contrast(text, bg) >= 3.0, "readable: {text:?}");
        assert!(text.r < 0x90, "dark now: {text:?}");
        // A dark red background of a removed diff line becomes a light red one.
        let back = h.color(rgb(0x3c1414));
        assert!(
            back.r > 0xc0 && back.r > back.g,
            "light and reddish: {back:?}"
        );
    }

    #[test]
    fn half_strength_is_between() {
        let full = mocha(full()).color(rgb(0xff0000));
        let half = mocha(Settings {
            strength: 0.5,
            min_contrast: 3.0,
        })
        .color(rgb(0xff0000));
        assert!(spread(half) > spread(full), "{half:?} vs {full:?}");
        assert!(spread(half) < 255);
    }

    #[test]
    fn text_gets_the_minimum_contrast() {
        let h = mocha(full());
        let bg = rgb(MOCHA_BG);
        let dark_blue = rgb(0x303050);
        assert!(contrast(dark_blue, bg) < 3.0);
        let fixed = h.readable(dark_blue, bg);
        assert!(contrast(fixed, bg) >= 3.0, "{fixed:?}");
        assert!(fixed.b >= fixed.r, "still bluish: {fixed:?}");
        // Already readable: no change.
        let white = rgb(0xffffff);
        assert_eq!(h.readable(white, bg), white);
        // On a light background text gets darker.
        let light = rgb(0xffffff);
        let on_light = h.readable(rgb(0xe0e0e0), light);
        assert!(contrast(on_light, light) >= 3.0 && on_light.r < 0xe0);
    }

    #[test]
    fn contrast_ratio() {
        let ratio = contrast(rgb(0x000000), rgb(0xffffff));
        assert!((ratio - 21.0).abs() < 0.1, "{ratio}");
        assert!((contrast(rgb(0x777777), rgb(0x777777)) - 1.0).abs() < 0.01);
    }
}
