//! Fonts: the embedded JetBrains Mono, cell size, and glyph images.

use anyhow::{Context, Result};
use cosmic_text::fontdb::{self, Database};
use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, SwashCache, SwashContent, Weight,
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

/// A glyph drawn as an alpha mask (one byte per pixel).
#[derive(Clone, Debug)]
pub struct GlyphImage {
    pub width: u32,
    pub height: u32,
    /// Offset from the pen position to the left edge of the image.
    pub left: i32,
    /// Offset from the baseline up to the top edge of the image.
    pub top: i32,
    pub alpha: Vec<u8>,
}

pub struct Fonts {
    system: FontSystem,
    cache: SwashCache,
    size_px: f32,
    cell: CellMetrics,
}

impl Fonts {
    /// Loads the embedded font at `size_px` pixels.
    pub fn new(size_px: f32) -> Result<Self> {
        // Only the embedded faces: the result is the same on every machine.
        let mut db = Database::new();
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
        };
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
        self.shape(c, false, false).map_or(0.0, |glyph| glyph.w)
    }

    /// Draws one char. `None` means there is nothing to draw (for example, a space).
    pub fn rasterize(&mut self, c: char, bold: bool, italic: bool) -> Option<GlyphImage> {
        let glyph = self.shape(c, bold, italic)?;
        let physical = glyph.physical((0.0, 0.0), 1.0);
        let image = self
            .cache
            .get_image(&mut self.system, physical.cache_key)
            .as_ref()?;
        let (width, height) = (image.placement.width, image.placement.height);
        if width == 0 || height == 0 {
            return None;
        }
        let alpha = match image.content {
            SwashContent::Mask => image.data.clone(),
            // Color glyphs (emoji) come in Phase 2. For now we keep only their shape.
            SwashContent::Color => image.data.chunks_exact(4).map(|px| px[3]).collect(),
            SwashContent::SubpixelMask => image
                .data
                .chunks_exact(4)
                .map(|px| px[0].max(px[1]).max(px[2]))
                .collect(),
        };
        Some(GlyphImage {
            width,
            height,
            left: image.placement.left + physical.x,
            top: image.placement.top - physical.y,
            alpha,
        })
    }

    /// Shapes one char and returns its glyph.
    fn shape(&mut self, c: char, bold: bool, italic: bool) -> Option<cosmic_text::LayoutGlyph> {
        let metrics = Metrics::new(self.size_px, self.cell.height.max(self.size_px));
        let mut buffer = Buffer::new(&mut self.system, metrics);
        let attrs = Attrs::new()
            .family(Family::Name(FAMILY))
            .weight(if bold { Weight::BOLD } else { Weight::NORMAL })
            .style(if italic { Style::Italic } else { Style::Normal });
        let mut text = [0; 4];
        let mut buffer = buffer.borrow_with(&mut self.system);
        buffer.set_text(c.encode_utf8(&mut text), &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(true);
        let glyph = buffer.layout_runs().next()?.glyphs.first()?.clone();
        Some(glyph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_font_gives_a_real_cell() {
        let fonts = Fonts::new(16.0).unwrap();
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
        let mut fonts = Fonts::new(16.0).unwrap();
        let m = fonts.advance('M');
        assert_eq!(fonts.advance('i'), m);
        assert_eq!(fonts.advance('W'), m);
        assert_eq!(fonts.cell().width, m.round());
    }

    #[test]
    fn cell_size_is_whole_pixels() {
        // Whole pixels: no thin gaps between the backgrounds of cells.
        for size in [11.0, 13.0, 15.5, 19.5] {
            let cell = Fonts::new(size).unwrap().cell();
            assert_eq!(cell.width.fract(), 0.0, "{size}: {cell:?}");
            assert_eq!(cell.height.fract(), 0.0, "{size}: {cell:?}");
        }
    }

    #[test]
    fn bigger_font_gives_bigger_cells() {
        let small = Fonts::new(12.0).unwrap().cell();
        let big = Fonts::new(24.0).unwrap().cell();
        assert!(big.width > small.width && big.height > small.height);
    }

    #[test]
    fn letter_has_an_image() {
        let mut fonts = Fonts::new(16.0).unwrap();
        let glyph = fonts.rasterize('A', false, false).expect("no image for A");
        assert!(glyph.width > 0 && glyph.height > 0);
        assert_eq!(glyph.alpha.len(), (glyph.width * glyph.height) as usize);
        assert!(
            glyph.alpha.iter().any(|&a| a > 200),
            "A has no solid pixels"
        );
        // Cyrillic is in JetBrains Mono too.
        assert!(fonts.rasterize('Ж', true, false).is_some());
    }

    #[test]
    fn space_has_no_image() {
        let mut fonts = Fonts::new(16.0).unwrap();
        assert!(fonts.rasterize(' ', false, false).is_none());
    }
}
