//! A message box in the middle of the window (for example: "Close the tab? claude is running").

use crate::atlas::{AtlasFull, AtlasGlyph, GlyphKey};
use crate::font::CellMetrics;
use crate::frame::{Instance, KIND_SOLID, Rect};
use crate::tabbar::{char_cells, fit_title, push_text, solid};
use crate::theme::UiColors;

/// How dark the window gets behind the box (0 = not at all, 1 = black).
pub const DIM: f32 = 0.5;
/// The widest palette, in cells.
pub const PALETTE_CELLS: usize = 80;

/// What the command palette shows.
pub struct PaletteView<'a> {
    pub query: &'a str,
    /// (label, key hint, is it selected).
    pub rows: &'a [(String, String, bool)],
    /// Short text on the right of the input line (for example "Commands · this folder").
    pub title: &'a str,
    /// Key hints under the rows. Empty = no line.
    pub footer: &'a str,
    /// Rows whose hint is red (by the row number). Empty = none.
    pub bad: &'a [bool],
}

/// The quads of the command palette, at the top of `view`.
pub fn build_palette(
    palette: &PaletteView,
    view: Rect,
    ui: &UiColors,
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
    // The input line, a line of space, the rows, the footer, and half a line at the bottom.
    let footer_lines = if palette.footer.is_empty() { 0.0 } else { 1.0 };
    let height = (2.5 + palette.rows.len() as f32 + footer_lines) * cell.height;
    quads.push(solid(Rect::new(x, y, width, height), ui.overlay));
    for border in [
        Rect::new(x, y, width, 1.0),
        Rect::new(x, y + height - 1.0, width, 1.0),
        Rect::new(x, y, 1.0, height),
        Rect::new(x + width - 1.0, y, 1.0, height),
    ] {
        quads.push(solid(border, ui.accent));
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
        ui.text,
        glyph,
    )?;
    quads.push(solid(
        Rect::new(
            x + (1 + used) as f32 * cell.width,
            input_y,
            2.0,
            cell.height,
        ),
        ui.text,
    ));
    if !palette.title.is_empty() {
        let title_cells: usize = palette.title.chars().map(char_cells).sum();
        let title_x = x + width - (1 + title_cells) as f32 * cell.width;
        push_text(
            &mut quads,
            palette.title,
            title_x,
            input_y,
            cell,
            ui.text_dim,
            glyph,
        )?;
    }
    quads.push(solid(
        Rect::new(x + 1.0, y + cell.height * 1.75, width - 2.0, 1.0),
        ui.accent,
    ));

    for (i, (label, key, selected)) in palette.rows.iter().enumerate() {
        let row_y = y + (2.0 + i as f32) * cell.height;
        if *selected {
            quads.push(solid(
                Rect::new(x + 1.0, row_y, width - 2.0, cell.height),
                ui.selected,
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
            ui.text,
            glyph,
        )?;
        if key_cells > 0 {
            let key_x = x + width - (1 + key_cells) as f32 * cell.width;
            let color = if palette.bad.get(i).copied().unwrap_or(false) {
                ui.error
            } else {
                ui.text_dim
            };
            push_text(&mut quads, key, key_x, row_y, cell, color, glyph)?;
        }
    }
    if !palette.footer.is_empty() {
        let footer_y = y + (2.0 + palette.rows.len() as f32) * cell.height + cell.height * 0.25;
        let shown = fit_title(palette.footer, cells.saturating_sub(2));
        push_text(
            &mut quads,
            &shown,
            x + cell.width,
            footer_y,
            cell,
            ui.text_dim,
            glyph,
        )?;
    }
    Ok(quads)
}

/// Grey text at (`x`, `y`), at most `max_cells` wide (the rest of the line).
pub fn build_ghost(
    text: &str,
    x: f32,
    y: f32,
    max_cells: usize,
    ui: &UiColors,
    cell: CellMetrics,
    glyph: &mut dyn FnMut(&GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
) -> Result<Vec<Instance>, AtlasFull> {
    let mut used = 0;
    let shown: String = text
        .chars()
        .take_while(|c| {
            used += char_cells(*c);
            used <= max_cells
        })
        .collect();
    let mut quads = Vec::new();
    push_text(&mut quads, &shown, x, y, cell, ui.text_ghost, glyph)?;
    Ok(quads)
}

/// The quads of a message box with `lines` of text, in the middle of `view`.
pub fn build_message_box(
    lines: &[String],
    view: Rect,
    ui: &UiColors,
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
    // At most as wide as the view: longer lines are cut (with "…").
    let max_cells = ((view.width / cell.width) as usize).saturating_sub(6);
    let lines: Vec<String> = lines.iter().map(|l| fit_title(l, max_cells)).collect();
    let longest = lines
        .iter()
        .map(|l| l.chars().map(char_cells).sum::<usize>())
        .max()
        .unwrap_or(0);
    let width = (longest + 4) as f32 * cell.width;
    let height = (lines.len() + 2) as f32 * cell.height;
    let x = (view.x + (view.width - width) / 2.0).round();
    let y = (view.y + (view.height - height) / 2.0).round();
    quads.push(solid(Rect::new(x, y, width, height), ui.overlay));
    for border in [
        Rect::new(x, y, width, 1.0),
        Rect::new(x, y + height - 1.0, width, 1.0),
        Rect::new(x, y, 1.0, height),
        Rect::new(x + width - 1.0, y, 1.0, height),
    ] {
        quads.push(solid(border, ui.accent));
    }
    for (i, line) in lines.iter().enumerate() {
        let line_y = y + (i + 1) as f32 * cell.height;
        push_text(
            &mut quads,
            line,
            x + 2.0 * cell.width,
            line_y,
            cell,
            ui.text,
            glyph,
        )?;
    }
    Ok(quads)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> UiColors {
        UiColors::default()
    }
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
        build_message_box(&lines, VIEW, &ui(), CELL, &mut |_| Ok(Some(GLYPH))).unwrap()
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
            .find(|q| q.kind == KIND_SOLID && q.color == linear(ui().overlay))
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
            .filter(|q| q.kind == KIND_SOLID && q.color == linear(ui().accent))
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
        let view = PaletteView {
            query,
            rows: &rows,
            title: "",
            footer: "",
            bad: &[],
        };
        build_palette(&view, VIEW, &ui(), CELL, &mut |_| Ok(Some(GLYPH))).unwrap()
    }

    fn box_height(quads: &[Instance]) -> f32 {
        quads
            .iter()
            .find(|q| q.kind == KIND_SOLID && q.color == linear(ui().overlay))
            .expect("no box")
            .rect[3]
    }

    #[test]
    fn ghost_text_is_grey_and_stops_at_the_line_end() {
        let quads = build_ghost(" status", 100.0, 40.0, 4, &ui(), CELL, &mut |_| {
            Ok(Some(GLYPH))
        })
        .unwrap();
        // " sta" fits in 4 cells; the space is not drawn.
        assert_eq!(quads.len(), 3);
        assert!(
            quads
                .iter()
                .all(|q| q.kind == KIND_GLYPH && q.color == linear(ui().text_ghost))
        );
        assert!(quads[0].rect[0] >= 100.0 + CELL.width, "after the space");
        assert!(quads.iter().all(|q| q.rect[0] < 100.0 + 4.0 * CELL.width));
        assert!(
            build_ghost("abc", 0.0, 0.0, 0, &ui(), CELL, &mut |_| Ok(Some(GLYPH)))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn palette_title_footer_and_bad_hints() {
        let rows = vec![
            ("make".to_owned(), "exit 2".to_owned(), true),
            ("ls".to_owned(), "now".to_owned(), false),
        ];
        let plain = PaletteView {
            query: "",
            rows: &rows,
            title: "",
            footer: "",
            bad: &[],
        };
        let full = PaletteView {
            query: "",
            rows: &rows,
            title: "Commands",
            footer: "Enter put",
            bad: &[true, false],
        };
        let build = |v: &PaletteView| {
            build_palette(v, VIEW, &ui(), CELL, &mut |_| Ok(Some(GLYPH))).unwrap()
        };
        let (a, b) = (build(&plain), build(&full));
        assert_eq!(
            box_height(&b),
            box_height(&a) + CELL.height,
            "one more line for the footer"
        );
        let glyphs = |q: &[Instance]| q.iter().filter(|q| q.kind == KIND_GLYPH).count();
        // "Commands" (8) + "Enter put" (8, the space is not drawn).
        assert_eq!(glyphs(&b), glyphs(&a) + 8 + 8);
        let red = linear(ui().error);
        let red_glyphs = b
            .iter()
            .filter(|q| q.kind == KIND_GLYPH && q.color == red)
            .count();
        assert_eq!(red_glyphs, 5, "`exit 2` is red (5 glyphs, no space)");
        assert!(!a.iter().any(|q| q.kind == KIND_GLYPH && q.color == red));
    }

    #[test]
    fn palette_box_is_centered_at_the_top() {
        let quads = palette("", &[("New tab", "", true)]);
        let bg = quads
            .iter()
            .find(|q| q.kind == KIND_SOLID && q.color == linear(ui().overlay))
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
            .filter(|q| q.kind == KIND_SOLID && q.color == linear(ui().selected))
            .collect();
        assert_eq!(selected.len(), 1);
        // Text: "> ne" (3 glyphs, the space is not drawn) + "New tab" (6) + the key (12) + "Next tab" (7).
        let glyphs = quads.iter().filter(|q| q.kind == KIND_GLYPH).count();
        assert_eq!(glyphs, 3 + 6 + 12 + 7);
    }

    #[test]
    fn a_message_box_with_a_long_line_stays_on_the_screen() {
        let view = Rect::new(0.0, 0.0, 400.0, 300.0);
        let lines = vec!["Short".to_owned(), "x".repeat(500)];
        let quads = build_message_box(&lines, view, &ui(), CELL, &mut |_| Ok(Some(GLYPH))).unwrap();
        for q in quads.iter().skip(1) {
            assert!(q.rect[0] >= view.x, "{:?}", q.rect);
            assert!(
                q.rect[0] + q.rect[2] <= view.x + view.width + 0.5,
                "{:?}",
                q.rect
            );
        }
    }

    #[test]
    fn key_hints_are_on_the_right() {
        let quads = palette("", &[("A", "K", false)]);
        let bg = quads
            .iter()
            .find(|q| q.kind == KIND_SOLID && q.color == linear(ui().overlay))
            .unwrap()
            .rect;
        let glyphs: Vec<&Instance> = quads.iter().filter(|q| q.kind == KIND_GLYPH).collect();
        let hint = glyphs
            .iter()
            .find(|g| g.color == linear(ui().text_dim))
            .expect("no hint");
        assert!(
            hint.rect[0] > bg[0] + bg[2] / 2.0,
            "the hint is on the right side"
        );
    }
}
