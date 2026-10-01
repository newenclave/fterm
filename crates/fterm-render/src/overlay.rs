//! A message box in the middle of the window (for example: "Close the tab? claude is running").

use fterm_term::alacritty_terminal::vte::ansi::Rgb;

use crate::atlas::{AtlasFull, AtlasGlyph, GlyphKey};
use crate::font::CellMetrics;
use crate::frame::{Instance, KIND_SOLID, Rect};
use crate::tabbar::{char_cells, push_text, solid};

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
}
