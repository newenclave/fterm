//! Glyph atlas: where each glyph image lives in one big texture.
//! This part has no GPU code. The caller uploads the pixels in `upload`.

use std::collections::HashMap;

use etagere::{AtlasAllocator, size2};

use crate::font::GlyphImage;

/// Empty pixels between glyphs, so one glyph never touches the next.
const GAP: u32 = 1;

/// One cell to draw: a char, its combining marks, and the style.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct GlyphKey {
    pub c: char,
    /// Zero-width chars after `c` (combining marks, VS16, ZWJ). `None` for most cells, so no allocation.
    pub extra: Option<Box<[char]>>,
    pub bold: bool,
    pub italic: bool,
    /// The char takes 2 cells.
    pub wide: bool,
    /// A Braille char drawn as round dots (`false` = as square pixels). Only Braille chars set it.
    pub dots: bool,
}

impl GlyphKey {
    /// The whole cluster as a string.
    pub fn text(&self) -> String {
        std::iter::once(self.c)
            .chain(self.extra.iter().flat_map(|extra| extra.iter().copied()))
            .collect()
    }
}

/// A glyph in the atlas texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasGlyph {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    /// RGBA (for example, color emoji) and not an alpha mask.
    pub color: bool,
}

/// There is no free space. Call `grow` and draw the frame again.
#[derive(Debug, PartialEq, Eq)]
pub struct AtlasFull;

pub struct GlyphAtlas {
    size: u32,
    allocator: AtlasAllocator,
    glyphs: HashMap<GlyphKey, Option<AtlasGlyph>>,
}

impl GlyphAtlas {
    pub fn new(size: u32) -> Self {
        Self {
            size,
            allocator: AtlasAllocator::new(size2(size as i32, size as i32)),
            glyphs: HashMap::new(),
        }
    }

    /// Width and height of the texture.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Finds the glyph, or draws it with `rasterize` and puts it in the atlas.
    /// `upload` gets the place and the image, to copy the pixels to the texture.
    /// `Ok(None)` means the glyph has nothing to draw.
    pub fn get(
        &mut self,
        key: &GlyphKey,
        rasterize: impl FnOnce() -> Option<GlyphImage>,
        upload: impl FnOnce(&AtlasGlyph, &GlyphImage),
    ) -> Result<Option<AtlasGlyph>, AtlasFull> {
        if let Some(glyph) = self.glyphs.get(key) {
            return Ok(*glyph);
        }
        let Some(image) = rasterize() else {
            self.glyphs.insert(key.clone(), None);
            return Ok(None);
        };
        let allocation = self
            .allocator
            .allocate(size2(
                (image.width + GAP) as i32,
                (image.height + GAP) as i32,
            ))
            .ok_or(AtlasFull)?;
        let origin = allocation.rectangle.min;
        let glyph = AtlasGlyph {
            x: origin.x as u32,
            y: origin.y as u32,
            width: image.width,
            height: image.height,
            left: image.left,
            top: image.top,
            color: image.kind == crate::font::ImageKind::Color,
        };
        upload(&glyph, &image);
        self.glyphs.insert(key.clone(), Some(glyph));
        Ok(Some(glyph))
    }

    /// `Some(glyph)` when this key is already in the atlas (`Some(None)` = known, nothing to draw).
    pub fn cached(&self, key: &GlyphKey) -> Option<Option<AtlasGlyph>> {
        self.glyphs.get(key).copied()
    }

    /// Makes the atlas 2 times bigger and forgets all glyphs.
    pub fn grow(&mut self) {
        *self = Self::new(self.size * 2);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn image(width: u32, height: u32) -> GlyphImage {
        GlyphImage {
            width,
            height,
            left: 1,
            top: 2,
            kind: crate::font::ImageKind::Mask,
            data: vec![255; (width * height) as usize],
        }
    }

    fn key(c: char) -> GlyphKey {
        GlyphKey {
            c,
            extra: None,
            bold: false,
            italic: false,
            wide: false,
            dots: false,
        }
    }

    #[test]
    fn key_text_has_the_whole_cluster() {
        assert_eq!(key('a').text(), "a");
        let k = GlyphKey {
            extra: Some(vec!['\u{0301}', '\u{0302}'].into_boxed_slice()),
            ..key('e')
        };
        assert_eq!(k.text(), "e\u{0301}\u{0302}");
    }

    #[test]
    fn cached_tells_if_the_key_is_known() {
        let mut atlas = GlyphAtlas::new(64);
        assert_eq!(atlas.cached(&key('a')), None);
        let placed = atlas
            .get(&key('a'), || Some(image(4, 4)), |_, _| {})
            .unwrap();
        assert_eq!(atlas.cached(&key('a')), Some(placed));
        atlas.get(&key(' '), || None, |_, _| {}).unwrap();
        assert_eq!(atlas.cached(&key(' ')), Some(None));
    }

    #[test]
    fn color_images_are_marked_as_color() {
        let mut atlas = GlyphAtlas::new(64);
        let color = GlyphImage {
            kind: crate::font::ImageKind::Color,
            data: vec![255; 4 * 4 * 4],
            ..image(4, 4)
        };
        let g = atlas
            .get(&key('x'), || Some(color), |_, _| {})
            .unwrap()
            .unwrap();
        assert!(g.color);
        let g = atlas
            .get(&key('y'), || Some(image(4, 4)), |_, _| {})
            .unwrap()
            .unwrap();
        assert!(!g.color);
    }

    fn overlap(a: &AtlasGlyph, b: &AtlasGlyph) -> bool {
        a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
    }

    #[test]
    fn same_key_is_drawn_once() {
        let mut atlas = GlyphAtlas::new(256);
        let calls = Cell::new(0);
        let uploads = Cell::new(0);
        for _ in 0..3 {
            let glyph = atlas
                .get(
                    &key('a'),
                    || {
                        calls.set(calls.get() + 1);
                        Some(image(8, 10))
                    },
                    |_, _| uploads.set(uploads.get() + 1),
                )
                .unwrap()
                .unwrap();
            assert_eq!(
                (glyph.width, glyph.height, glyph.left, glyph.top),
                (8, 10, 1, 2)
            );
        }
        assert_eq!(calls.get(), 1);
        assert_eq!(uploads.get(), 1);
    }

    #[test]
    fn empty_glyph_is_remembered() {
        let mut atlas = GlyphAtlas::new(256);
        let calls = Cell::new(0);
        for _ in 0..2 {
            let got = atlas.get(
                &key(' '),
                || {
                    calls.set(calls.get() + 1);
                    None
                },
                |_, _| panic!("nothing to upload"),
            );
            assert_eq!(got, Ok(None));
        }
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn glyphs_do_not_overlap_and_stay_inside() {
        let mut atlas = GlyphAtlas::new(128);
        let mut placed = Vec::new();
        for c in 'a'..='z' {
            let glyph = atlas
                .get(&key(c), || Some(image(9, 17)), |_, _| {})
                .unwrap()
                .unwrap();
            assert!(glyph.x + glyph.width <= 128 && glyph.y + glyph.height <= 128);
            placed.push(glyph);
        }
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                assert!(!overlap(a, b), "{a:?} and {b:?} overlap");
            }
        }
    }

    #[test]
    fn full_atlas_says_so_and_can_grow() {
        let mut atlas = GlyphAtlas::new(32);
        assert_eq!(
            atlas
                .get(&key('a'), || Some(image(20, 20)), |_, _| {})
                .map(|g| g.is_some()),
            Ok(true)
        );
        assert_eq!(
            atlas.get(&key('b'), || Some(image(20, 20)), |_, _| {}),
            Err(AtlasFull)
        );

        atlas.grow();
        assert_eq!(atlas.size(), 64);
        // Old glyphs are gone after `grow`, so `a` is drawn again.
        let drawn = Cell::new(false);
        atlas
            .get(
                &key('a'),
                || {
                    drawn.set(true);
                    Some(image(20, 20))
                },
                |_, _| {},
            )
            .unwrap();
        assert!(drawn.get());
        assert!(
            atlas
                .get(&key('b'), || Some(image(20, 20)), |_, _| {})
                .is_ok()
        );
    }
}
