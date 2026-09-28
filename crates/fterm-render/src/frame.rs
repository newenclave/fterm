//! Builds one frame: the terminal grid becomes a list of quads for the GPU.
//! This part has no GPU code, so we can test it with a real `Term`.

use bytemuck::{Pod, Zeroable};
use fterm_term::alacritty_terminal::event::EventListener;
use fterm_term::alacritty_terminal::term::Term;
use fterm_term::alacritty_terminal::term::cell::Flags;
use fterm_term::alacritty_terminal::vte::ansi::{CursorShape, NamedColor};
use fterm_term::colors::{Palette, cell_colors};

use crate::atlas::{AtlasFull, AtlasGlyph, GlyphKey};
use crate::color::linear;
use crate::font::CellMetrics;

/// A filled rectangle (background, cursor, underline).
pub const KIND_SOLID: u32 = 0;
/// A glyph: alpha from the atlas times `color`.
pub const KIND_GLYPH: u32 = 1;

/// One quad. The layout must match `Instance` in `shader.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Instance {
    /// x, y, width, height in pixels.
    pub rect: [f32; 4],
    /// x, y, width, height in atlas pixels (only for glyphs).
    pub uv: [f32; 4],
    /// Linear RGBA.
    pub color: [f32; 4],
    pub kind: u32,
    pub _pad: [u32; 3],
}

pub struct FrameInput<'a> {
    pub cell: CellMetrics,
    pub padding: f32,
    pub palette: &'a Palette,
    pub focused: bool,
}

/// Makes all quads for the frame. `glyph` finds a glyph in the atlas (or adds it).
pub fn build_frame<T: EventListener>(
    term: &Term<T>,
    input: &FrameInput,
    glyph: &mut impl FnMut(GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
) -> Result<Vec<Instance>, AtlasFull> {
    let content = term.renderable_content();
    let cell = input.cell;
    let overrides = content.colors;
    let default_bg = input
        .palette
        .get(NamedColor::Background as usize, overrides);
    let offset = content.display_offset as i32;
    let origin = |col: usize, row: i32| {
        (
            input.padding + col as f32 * cell.width,
            input.padding + row as f32 * cell.height,
        )
    };
    let line = (cell.height / 16.0).round().max(1.0);

    let cursor = content.cursor;
    let cursor_shape = match cursor.shape {
        CursorShape::Block if !input.focused => CursorShape::HollowBlock,
        shape => shape,
    };
    let cursor_row = cursor.point.line.0 + offset;
    let cursor_col = cursor.point.column.0;

    let mut backgrounds = Vec::new();
    let mut glyphs = Vec::new();
    let mut lines = Vec::new();
    let mut cursor_width = cell.width;

    for indexed in content.display_iter {
        let flags = indexed.cell.flags;
        if flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let row = indexed.point.line.0 + offset;
        let col = indexed.point.column.0;
        let (x, y) = origin(col, row);
        let width = if flags.contains(Flags::WIDE_CHAR) {
            2.0 * cell.width
        } else {
            cell.width
        };
        let (fg, bg) = cell_colors(
            indexed.cell.fg,
            indexed.cell.bg,
            flags,
            input.palette,
            overrides,
        );
        let under_block_cursor =
            cursor_shape == CursorShape::Block && row == cursor_row && col == cursor_col;
        if row == cursor_row && col == cursor_col {
            cursor_width = width;
        }

        if bg != default_bg {
            backgrounds.push(solid([x, y, width, cell.height], linear(bg)));
        }

        let c = indexed.cell.c;
        if c != ' ' && c != '\t' {
            let key = GlyphKey {
                c,
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
            };
            if let Some(g) = glyph(key)? {
                let color = if under_block_cursor { bg } else { fg };
                glyphs.push(Instance {
                    rect: [
                        x + g.left as f32,
                        y + cell.baseline - g.top as f32,
                        g.width as f32,
                        g.height as f32,
                    ],
                    uv: [g.x as f32, g.y as f32, g.width as f32, g.height as f32],
                    color: linear(color),
                    kind: KIND_GLYPH,
                    _pad: [0; 3],
                });
            }
        }

        if flags.intersects(Flags::ALL_UNDERLINES) {
            let top = (cell.baseline + line).min(cell.height - line);
            lines.push(solid([x, y + top, width, line], linear(fg)));
        }
        if flags.contains(Flags::STRIKEOUT) {
            let top = (cell.baseline * 0.65).round();
            lines.push(solid([x, y + top, width, line], linear(fg)));
        }
    }

    let cursor_color = linear(input.palette.get(NamedColor::Cursor as usize, overrides));
    let (x, y) = origin(cursor_col, cursor_row);
    let (w, h) = (cursor_width, cell.height);
    let beam = (cell.width / 8.0).round().max(1.0);
    let mut cursor_back = Vec::new();
    let mut cursor_front = Vec::new();
    match cursor_shape {
        CursorShape::Block => cursor_back.push(solid([x, y, w, h], cursor_color)),
        CursorShape::Beam => cursor_front.push(solid([x, y, beam, h], cursor_color)),
        CursorShape::Underline => {
            cursor_front.push(solid([x, y + h - line, w, line], cursor_color))
        }
        CursorShape::HollowBlock => cursor_front.extend([
            solid([x, y, w, line], cursor_color),
            solid([x, y + h - line, w, line], cursor_color),
            solid([x, y + line, line, h - 2.0 * line], cursor_color),
            solid([x + w - line, y + line, line, h - 2.0 * line], cursor_color),
        ]),
        CursorShape::Hidden => {}
    }

    let mut quads = backgrounds;
    quads.extend(cursor_back);
    quads.extend(glyphs);
    quads.extend(lines);
    quads.extend(cursor_front);
    Ok(quads)
}

fn solid(rect: [f32; 4], color: [f32; 4]) -> Instance {
    Instance {
        rect,
        uv: [0.0; 4],
        color,
        kind: KIND_SOLID,
        _pad: [0; 3],
    }
}

#[cfg(test)]
mod tests {
    use fterm_term::alacritty_terminal::event::VoidListener;
    use fterm_term::alacritty_terminal::term::Config;
    use fterm_term::alacritty_terminal::term::color::Colors;
    use fterm_term::alacritty_terminal::vte::ansi::{Processor, Rgb};
    use fterm_term::size::GridSize;

    use super::*;

    const CELL: CellMetrics = CellMetrics {
        width: 10.0,
        height: 20.0,
        baseline: 15.0,
    };
    const PADDING: f32 = 4.0;
    /// Every glyph in the fake atlas has this place and size.
    const GLYPH: AtlasGlyph = AtlasGlyph {
        x: 3,
        y: 5,
        width: 8,
        height: 12,
        left: 1,
        top: 11,
    };

    fn term_with(bytes: &[u8]) -> Term<VoidListener> {
        let mut term = Term::new(Config::default(), &GridSize::new(10, 3), VoidListener);
        Processor::<fterm_term::alacritty_terminal::vte::ansi::StdSyncHandler>::new()
            .advance(&mut term, bytes);
        term
    }

    fn frame(bytes: &[u8], focused: bool) -> (Vec<Instance>, Vec<GlyphKey>) {
        let term = term_with(bytes);
        let palette = Palette::default();
        let input = FrameInput {
            cell: CELL,
            padding: PADDING,
            palette: &palette,
            focused,
        };
        let mut keys = Vec::new();
        let quads = build_frame(&term, &input, &mut |key| {
            keys.push(key);
            Ok(Some(GLYPH))
        })
        .unwrap();
        (quads, keys)
    }

    fn color(named: NamedColor) -> [f32; 4] {
        linear(Palette::default().get(named as usize, &Colors::default()))
    }

    fn glyphs(quads: &[Instance]) -> Vec<&Instance> {
        quads.iter().filter(|q| q.kind == KIND_GLYPH).collect()
    }

    fn solids(quads: &[Instance]) -> Vec<&Instance> {
        quads.iter().filter(|q| q.kind == KIND_SOLID).collect()
    }

    /// Cell rect in pixels.
    fn cell_rect(col: f32, row: f32) -> [f32; 4] {
        [
            PADDING + col * CELL.width,
            PADDING + row * CELL.height,
            CELL.width,
            CELL.height,
        ]
    }

    #[test]
    fn text_becomes_glyphs_at_the_right_place() {
        let (quads, keys) = frame(b"ab", true);
        let chars: Vec<char> = keys.iter().map(|k| k.c).collect();
        assert_eq!(chars, ['a', 'b']);
        let g = glyphs(&quads);
        assert_eq!(g.len(), 2);
        // x = padding + col * width + left; y = padding + row * height + baseline - top.
        assert_eq!(g[0].rect, [4.0 + 1.0, 4.0 + 15.0 - 11.0, 8.0, 12.0]);
        assert_eq!(g[1].rect, [4.0 + 10.0 + 1.0, 4.0 + 15.0 - 11.0, 8.0, 12.0]);
        assert_eq!(g[0].uv, [3.0, 5.0, 8.0, 12.0]);
        assert_eq!(g[0].color, color(NamedColor::Foreground));
    }

    #[test]
    fn spaces_are_not_looked_up() {
        let (_, keys) = frame(b"a  b", true);
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn default_background_has_no_rects() {
        let (quads, _) = frame(b"ab", true);
        // Only the cursor is a solid quad.
        assert_eq!(solids(&quads).len(), 1);
    }

    #[test]
    fn colored_background_is_a_rect() {
        let (quads, _) = frame(b"\x1b[41mX", true);
        let bg = solids(&quads)
            .into_iter()
            .find(|q| q.rect == cell_rect(0.0, 0.0))
            .expect("no bg rect");
        assert_eq!(bg.color, color(NamedColor::Red));
    }

    #[test]
    fn truecolor_text() {
        let (quads, _) = frame(b"\x1b[38;2;255;0;0mX", true);
        assert_eq!(glyphs(&quads)[0].color, linear(Rgb { r: 255, g: 0, b: 0 }));
    }

    #[test]
    fn bold_and_italic_go_to_the_glyph_key() {
        let (_, keys) = frame(b"\x1b[1;3mX", true);
        assert_eq!(
            keys[0],
            GlyphKey {
                c: 'X',
                bold: true,
                italic: true
            }
        );
    }

    #[test]
    fn underline_is_a_thin_rect_at_the_bottom() {
        let (quads, _) = frame(b"\x1b[4mX", true);
        let line = solids(&quads)
            .into_iter()
            .find(|q| {
                q.rect[2] == CELL.width && q.rect[3] < CELL.height / 4.0 && q.rect[0] == PADDING
            })
            .expect("no underline");
        assert!(line.rect[1] > PADDING + CELL.baseline - 1.0);
        assert!(line.rect[1] + line.rect[3] <= PADDING + CELL.height);
    }

    #[test]
    fn block_cursor_is_on_the_cursor_cell() {
        let (quads, _) = frame(b"ab", true);
        let cursor = solids(&quads)[0];
        assert_eq!(cursor.rect, cell_rect(2.0, 0.0));
        assert_eq!(cursor.color, color(NamedColor::Cursor));
    }

    #[test]
    fn char_under_block_cursor_uses_the_bg_color() {
        // ESC [ D moves the cursor back onto "b".
        let (quads, _) = frame(b"ab\x1b[D", true);
        let g = glyphs(&quads);
        assert_eq!(g[1].color, color(NamedColor::Background));
        assert_eq!(g[0].color, color(NamedColor::Foreground));
    }

    #[test]
    fn cursor_is_hollow_without_focus() {
        let (quads, _) = frame(b"ab", false);
        let s = solids(&quads);
        // Four thin lines around the cell.
        assert_eq!(s.len(), 4);
        let cell = cell_rect(2.0, 0.0);
        for q in s {
            assert!(q.rect[0] >= cell[0] && q.rect[0] + q.rect[2] <= cell[0] + cell[2]);
            assert!(q.rect[1] >= cell[1] && q.rect[1] + q.rect[3] <= cell[1] + cell[3]);
            assert!(q.rect[2] < cell[2] / 2.0 || q.rect[3] < cell[3] / 2.0);
        }
    }

    #[test]
    fn hidden_cursor_is_not_drawn() {
        let (quads, _) = frame(b"ab\x1b[?25l", true);
        assert!(solids(&quads).is_empty());
    }

    #[test]
    fn beam_cursor_is_a_thin_line_on_the_left() {
        // DECSCUSR 5 = blinking beam.
        let (quads, _) = frame(b"ab\x1b[5 q", true);
        let s = solids(&quads);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].rect[0], cell_rect(2.0, 0.0)[0]);
        assert!(s[0].rect[2] < CELL.width / 2.0);
    }

    #[test]
    fn wide_char_takes_two_cells_and_the_spacer_is_skipped() {
        let (quads, keys) = frame("\x1b[44m界".as_bytes(), true);
        assert_eq!(keys.len(), 1);
        let bg = solids(&quads)
            .into_iter()
            .find(|q| q.color == color(NamedColor::Blue))
            .expect("no bg");
        assert_eq!(bg.rect[2], 2.0 * CELL.width);
    }

    #[test]
    fn full_atlas_stops_the_frame() {
        let term = term_with(b"ab");
        let palette = Palette::default();
        let input = FrameInput {
            cell: CELL,
            padding: PADDING,
            palette: &palette,
            focused: true,
        };
        let got = build_frame(&term, &input, &mut |_| Err(AtlasFull));
        assert_eq!(got, Err(AtlasFull));
    }
}
