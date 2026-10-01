//! A message box in the middle of the window (for example: "Close the tab? claude is running").

use fterm_term::alacritty_terminal::vte::ansi::Rgb;

use crate::atlas::{AtlasFull, AtlasGlyph, GlyphKey};
use crate::font::CellMetrics;
use crate::frame::{Instance, KIND_SOLID, Rect};
use crate::tabbar::{char_cells, fit_title, push_text, solid};

/// How dark the window gets behind the box (0 = not at all, 1 = black).
pub const DIM: f32 = 0.5;
pub const BOX_BG: Rgb = Rgb {
    r: 0x31,
    g: 0x32,
    b: 0x44,
};
pub const BOX_BORDER: Rgb = Rgb {
    r: 0xcb,
    g: 0xa6,
    b: 0xf7,
};
pub const BOX_TEXT: Rgb = Rgb {
    r: 0xcd,
    g: 0xd6,
    b: 0xf4,
};
/// The selected line of the palette.
pub const ROW_SELECTED: Rgb = Rgb {
    r: 0x45,
    g: 0x47,
    b: 0x5a,
};
/// Key hints and other quiet text.
pub const HINT_TEXT: Rgb = Rgb {
    r: 0x93,
    g: 0x99,
    b: 0xb2,
};
/// The widest palette, in cells.
pub const PALETTE_CELLS: usize = 80;

/// What the command palette shows.
pub struct PaletteView<'a> {
    pub query: &'a str,
    /// (label, key hint, is it selected).
    pub rows: &'a [(String, String, bool)],
}

/// The quads of the command palette, at the top of `view`.
pub fn build_palette(
    palette: &PaletteView,
    view: Rect,
    cell: CellMetrics,
    glyph: &mut dyn FnMut(&GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
) -> Result<Vec<Instance>, AtlasFull> {
    let mut quads = vec![Instance {
        rect: [view.x, view.y, view.width, view.height],
        uv: [0.0; 4],
        color: [0.0, 0.0, 0.0, DIM * 0.6],
        kind: KIND_SOLID,
        _pad: [0; 3],
    }];
    let cells = PALETTE_CELLS
        .min(((view.width / cell.width) as usize).saturating_sub(4))
        .max(10);
    let width = cells as f32 * cell.width;
    let x = (view.x + (view.width - width) / 2.0).round();
    let y = (view.y + 2.0 * cell.height).round();
    // The input line, a line of space, the rows, and half a line at the bottom.
    let height = (2.5 + palette.rows.len() as f32) * cell.height;
    quads.push(solid(Rect::new(x, y, width, height), BOX_BG));
    for border in [
        Rect::new(x, y, width, 1.0),
        Rect::new(x, y + height - 1.0, width, 1.0),
        Rect::new(x, y, 1.0, height),
        Rect::new(x + width - 1.0, y, 1.0, height),
    ] {
        quads.push(solid(border, BOX_BORDER));
    }

    // The input line: "> query" and a text cursor.
    let input_y = y + cell.height * 0.5;
    let prompt = format!("> {}", palette.query);
    let used = push_text(
        &mut quads,
        &prompt,
        x + cell.width,
        input_y,
        cell,
        BOX_TEXT,
        glyph,
    )?;
    quads.push(solid(
        Rect::new(
            x + (1 + used) as f32 * cell.width,
            input_y,
            2.0,
            cell.height,
        ),
        BOX_TEXT,
    ));
    quads.push(solid(
        Rect::new(x + 1.0, y + cell.height * 1.75, width - 2.0, 1.0),
        BOX_BORDER,
    ));

    for (i, (label, key, selected)) in palette.rows.iter().enumerate() {
        let row_y = y + (2.0 + i as f32) * cell.height;
        if *selected {
            quads.push(solid(
                Rect::new(x + 1.0, row_y, width - 2.0, cell.height),
                ROW_SELECTED,
            ));
        }
        let key_cells: usize = key.chars().map(char_cells).sum();
        let label_cells = cells.saturating_sub(4 + key_cells);
        let label = fit_title(label, label_cells);
        push_text(
            &mut quads,
            &label,
            x + cell.width,
            row_y,
            cell,
            BOX_TEXT,
            glyph,
        )?;
        if key_cells > 0 {
            let key_x = x + width - (1 + key_cells) as f32 * cell.width;
            push_text(&mut quads, key, key_x, row_y, cell, HINT_TEXT, glyph)?;
        }
    }
    Ok(quads)
}

/// The quads of a message box with `lines` of text, in the middle of `view`.
pub fn build_message_box(
    lines: &[String],
    view: Rect,
    cell: CellMetrics,
    glyph: &mut dyn FnMut(&GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
) -> Result<Vec<Instance>, AtlasFull> {
    let mut quads = vec![Instance {
        rect: [view.x, view.y, view.width, view.height],
        uv: [0.0; 4],
        color: [0.0, 0.0, 0.0, DIM],
        kind: KIND_SOLID,
        _pad: [0; 3],
    }];
    let longest = lines
        .iter()
        .map(|l| l.chars().map(char_cells).sum::<usize>())
        .max()
        .unwrap_or(0);
    let width = (longest + 4) as f32 * cell.width;
    let height = (lines.len() + 2) as f32 * cell.height;
    let x = (view.x + (view.width - width) / 2.0).round();
    let y = (view.y + (view.height - height) / 2.0).round();
    quads.push(solid(Rect::new(x, y, width, height), BOX_BG));
    for border in [
        Rect::new(x, y, width, 1.0),
        Rect::new(x, y + height - 1.0, width, 1.0),
        Rect::new(x, y, 1.0, height),
        Rect::new(x + width - 1.0, y, 1.0, height),
    ] {
        quads.push(solid(border, BOX_BORDER));
    }
    for (i, line) in lines.iter().enumerate() {
        let line_y = y + (i + 1) as f32 * cell.height;
        push_text(
            &mut quads,
            line,
            x + 2.0 * cell.width,
            line_y,
            cell,
            BOX_TEXT,
            glyph,
        )?;
    }
    Ok(quads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::linear;
    use crate::frame::{KIND_GLYPH, KIND_SOLID};

    const CELL: CellMetrics = CellMetrics {
        width: 10.0,
        height: 20.0,
        baseline: 15.0,
    };
    const VIEW: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };
    const GLYPH: AtlasGlyph = AtlasGlyph {
        x: 0,
        y: 0,
        width: 6,
        height: 10,
        left: 1,
        top: 10,
        color: false,
    };

    fn build(lines: &[&str]) -> Vec<Instance> {
        let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        build_message_box(&lines, VIEW, CELL, &mut |_| Ok(Some(GLYPH))).unwrap()
    }

    #[test]
    fn the_window_gets_darker_behind_the_box() {
        let quads = build(&["Close?"]);
        let dim = quads[0];
        assert_eq!(dim.kind, KIND_SOLID);
        assert_eq!(dim.rect, [0.0, 0.0, 800.0, 600.0]);
        assert_eq!(dim.color, [0.0, 0.0, 0.0, DIM]);
    }

    #[test]
    fn the_box_is_in_the_middle_and_fits_the_longest_line() {
        let quads = build(&["Close the tab?", "Enter = yes"]);
        let bg = quads
            .iter()
            .find(|q| q.kind == KIND_SOLID && q.color == linear(BOX_BG))
            .expect("no box");
        let [x, y, w, h] = bg.rect;
        assert!(
            (x + w / 2.0 - 400.0).abs() <= 1.0,
            "centered: {:?}",
            bg.rect
        );
        assert!(
            (y + h / 2.0 - 300.0).abs() <= 1.0,
            "centered: {:?}",
            bg.rect
        );
        // 14 chars + 2 cells of space on each side.
        assert_eq!(w, 18.0 * CELL.width);
        // 2 lines + 1 line of space above and below.
        assert_eq!(h, 4.0 * CELL.height);
    }

    #[test]
    fn the_box_has_a_border_and_text() {
        let quads = build(&["ab", "c d"]);
        let borders = quads
            .iter()
            .filter(|q| q.kind == KIND_SOLID && q.color == linear(BOX_BORDER))
            .count();
        assert_eq!(borders, 4);
        let glyphs = quads.iter().filter(|q| q.kind == KIND_GLYPH).count();
        assert_eq!(glyphs, 4);
    }

    fn palette(query: &str, rows: &[(&str, &str, bool)]) -> Vec<Instance> {
        let rows: Vec<(String, String, bool)> = rows
            .iter()
            .map(|(l, k, s)| (l.to_string(), k.to_string(), *s))
            .collect();
        let view = PaletteView { query, rows: &rows };
        build_palette(&view, VIEW, CELL, &mut |_| Ok(Some(GLYPH))).unwrap()
    }

    #[test]
    fn palette_box_is_centered_at_the_top() {
        let quads = palette("", &[("New tab", "", true)]);
        let bg = quads
            .iter()
            .find(|q| q.kind == KIND_SOLID && q.color == linear(BOX_BG))
            .expect("no box");
        let [x, y, w, _] = bg.rect;
        assert!(
            (x + w / 2.0 - 400.0).abs() <= 1.0,
            "centered: {:?}",
            bg.rect
        );
        assert!(y < 100.0, "near the top");
        // The window is 80 cells wide, so the box is a bit narrower.
        assert!(w <= 800.0 - 2.0 * CELL.width);
    }

    #[test]
    fn palette_has_one_line_per_row_and_a_highlight() {
        let quads = palette(
            "ne",
            &[("New tab", "Ctrl+Shift+T", true), ("Next tab", "", false)],
        );
        let selected: Vec<&Instance> = quads
            .iter()
            .filter(|q| q.kind == KIND_SOLID && q.color == linear(ROW_SELECTED))
            .collect();
        assert_eq!(selected.len(), 1);
        // Text: "> ne" (3 glyphs, the space is not drawn) + "New tab" (6) + the key (12) + "Next tab" (7).
        let glyphs = quads.iter().filter(|q| q.kind == KIND_GLYPH).count();
        assert_eq!(glyphs, 3 + 6 + 12 + 7);
    }

    #[test]
    fn key_hints_are_on_the_right() {
        let quads = palette("", &[("A", "K", false)]);
        let bg = quads
            .iter()
            .find(|q| q.kind == KIND_SOLID && q.color == linear(BOX_BG))
            .unwrap()
            .rect;
        let glyphs: Vec<&Instance> = quads.iter().filter(|q| q.kind == KIND_GLYPH).collect();
        let hint = glyphs
            .iter()
            .find(|g| g.color == linear(HINT_TEXT))
            .expect("no hint");
        assert!(
            hint.rect[0] > bg[0] + bg[2] / 2.0,
            "the hint is on the right side"
        );
    }
}
