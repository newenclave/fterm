//! Fonts: the embedded JetBrains Mono, fallback fonts from the system, cell size, and glyph images.

use std::time::Instant;

use anyhow::{Context, Result};
use cosmic_text::fontdb::{self, Database};
use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, LayoutGlyph, Metrics, Shaping, Style, SwashCache,
    SwashContent, Weight,
};

const FAMILY: &str = "JetBrains Mono";
const FONT_FILES: [&[u8]; 4] = [
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-BoldItalic.ttf"),
];

/// Size of one terminal cell in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
    /// Distance from the top of the cell to the text baseline.
    pub baseline: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    /// One byte per pixel: alpha. The text color is added on the GPU.
    Mask,
    /// Four bytes per pixel: RGBA (for example, color emoji).
    Color,
}

/// A drawn glyph.
#[derive(Clone, Debug)]
pub struct GlyphImage {
    pub width: u32,
    pub height: u32,
    /// Offset from the left edge of the cell to the left edge of the image.
    pub left: i32,
    /// Offset from the baseline up to the top edge of the image.
    pub top: i32,
    pub kind: ImageKind,
    pub data: Vec<u8>,
}

pub struct Fonts {
    system: FontSystem,
    cache: SwashCache,
    size_px: f32,
    cell: CellMetrics,
    /// The color emoji font of this system (for chars with VS16), if there is one.
    emoji_family: Option<String>,
}

/// The color emoji fonts of macOS, Windows, and Linux, in this order of choice.
const EMOJI_FAMILIES: [&str; 5] = [
    "Apple Color Emoji",
    "Segoe UI Emoji",
    "Noto Color Emoji",
    "Twemoji Mozilla",
    "JoyPixels",
];

/// The best color emoji font among `families`.
fn pick_emoji_family<'a>(families: impl Iterator<Item = &'a str>) -> Option<String> {
    let found: Vec<&str> = families.collect();
    EMOJI_FAMILIES
        .iter()
        .find(|want| found.iter().any(|f| f.eq_ignore_ascii_case(want)))
        .map(|f| (*f).to_owned())
}

/// The cluster asks for an emoji picture: it has VS16 (U+FE0F). A char like `❤` alone may be text.
fn wants_emoji(text: &str) -> bool {
    text.contains('\u{fe0f}')
}

impl Fonts {
    /// Loads the embedded font and the system fonts (for fallback) at `size_px` pixels.
    pub fn new(size_px: f32) -> Result<Self> {
        let start = Instant::now();
        let mut db = Database::new();
        db.load_system_fonts();
        let fonts = Self::with_db(db, size_px)?;
        tracing::info!(
            faces = fonts.system.db().len(),
            ms = start.elapsed().as_millis(),
            "fonts loaded"
        );
        Ok(fonts)
    }

    /// Only the embedded font. Tests use it, so they give the same result on every machine.
    pub fn embedded_only(size_px: f32) -> Result<Self> {
        Self::with_db(Database::new(), size_px)
    }

    fn with_db(mut db: Database, size_px: f32) -> Result<Self> {
        for data in FONT_FILES {
            db.load_font_data(data.to_vec());
        }
        let regular = db
            .query(&fontdb::Query {
                families: &[fontdb::Family::Name(FAMILY)],
                ..Default::default()
            })
            .context("the embedded font is missing")?;
        let mut system = FontSystem::new_with_locale_and_db("en-US".to_owned(), db);
        let font = system
            .get_font(regular, fontdb::Weight::NORMAL)
            .context("cannot load the embedded font")?;
        let m = font.metrics();
        let scale = size_px / f32::from(m.units_per_em);
        // `descent` is negative in font units.
        let ascent = m.ascent * scale;
        let height = ((m.ascent - m.descent + m.leading) * scale).ceil();
        let baseline = (ascent + (height - (m.ascent - m.descent) * scale) / 2.0).round();

        let mut fonts = Self {
            system,
            cache: SwashCache::new(),
            size_px,
            cell: CellMetrics {
                width: 0.0,
                height,
                baseline,
            },
            emoji_family: None,
        };
        fonts.emoji_family = pick_emoji_family(
            fonts
                .system
                .db()
                .faces()
                .flat_map(|face| face.families.iter().map(|(name, _)| name.as_str())),
        );
        // Whole pixels, so the backgrounds of cells meet with no gaps.
        fonts.cell.width = fonts.advance('M').round();
        Ok(fonts)
    }

    pub fn cell(&self) -> CellMetrics {
        self.cell
    }

    pub fn size_px(&self) -> f32 {
        self.size_px
    }

    /// Advance width of one char, in pixels.
    pub fn advance(&mut self, c: char) -> f32 {
        let mut text = [0; 4];
        let glyphs = self.shape(c.encode_utf8(&mut text), false, false, self.size_px);
        glyphs.first().map_or(0.0, |glyph| glyph.w)
    }

    /// Name of the font that draws the first char of `text`. For logs and tests.
    pub fn font_family(&mut self, text: &str) -> Option<String> {
        let glyph = self
            .shape(text, false, false, self.size_px)
            .into_iter()
            .next()?;
        let face = self.system.db().face(glyph.font_id)?;
        face.families.first().map(|(name, _)| name.clone())
    }

    /// Draws a cluster (a char and its combining marks) that takes `cells` cells (1 or 2).
    /// `None` means there is nothing to draw (for example, a space).
    pub fn rasterize(
        &mut self,
        text: &str,
        bold: bool,
        italic: bool,
        cells: u32,
    ) -> Option<GlyphImage> {
        let cell = self.cell;
        let target_width = cells.max(1) as f32 * cell.width;
        let mut glyphs = self.shape(text, bold, italic, self.size_px);
        if glyphs.is_empty() {
            return None;
        }
        if glyphs.iter().all(|g| g.glyph_id == 0) {
            // No font has this char.
            return Some(missing_glyph_box(cell, target_width));
        }

        let mut image = self.compose(&glyphs)?;
        let scale = if image.kind == ImageKind::Color {
            // Emoji fill their cells, like in other terminals: they can get bigger or smaller.
            (target_width / image.width as f32).min(cell.height / image.height as f32) * 0.95
        } else {
            // Too wide or too high (for example a fallback font with big glyphs): draw it smaller.
            (target_width / run_width(&glyphs))
                .min(cell.height / image.height as f32)
                .min(1.0)
        };
        if !(0.98..=1.02).contains(&scale) {
            glyphs = self.shape(text, bold, italic, self.size_px * scale * 0.98);
            image = self.compose(&glyphs)?;
        }

        // Center narrow glyphs in their cells (for example, a normal letter in a wide cell).
        let width = run_width(&glyphs);
        let shift = ((target_width - width) / 2.0).max(0.0).round() as i32;
        image.left += shift;
        // Keep the image inside the cells.
        let max_left = (target_width - image.width as f32).max(0.0) as i32;
        image.left = image.left.clamp(0, max_left);
        let above = cell.baseline as i32;
        let below = (cell.height - cell.baseline) as i32;
        image.top = image.top.min(above).max(image.height as i32 - below);
        Some(image)
    }

    /// Draws all glyphs of a run into one image.
    fn compose(&mut self, glyphs: &[LayoutGlyph]) -> Option<GlyphImage> {
        struct Part {
            x: i32,
            /// y of the top edge, down from the baseline.
            y: i32,
            width: u32,
            height: u32,
            content: SwashContent,
            data: Vec<u8>,
        }
        let mut parts = Vec::new();
        for glyph in glyphs {
            let physical = glyph.physical((0.0, 0.0), 1.0);
            let Some(image) = self
                .cache
                .get_image(&mut self.system, physical.cache_key)
                .as_ref()
            else {
                continue;
            };
            if image.placement.width == 0 || image.placement.height == 0 {
                continue;
            }
            parts.push(Part {
                x: physical.x + image.placement.left,
                y: physical.y - image.placement.top,
                width: image.placement.width,
                height: image.placement.height,
                content: image.content,
                data: image.data.clone(),
            });
        }
        let min_x = parts.iter().map(|p| p.x).min()?;
        let min_y = parts.iter().map(|p| p.y).min()?;
        let max_x = parts.iter().map(|p| p.x + p.width as i32).max()?;
        let max_y = parts.iter().map(|p| p.y + p.height as i32).max()?;
        let (width, height) = ((max_x - min_x) as u32, (max_y - min_y) as u32);
        let color = parts.iter().any(|p| p.content == SwashContent::Color);
        let kind = if color {
            ImageKind::Color
        } else {
            ImageKind::Mask
        };
        let bytes = if color { 4 } else { 1 };
        let mut data = vec![0u8; (width * height * bytes) as usize];

        for part in &parts {
            let (ox, oy) = ((part.x - min_x) as u32, (part.y - min_y) as u32);
            for py in 0..part.height {
                for px in 0..part.width {
                    let i = (py * part.width + px) as usize;
                    // This pixel as RGBA. Masks are white, so a color glyph keeps its own colors.
                    let rgba = match part.content {
                        SwashContent::Mask => [255, 255, 255, part.data[i]],
                        SwashContent::Color => {
                            let p = &part.data[i * 4..i * 4 + 4];
                            [p[0], p[1], p[2], p[3]]
                        }
                        SwashContent::SubpixelMask => {
                            let p = &part.data[i * 4..i * 4 + 4];
                            [255, 255, 255, p[0].max(p[1]).max(p[2])]
                        }
                    };
                    let o = (((oy + py) * width) + ox + px) as usize;
                    if color {
                        if rgba[3] > data[o * 4 + 3] {
                            data[o * 4..o * 4 + 4].copy_from_slice(&rgba);
                        }
                    } else {
                        data[o] = data[o].max(rgba[3]);
                    }
                }
            }
        }
        Some(GlyphImage {
            width,
            height,
            left: min_x,
            top: -min_y,
            kind,
            data,
        })
    }

    /// Shapes `text` and returns its glyphs.
    fn shape(&mut self, text: &str, bold: bool, italic: bool, size_px: f32) -> Vec<LayoutGlyph> {
        let metrics = Metrics::new(size_px, self.cell.height.max(size_px));
        let mut buffer = Buffer::new(&mut self.system, metrics);
        // VS16 asks for a color emoji. The fallback of cosmic-text can take a text font for it
        // (macOS draws `❤️` with a text heart), so ask the emoji font by name.
        let family = match &self.emoji_family {
            Some(emoji) if wants_emoji(text) => emoji.clone(),
            _ => FAMILY.to_owned(),
        };
        let attrs = Attrs::new()
            .family(Family::Name(&family))
            .weight(if bold { Weight::BOLD } else { Weight::NORMAL })
            .style(if italic { Style::Italic } else { Style::Normal });
        let mut buffer = buffer.borrow_with(&mut self.system);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(true);
        buffer
            .layout_runs()
            .next()
            .map(|run| run.glyphs.to_vec())
            .unwrap_or_default()
    }
}

/// Width of a shaped run: from the start to the end of the last glyph.
fn run_width(glyphs: &[LayoutGlyph]) -> f32 {
    glyphs.iter().map(|g| g.x + g.w).fold(0.0, f32::max)
}

/// A hollow box for chars that no font has. It looks the same on every machine.
fn missing_glyph_box(cell: CellMetrics, target_width: f32) -> GlyphImage {
    let width = (target_width - 2.0).max(3.0) as u32;
    let height = (cell.height * 0.7).round().max(3.0) as u32;
    let mut data = vec![0u8; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            if x == 0 || y == 0 || x == width - 1 || y == height - 1 {
                data[(y * width + x) as usize] = 255;
            }
        }
    }
    // Put the box in the middle of the cell height.
    let top_gap = ((cell.height - height as f32) / 2.0).round();
    GlyphImage {
        width,
        height,
        left: 1,
        top: (cell.baseline - top_gap) as i32,
        kind: ImageKind::Mask,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embedded(size: f32) -> Fonts {
        Fonts::embedded_only(size).unwrap()
    }

    fn solid_pixels(image: &GlyphImage) -> usize {
        match image.kind {
            ImageKind::Mask => image.data.iter().filter(|&&a| a > 200).count(),
            ImageKind::Color => image.data.chunks(4).filter(|px| px[3] > 200).count(),
        }
    }

    #[test]
    fn embedded_font_gives_a_real_cell() {
        let fonts = embedded(16.0);
        let cell = fonts.cell();
        assert!(cell.width > 5.0 && cell.width < 16.0, "{cell:?}");
        assert!(cell.height >= 16.0 && cell.height < 30.0, "{cell:?}");
        assert!(
            cell.baseline > 0.0 && cell.baseline < cell.height,
            "{cell:?}"
        );
    }

    #[test]
    fn font_is_monospace() {
        let mut fonts = embedded(16.0);
        let m = fonts.advance('M');
        assert_eq!(fonts.advance('i'), m);
        assert_eq!(fonts.advance('W'), m);
        assert_eq!(fonts.cell().width, m.round());
    }

    #[test]
    fn cell_size_is_whole_pixels() {
        // Whole pixels: no thin gaps between the backgrounds of cells.
        for size in [11.0, 13.0, 15.5, 19.5] {
            let cell = embedded(size).cell();
            assert_eq!(cell.width.fract(), 0.0, "{size}: {cell:?}");
            assert_eq!(cell.height.fract(), 0.0, "{size}: {cell:?}");
        }
    }

    #[test]
    fn bigger_font_gives_bigger_cells() {
        let small = embedded(12.0).cell();
        let big = embedded(24.0).cell();
        assert!(big.width > small.width && big.height > small.height);
    }

    #[test]
    fn letter_has_an_image() {
        let mut fonts = embedded(16.0);
        let glyph = fonts
            .rasterize("A", false, false, 1)
            .expect("no image for A");
        assert!(glyph.width > 0 && glyph.height > 0);
        assert_eq!(glyph.kind, ImageKind::Mask);
        assert_eq!(glyph.data.len(), (glyph.width * glyph.height) as usize);
        assert!(solid_pixels(&glyph) > 0, "A has no solid pixels");
        // Cyrillic is in JetBrains Mono too.
        assert!(fonts.rasterize("Ж", true, false, 1).is_some());
    }

    #[test]
    fn space_has_no_image() {
        let mut fonts = embedded(16.0);
        assert!(fonts.rasterize(" ", false, false, 1).is_none());
    }

    #[test]
    fn combining_mark_is_drawn_with_its_letter() {
        let mut fonts = embedded(16.0);
        let e = fonts.rasterize("e", false, false, 1).unwrap();
        let e_acute = fonts.rasterize("e\u{0301}", false, false, 1).unwrap();
        // The accent is above the letter, so the image goes higher.
        assert!(e_acute.top > e.top, "{} vs {}", e_acute.top, e.top);
        assert!(solid_pixels(&e_acute) > solid_pixels(&e));
    }

    #[test]
    fn glyph_stays_inside_its_cells() {
        let mut fonts = embedded(16.0);
        let cell = fonts.cell();
        for text in ["W", "@", "|", "g", "Ж", "Q"] {
            let g = fonts.rasterize(text, false, false, 1).unwrap();
            assert!(g.left >= 0, "{text}: {g:?}");
            assert!(
                g.left as f32 + g.width as f32 <= cell.width + 1.0,
                "{text}: {g:?}"
            );
            assert!(g.top as f32 <= cell.baseline, "{text}: {g:?}");
            assert!(
                g.height as f32 - g.top as f32 <= cell.height - cell.baseline + 0.5,
                "{text}"
            );
        }
    }

    #[test]
    fn too_wide_text_is_made_smaller() {
        let mut fonts = embedded(16.0);
        let cell = fonts.cell();
        // Three letters in one cell: they must be scaled down to fit.
        let g = fonts.rasterize("WWW", false, false, 1).unwrap();
        assert!(g.left as f32 + g.width as f32 <= cell.width + 1.0, "{g:?}");
    }

    #[test]
    fn narrow_glyph_is_centered_in_two_cells() {
        let mut fonts = embedded(16.0);
        let cell = fonts.cell();
        let one = fonts.rasterize("M", false, false, 1).unwrap();
        let two = fonts.rasterize("M", false, false, 2).unwrap();
        let expected = one.left as f32 + cell.width / 2.0;
        assert!((two.left as f32 - expected).abs() <= 1.0, "{one:?} {two:?}");
    }

    #[test]
    fn char_without_a_font_is_a_hollow_box() {
        let mut fonts = embedded(16.0);
        let cell = fonts.cell();
        let g = fonts.rasterize("界", false, false, 2).expect("no box");
        assert_eq!(g.kind, ImageKind::Mask);
        let at = |x: u32, y: u32| g.data[(y * g.width + x) as usize];
        assert!(at(g.width / 2, 0) > 200, "top border");
        assert!(at(0, g.height / 2) > 200, "left border");
        assert_eq!(at(g.width / 2, g.height / 2), 0, "hollow inside");
        assert!(g.width as f32 <= 2.0 * cell.width);
    }

    /// Fonts from this machine. Returns `None` (and the test does nothing) when `text` has no fallback font.
    fn system_with(text: &str) -> Option<Fonts> {
        let mut fonts = Fonts::new(16.0).unwrap();
        match fonts.font_family(text) {
            Some(family) if family != FAMILY => Some(fonts),
            _ => {
                eprintln!("skipped: no system font for {text:?}");
                None
            }
        }
    }

    #[test]
    fn the_emoji_font_of_each_system() {
        let families = ["Arial", "Noto Color Emoji", "Segoe UI Emoji"];
        assert_eq!(
            pick_emoji_family(families.iter().copied()).as_deref(),
            Some("Segoe UI Emoji")
        );
        assert_eq!(
            pick_emoji_family(["Noto Color Emoji", "DejaVu Sans"].iter().copied()).as_deref(),
            Some("Noto Color Emoji")
        );
        assert_eq!(
            pick_emoji_family(["Apple Color Emoji"].iter().copied()).as_deref(),
            Some("Apple Color Emoji")
        );
        assert_eq!(pick_emoji_family(["Arial"].iter().copied()), None);
    }

    #[test]
    fn vs16_asks_for_an_emoji() {
        assert!(wants_emoji("❤\u{fe0f}"));
        assert!(wants_emoji("✔\u{fe0f}"));
        assert!(!wants_emoji("❤"), "a heart alone may be text");
        assert!(!wants_emoji("a"));
    }

    #[test]
    fn a_char_with_vs16_uses_the_emoji_font() {
        let mut fonts = Fonts::new(16.0).unwrap();
        let Some(emoji) = fonts.emoji_family.clone() else {
            eprintln!("skipped: no emoji font here");
            return;
        };
        for text in ["❤\u{fe0f}", "✔\u{fe0f}", "☀\u{fe0f}"] {
            assert_eq!(
                fonts.font_family(text).as_deref(),
                Some(emoji.as_str()),
                "{text:?}"
            );
        }
        // Without VS16 a plain char keeps its text font.
        assert_eq!(fonts.font_family("a").as_deref(), Some(FAMILY));
    }

    #[test]
    fn ascii_still_uses_jetbrains_mono_with_system_fonts() {
        let mut fonts = Fonts::new(16.0).unwrap();
        assert_eq!(fonts.font_family("a").as_deref(), Some(FAMILY));
        assert_eq!(fonts.font_family("Ж").as_deref(), Some(FAMILY));
    }

    #[test]
    fn cjk_uses_a_fallback_font() {
        let Some(mut fonts) = system_with("界") else {
            return;
        };
        let g = fonts.rasterize("界", false, false, 2).unwrap();
        assert_eq!(g.kind, ImageKind::Mask);
        // A real glyph has pixels inside, not only a border.
        let at = |x: u32, y: u32| g.data[(y * g.width + x) as usize];
        assert!((0..g.height).any(|y| at(g.width / 2, y) > 200));
    }

    #[test]
    fn all_sample_emoji_fill_their_cells() {
        let Some(_) = system_with("😀") else {
            return;
        };
        for size in [14.0, 16.0, 21.0] {
            let mut fonts = Fonts::new(size).unwrap();
            let cell = fonts.cell();
            for emoji in ["😀", "🎉", "🚀", "❤\u{fe0f}", "✅", "👍", "👨\u{200d}"] {
                let g = fonts.rasterize(emoji, false, false, 2).unwrap();
                // Wide shapes (like a heart) fill the width, tall shapes fill the height.
                let fills = g.height as f32 >= 0.7 * cell.height
                    || g.width as f32 >= 0.85 * 2.0 * cell.width;
                assert!(
                    fills && g.height as f32 <= cell.height + 1.0,
                    "{emoji:?} at {size}: {}x{} in {cell:?}",
                    g.width,
                    g.height
                );
            }
        }
    }

    #[test]
    fn emoji_is_a_color_image_that_fits_two_cells() {
        let Some(mut fonts) = system_with("😀") else {
            return;
        };
        let cell = fonts.cell();
        let g = fonts.rasterize("😀", false, false, 2).unwrap();
        assert_eq!(g.kind, ImageKind::Color);
        assert_eq!(g.data.len(), (g.width * g.height * 4) as usize);
        assert!(g.width as f32 <= 2.0 * cell.width + 1.0, "{g:?}");
        assert!(g.height as f32 <= cell.height + 1.0, "{g:?}");
        assert!(solid_pixels(&g) > 20);
        // And not too small: it should use most of the cell height.
        assert!(
            g.height as f32 >= 0.7 * cell.height,
            "emoji is too small: {}x{} in a {:?} cell",
            g.width,
            g.height,
            cell
        );
    }
}
