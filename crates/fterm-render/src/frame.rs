//! Builds one frame: the terminal grid becomes a list of quads for the GPU.
//! This part has no GPU code, so we can test it with a real `Term`.

use bytemuck::{Pod, Zeroable};
use fterm_term::alacritty_terminal::event::EventListener;
use fterm_term::alacritty_terminal::grid::Dimensions;
use fterm_term::alacritty_terminal::term::cell::Flags;
use fterm_term::alacritty_terminal::term::{Term, TermMode};
use fterm_term::alacritty_terminal::vte::ansi::{CursorShape, NamedColor};
use fterm_term::colors::{Palette, cell_colors};

use crate::atlas::{AtlasFull, AtlasGlyph, GlyphKey};
use crate::color::linear;
use crate::font::CellMetrics;
use crate::theme::UiColors;
pub use fterm_mux::Rect;

/// A filled rectangle (background, cursor, underline).
pub const KIND_SOLID: u32 = 0;
/// A glyph: alpha from the atlas times `color`.
pub const KIND_GLYPH: u32 = 1;
/// A color glyph (emoji): RGBA from the color atlas.
pub const KIND_COLOR_GLYPH: u32 = 2;

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
    pub ui: &'a UiColors,
    pub focused: bool,
    /// Where the pane is in the window, in pixels. The padding is inside it.
    pub area: Rect,
}

/// Width of the scroll indicator in pixels.
pub const SCROLL_INDICATOR_WIDTH: f32 = 4.0;

/// Makes all quads for the frame. `glyph` finds a glyph in the atlas (or adds it).
pub fn build_frame<T: EventListener>(
    term: &Term<T>,
    input: &FrameInput,
    glyph: &mut impl FnMut(&GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
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
            input.area.x + input.padding + col as f32 * cell.width,
            input.area.y + input.padding + row as f32 * cell.height,
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
    let selection = content.selection;
    // The copy mode cursor (vi mode of alacritty), in screen rows.
    let copy_cursor = content.mode.contains(TermMode::VI).then(|| {
        let point = term.vi_mode_cursor.point;
        (point.line.0 + offset, point.column.0)
    });

    let mut backgrounds = Vec::new();
    // The app changed the background color (OSC 11): paint the whole pane with it.
    if overrides[NamedColor::Background as usize].is_some() {
        let a = input.area;
        backgrounds.push(solid([a.x, a.y, a.width, a.height], linear(default_bg)));
    }
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
        let under_copy_cursor = copy_cursor == Some((row, col));
        let under_block_cursor = under_copy_cursor
            || (cursor_shape == CursorShape::Block && row == cursor_row && col == cursor_col);
        if row == cursor_row && col == cursor_col {
            cursor_width = width;
        }

        if selection.is_some_and(|range| range.contains(indexed.point)) {
            backgrounds.push(solid(
                [x, y, width, cell.height],
                linear(input.palette.selection),
            ));
        } else if bg != default_bg {
            backgrounds.push(solid([x, y, width, cell.height], linear(bg)));
        }

        let c = indexed.cell.c;
        if c != ' ' && c != '\t' {
            let key = GlyphKey {
                c,
                extra: indexed.cell.zerowidth().map(Box::from),
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
                wide: flags.contains(Flags::WIDE_CHAR),
            };
            if let Some(g) = glyph(&key)? {
                let color = if under_block_cursor { bg } else { fg };
                let kind = if g.color {
                    KIND_COLOR_GLYPH
                } else {
                    KIND_GLYPH
                };
                glyphs.push(Instance {
                    rect: [
                        x + g.left as f32,
                        y + cell.baseline - g.top as f32,
                        g.width as f32,
                        g.height as f32,
                    ],
                    uv: [g.x as f32, g.y as f32, g.width as f32, g.height as f32],
                    color: linear(color),
                    kind,
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
    if let Some((row, col)) = copy_cursor {
        let (x, y) = origin(col, row);
        cursor_back.push(solid([x, y, cell.width, h], linear(input.ui.copy_cursor)));
    }
    if let Some(indicator) = scroll_indicator(term, content.display_offset, input.area, input.ui) {
        cursor_front.push(indicator);
    }

    let mut quads = backgrounds;
    quads.extend(cursor_back);
    quads.extend(glyphs);
    quads.extend(lines);
    quads.extend(cursor_front);
    Ok(quads)
}

/// A thin bar on the right edge that shows where the view is in the history.
/// Only when the view is scrolled up.
fn scroll_indicator<T: EventListener>(
    term: &Term<T>,
    display_offset: usize,
    area: Rect,
    ui: &UiColors,
) -> Option<Instance> {
    let (view_width, view_height) = (area.width, area.height);
    if display_offset == 0 {
        return None;
    }
    let history = term.history_size() as f32;
    let total = history + term.screen_lines() as f32;
    let height = (view_height * term.screen_lines() as f32 / total)
        .max(12.0)
        .min(view_height);
    let top = (view_height * (history - display_offset as f32) / total).min(view_height - height);
    Some(solid(
        [
            area.x + view_width - SCROLL_INDICATOR_WIDTH,
            area.y + top.max(0.0),
            SCROLL_INDICATOR_WIDTH,
            height,
        ],
        linear(ui.scrollbar),
    ))
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
    use fterm_term::alacritty_terminal::grid::Scroll;
    use fterm_term::alacritty_terminal::index::{Column, Line, Point, Side};
    use fterm_term::alacritty_terminal::selection::{Selection, SelectionType};
    use fterm_term::alacritty_terminal::term::Config;
    use fterm_term::alacritty_terminal::term::color::Colors;
    use fterm_term::alacritty_terminal::vte::ansi::Processor;
    use fterm_term::alacritty_terminal::vte::ansi::Rgb;
    use fterm_term::size::GridSize;

    use super::*;

    const CELL: CellMetrics = CellMetrics {
        width: 10.0,
        height: 20.0,
        baseline: 15.0,
    };
    const PADDING: f32 = 4.0;
    /// 10x3 cells + padding, at the top-left of the window.
    const AREA: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 108.0,
        height: 68.0,
    };
    /// Every glyph in the fake atlas has this place and size.
    const GLYPH: AtlasGlyph = AtlasGlyph {
        x: 3,
        y: 5,
        width: 8,
        height: 12,
        left: 1,
        top: 11,
        color: false,
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
            ui: &UiColors::default(),
            focused,
            area: AREA,
        };
        let mut keys = Vec::new();
        let quads = build_frame(&term, &input, &mut |key| {
            keys.push(key.clone());
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
                extra: None,
                bold: true,
                italic: true,
                wide: false,
            }
        );
    }

    #[test]
    fn combining_mark_goes_into_the_key() {
        let (quads, keys) = frame("e\u{0301}x".as_bytes(), true);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].c, 'e');
        assert_eq!(keys[0].extra.as_deref(), Some(&['\u{0301}'][..]));
        assert_eq!(keys[1].extra, None);
        assert_eq!(keys[0].text(), "e\u{0301}");
        assert_eq!(glyphs(&quads).len(), 2);
    }

    #[test]
    fn wide_char_key_is_wide() {
        let (_, keys) = frame("a界".as_bytes(), true);
        assert!(!keys[0].wide);
        assert!(keys[1].wide);
    }

    #[test]
    fn zwj_emoji_are_separate_cells_in_alacritty() {
        // alacritty keeps ZWJ (width 0) in the previous cell, but the next emoji gets its own
        // wide cell. So a ZWJ family is drawn as separate emoji. A grapheme mode can fix this later.
        let (_, keys) = frame("👨\u{200d}👩".as_bytes(), true);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].text(), "👨\u{200d}");
        assert!(keys[0].wide && keys[1].wide);
    }

    #[test]
    fn color_glyph_is_a_color_quad() {
        let term = term_with("A😀".as_bytes());
        let palette = Palette::default();
        let input = FrameInput {
            cell: CELL,
            padding: PADDING,
            palette: &palette,
            ui: &UiColors::default(),
            focused: true,
            area: AREA,
        };
        let quads = build_frame(&term, &input, &mut |key| {
            Ok(Some(AtlasGlyph {
                color: key.c == '😀',
                ..GLYPH
            }))
        })
        .unwrap();
        let kinds: Vec<u32> = quads
            .iter()
            .filter(|q| q.kind != KIND_SOLID)
            .map(|q| q.kind)
            .collect();
        assert_eq!(kinds, [KIND_GLYPH, KIND_COLOR_GLYPH]);
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
            ui: &UiColors::default(),
            focused: true,
            area: AREA,
        };
        let got = build_frame(&term, &input, &mut |_| Err(AtlasFull));
        assert_eq!(got, Err(AtlasFull));
    }

    fn build(term: &Term<VoidListener>) -> Vec<Instance> {
        let palette = Palette::default();
        let input = FrameInput {
            cell: CELL,
            padding: PADDING,
            palette: &palette,
            ui: &UiColors::default(),
            focused: true,
            area: AREA,
        };
        build_frame(term, &input, &mut |_| Ok(Some(GLYPH))).unwrap()
    }

    fn select(term: &mut Term<VoidListener>, line: i32, from: usize, to: usize) {
        let mut selection = Selection::new(
            SelectionType::Simple,
            Point::new(Line(line), Column(from)),
            Side::Left,
        );
        selection.update(Point::new(Line(line), Column(to)), Side::Right);
        term.selection = Some(selection);
    }

    fn rects_with_color(quads: &[Instance], color: Rgb) -> Vec<[f32; 4]> {
        quads
            .iter()
            .filter(|q| q.kind == KIND_SOLID && q.color == linear(color))
            .map(|q| q.rect)
            .collect()
    }

    #[test]
    fn selected_cells_get_the_selection_background() {
        let mut term = term_with(b"abcd");
        select(&mut term, 0, 1, 2);
        let rects = rects_with_color(&build(&term), Palette::default().selection);
        assert_eq!(rects, [cell_rect(1.0, 0.0), cell_rect(2.0, 0.0)]);
    }

    #[test]
    fn empty_cells_can_be_selected_too() {
        let mut term = term_with(b"ab");
        select(&mut term, 1, 4, 4);
        let rects = rects_with_color(&build(&term), Palette::default().selection);
        assert_eq!(rects, [cell_rect(4.0, 1.0)]);
    }

    #[test]
    fn selected_text_keeps_its_color() {
        let mut term = term_with(b"ab");
        select(&mut term, 0, 0, 1);
        let quads = build(&term);
        assert!(
            glyphs(&quads)
                .iter()
                .all(|g| g.color == color(NamedColor::Foreground))
        );
    }

    #[test]
    fn selection_scrolls_with_the_view() {
        // 10 lines in a 3-row terminal, scrolled up 2 lines: history line -2 is the top row.
        let text: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let mut term = term_with(text.join("\r\n").as_bytes());
        term.scroll_display(Scroll::Delta(2));
        select(&mut term, -2, 0, 0);
        let rects = rects_with_color(&build(&term), Palette::default().selection);
        assert_eq!(rects, [cell_rect(0.0, 0.0)]);
    }

    #[test]
    fn copy_mode_cursor_is_a_yellow_block() {
        let mut term = term_with(b"abc");
        term.toggle_vi_mode();
        let rects = rects_with_color(&build(&term), UiColors::default().copy_cursor);
        // The copy mode cursor starts at the shell cursor (after "abc").
        assert_eq!(rects, [cell_rect(3.0, 0.0)]);
    }

    #[test]
    fn scroll_indicator_shows_only_when_scrolled_up() {
        let text: Vec<String> = (0..30).map(|i| i.to_string()).collect();
        let mut term = term_with(text.join("\r\n").as_bytes());
        assert!(rects_with_color(&build(&term), UiColors::default().scrollbar).is_empty());

        term.scroll_display(Scroll::Delta(5));
        let rects = rects_with_color(&build(&term), UiColors::default().scrollbar);
        assert_eq!(rects.len(), 1);
        let [x, y, w, h] = rects[0];
        assert_eq!(x + w, AREA.width, "at the right edge");
        assert_eq!(w, SCROLL_INDICATOR_WIDTH);
        assert!(
            y >= 0.0 && y + h <= AREA.height && h >= 4.0,
            "{:?}",
            rects[0]
        );
    }

    #[test]
    fn scroll_indicator_is_at_the_top_when_at_the_top() {
        let text: Vec<String> = (0..30).map(|i| i.to_string()).collect();
        let mut term = term_with(text.join("\r\n").as_bytes());
        term.scroll_display(Scroll::Top);
        let rects = rects_with_color(&build(&term), UiColors::default().scrollbar);
        assert_eq!(rects[0][1], 0.0);
    }

    #[test]
    fn pane_at_an_offset_moves_everything() {
        let term = term_with(b"ab");
        let palette = Palette::default();
        let area = Rect::new(200.0, 30.0, 108.0, 68.0);
        let input = FrameInput {
            cell: CELL,
            padding: PADDING,
            palette: &palette,
            ui: &UiColors::default(),
            focused: true,
            area,
        };
        let moved = build_frame(&term, &input, &mut |_| Ok(Some(GLYPH))).unwrap();
        let at_origin = build(&term);
        assert_eq!(moved.len(), at_origin.len());
        for (m, o) in moved.iter().zip(&at_origin) {
            assert_eq!(m.rect[0], o.rect[0] + 200.0);
            assert_eq!(m.rect[1], o.rect[1] + 30.0);
        }
    }

    #[test]
    fn scroll_indicator_is_at_the_right_edge_of_the_pane() {
        let text: Vec<String> = (0..30).map(|i| i.to_string()).collect();
        let mut term = term_with(text.join("\r\n").as_bytes());
        term.scroll_display(Scroll::Delta(5));
        let palette = Palette::default();
        let area = Rect::new(200.0, 30.0, 108.0, 68.0);
        let input = FrameInput {
            cell: CELL,
            padding: PADDING,
            palette: &palette,
            ui: &UiColors::default(),
            focused: true,
            area,
        };
        let quads = build_frame(&term, &input, &mut |_| Ok(Some(GLYPH))).unwrap();
        let rects = rects_with_color(&quads, UiColors::default().scrollbar);
        let [x, y, w, h] = rects[0];
        assert_eq!(x + w, 308.0);
        assert!(y >= 30.0 && y + h <= 98.0);
    }

    #[test]
    fn pane_with_an_app_background_fills_its_area() {
        // OSC 11 sets the background color: the pane paints its whole area with it.
        let term = term_with(b"\x1b]11;rgb:10/20/30\x07x");
        let quads = build(&term);
        let fill = rects_with_color(
            &quads,
            Rgb {
                r: 0x10,
                g: 0x20,
                b: 0x30,
            },
        );
        assert_eq!(fill.first(), Some(&[0.0, 0.0, 108.0, 68.0]));
    }

    #[test]
    fn a_custom_theme_colors_the_scroll_indicator() {
        let text: Vec<String> = (0..30).map(|i| i.to_string()).collect();
        let mut term = term_with(
            text.join(
                "
",
            )
            .as_bytes(),
        );
        term.scroll_display(Scroll::Delta(5));
        let palette = Palette::default();
        let ui = UiColors {
            scrollbar: Rgb { r: 1, g: 2, b: 3 },
            ..UiColors::default()
        };
        let input = FrameInput {
            cell: CELL,
            padding: PADDING,
            palette: &palette,
            ui: &ui,
            focused: true,
            area: AREA,
        };
        let quads = build_frame(&term, &input, &mut |_| Ok(Some(GLYPH))).unwrap();
        assert_eq!(rects_with_color(&quads, ui.scrollbar).len(), 1);
        assert!(rects_with_color(&quads, UiColors::default().scrollbar).is_empty());
    }
}
