//! Toasts: small notification boxes in a corner of the window. They do not take the focus.

use fterm_term::alacritty_terminal::vte::ansi::Rgb;

use crate::atlas::{AtlasFull, AtlasGlyph, GlyphKey};
use crate::font::CellMetrics;
use crate::frame::{Instance, Rect};
use crate::overlay::{BOX_BG, BOX_TEXT, HINT_TEXT};
use crate::tabbar::{fit_title, push_text, solid};

/// Space between the window edge and the toasts, and between two toasts, in pixels.
pub const MARGIN: f32 = 10.0;
pub const GAP: f32 = 6.0;
/// The widest toast, in cells.
pub const TOAST_CELLS: f32 = 44.0;
/// How solid the toast background is (1 = not see-through).
pub const BG_ALPHA: f32 = 0.94;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    BottomRight,
    TopRight,
    BottomLeft,
    TopLeft,
    /// Bottom, in the middle.
    Bottom,
}

/// The color of a level (the bar on the left of a toast).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
    Attention,
}

impl ToastLevel {
    pub fn color(self) -> Rgb {
        let (r, g, b) = match self {
            ToastLevel::Info => (0x89, 0xb4, 0xfa),
            ToastLevel::Success => (0xa6, 0xe3, 0xa1),
            ToastLevel::Warning => (0xf9, 0xe2, 0xaf),
            ToastLevel::Error => (0xf3, 0x8b, 0xa8),
            ToastLevel::Attention => (0xcb, 0xa6, 0xf7),
        };
        Rgb { r, g, b }
    }
}

pub struct ToastView<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub level: ToastLevel,
    /// The mouse is over it: the `×` shows.
    pub hover: bool,
}

/// The height of one toast: a title line and two body lines, and some space.
pub fn toast_height(cell: CellMetrics) -> f32 {
    3.5 * cell.height
}

/// Places `count` toasts in `area`. Index 0 is the oldest; the newest is next to the window edge.
pub fn layout_toasts(count: usize, area: Rect, corner: Corner, cell: CellMetrics) -> Vec<Rect> {
    let width = (TOAST_CELLS * cell.width)
        .min(area.width - 2.0 * MARGIN)
        .max(cell.width * 4.0);
    let height = toast_height(cell);
    let x = match corner {
        Corner::BottomRight | Corner::TopRight => area.x + area.width - MARGIN - width,
        Corner::BottomLeft | Corner::TopLeft => area.x + MARGIN,
        Corner::Bottom => (area.x + (area.width - width) / 2.0).round(),
    };
    let from_top = matches!(corner, Corner::TopRight | Corner::TopLeft);
    (0..count)
        .map(|i| {
            // `step` 0 is the newest toast, next to the edge.
            let step = (count - 1 - i) as f32;
            let y = if from_top {
                area.y + MARGIN + step * (height + GAP)
            } else {
                area.y + area.height - MARGIN - height - step * (height + GAP)
            };
            Rect::new(x, y, width, height)
        })
        .collect()
}

/// The `×` button of a toast.
pub fn close_rect(toast: Rect, cell: CellMetrics) -> Rect {
    let size = 2.0 * cell.width;
    Rect::new(
        toast.x + toast.width - size,
        toast.y,
        size,
        cell.height * 1.5,
    )
}

/// If the stack covers the text cursor, the stack goes to the other side (top <-> bottom).
pub fn avoid_cursor(corner: Corner, stack: Rect, cursor: Rect) -> Corner {
    let covers = cursor.x < stack.x + stack.width
        && stack.x < cursor.x + cursor.width
        && cursor.y < stack.y + stack.height
        && stack.y < cursor.y + cursor.height;
    if !covers {
        return corner;
    }
    match corner {
        Corner::BottomRight | Corner::Bottom => Corner::TopRight,
        Corner::BottomLeft => Corner::TopLeft,
        Corner::TopRight => Corner::BottomRight,
        Corner::TopLeft => Corner::BottomLeft,
    }
}

/// The quads of all toasts.
pub fn build_toasts(
    toasts: &[ToastView],
    rects: &[Rect],
    cell: CellMetrics,
    glyph: &mut dyn FnMut(&GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
) -> Result<Vec<Instance>, AtlasFull> {
    let mut quads = Vec::new();
    for (toast, rect) in toasts.iter().zip(rects) {
        let mut bg = solid(*rect, BOX_BG);
        bg.color[3] = BG_ALPHA;
        quads.push(bg);
        quads.push(solid(
            Rect::new(rect.x, rect.y, 3.0, rect.height),
            toast.level.color(),
        ));
        let text_x = rect.x + cell.width;
        let cells = ((rect.width / cell.width) as usize).saturating_sub(3);
        let top = rect.y + cell.height * 0.25;
        if !toast.title.is_empty() {
            let title = fit_title(toast.title, cells.saturating_sub(1));
            push_text(&mut quads, &title, text_x, top, cell, BOX_TEXT, glyph)?;
        }
        let (first, second) = wrap_two_lines(toast.body, cells);
        let body_top = if toast.title.is_empty() {
            top
        } else {
            top + cell.height
        };
        push_text(&mut quads, &first, text_x, body_top, cell, HINT_TEXT, glyph)?;
        if toast.title.is_empty() || !second.is_empty() {
            push_text(
                &mut quads,
                &second,
                text_x,
                body_top + cell.height,
                cell,
                HINT_TEXT,
                glyph,
            )?;
        }
        if toast.hover {
            let close = close_rect(*rect, cell);
            push_text(
                &mut quads,
                "×",
                close.x + cell.width * 0.5,
                top,
                cell,
                BOX_TEXT,
                glyph,
            )?;
        }
    }
    Ok(quads)
}

/// Cuts text into two lines of `cells` cells (at a space when it can). The second line ends with `…` if it is cut.
fn wrap_two_lines(text: &str, cells: usize) -> (String, String) {
    let text = text.replace(['\r', '\n'], " ");
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= cells {
        return (text, String::new());
    }
    let cut = chars[..cells]
        .iter()
        .rposition(|c| *c == ' ')
        .filter(|&i| i > cells / 2)
        .unwrap_or(cells);
    let first: String = chars[..cut].iter().collect();
    let rest: String = chars[cut..]
        .iter()
        .collect::<String>()
        .trim_start()
        .to_owned();
    (first, fit_title(&rest, cells))
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
    const AREA: Rect = Rect {
        x: 0.0,
        y: 30.0,
        width: 1000.0,
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

    fn overlap(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
    }

    #[test]
    fn bottom_right_stack() {
        let rects = layout_toasts(3, AREA, Corner::BottomRight, CELL);
        assert_eq!(rects.len(), 3);
        let newest = rects[2];
        assert_eq!(newest.x + newest.width, AREA.x + AREA.width - MARGIN);
        assert_eq!(
            newest.y + newest.height,
            AREA.y + AREA.height - MARGIN,
            "the newest is at the bottom"
        );
        assert!(rects[0].y < rects[1].y && rects[1].y < rects[2].y);
        assert!(!overlap(&rects[0], &rects[1]) && !overlap(&rects[1], &rects[2]));
        assert_eq!(newest.width, TOAST_CELLS * CELL.width);
        assert_eq!(newest.height, toast_height(CELL));
    }

    #[test]
    fn top_left_stack() {
        let rects = layout_toasts(2, AREA, Corner::TopLeft, CELL);
        assert_eq!(rects[1].x, AREA.x + MARGIN);
        assert_eq!(rects[1].y, AREA.y + MARGIN, "the newest is at the top");
        assert!(rects[0].y > rects[1].y);
    }

    #[test]
    fn bottom_middle_stack() {
        let rects = layout_toasts(1, AREA, Corner::Bottom, CELL);
        let r = rects[0];
        assert!((r.x + r.width / 2.0 - 500.0).abs() <= 1.0);
    }

    #[test]
    fn a_small_window_gets_narrow_toasts() {
        let small = Rect::new(0.0, 0.0, 200.0, 300.0);
        let rects = layout_toasts(1, small, Corner::BottomRight, CELL);
        assert!(rects[0].width <= 200.0 - 2.0 * MARGIN);
        assert!(rects[0].x >= MARGIN);
    }

    #[test]
    fn the_close_button_is_at_the_top_right() {
        let toast = Rect::new(100.0, 100.0, 440.0, 70.0);
        let close = close_rect(toast, CELL);
        assert_eq!(close.x + close.width, toast.x + toast.width);
        assert_eq!(close.y, toast.y);
    }

    #[test]
    fn the_stack_moves_away_from_the_cursor() {
        let stack = Rect::new(500.0, 500.0, 440.0, 100.0);
        let under = Rect::new(600.0, 550.0, 10.0, 20.0);
        let away = Rect::new(10.0, 40.0, 10.0, 20.0);
        assert_eq!(
            avoid_cursor(Corner::BottomRight, stack, under),
            Corner::TopRight
        );
        assert_eq!(
            avoid_cursor(Corner::BottomLeft, stack, under),
            Corner::TopLeft
        );
        assert_eq!(
            avoid_cursor(Corner::TopRight, stack, under),
            Corner::BottomRight
        );
        assert_eq!(avoid_cursor(Corner::Bottom, stack, under), Corner::TopRight);
        assert_eq!(
            avoid_cursor(Corner::BottomRight, stack, away),
            Corner::BottomRight
        );
    }

    #[test]
    fn a_toast_has_a_background_a_level_bar_and_text() {
        let rects = layout_toasts(1, AREA, Corner::BottomRight, CELL);
        let views = [ToastView {
            title: "Build",
            body: "ok",
            level: ToastLevel::Success,
            hover: false,
        }];
        let quads = build_toasts(&views, &rects, CELL, &mut |_| Ok(Some(GLYPH))).unwrap();
        let bg = quads
            .iter()
            .find(|q| {
                q.kind == KIND_SOLID
                    && q.rect == [rects[0].x, rects[0].y, rects[0].width, rects[0].height]
            })
            .expect("no background");
        assert!(bg.color[3] < 1.0, "a bit see-through");
        let bar = linear(ToastLevel::Success.color());
        assert!(quads.iter().any(|q| q.kind == KIND_SOLID && q.color == bar));
        // "Build" + "ok", no × without the mouse.
        assert_eq!(quads.iter().filter(|q| q.kind == KIND_GLYPH).count(), 7);
    }

    #[test]
    fn hover_shows_the_close_button() {
        let rects = layout_toasts(1, AREA, Corner::BottomRight, CELL);
        let views = [ToastView {
            title: "A",
            body: "",
            level: ToastLevel::Info,
            hover: true,
        }];
        let quads = build_toasts(&views, &rects, CELL, &mut |_| Ok(Some(GLYPH))).unwrap();
        assert_eq!(
            quads.iter().filter(|q| q.kind == KIND_GLYPH).count(),
            2,
            "A and ×"
        );
    }

    #[test]
    fn a_long_body_is_cut_to_two_lines() {
        let rects = layout_toasts(1, AREA, Corner::BottomRight, CELL);
        let body = "word ".repeat(100);
        let views = [ToastView {
            title: "",
            body: &body,
            level: ToastLevel::Info,
            hover: false,
        }];
        let quads = build_toasts(&views, &rects, CELL, &mut |_| Ok(Some(GLYPH))).unwrap();
        let rows: std::collections::BTreeSet<i64> = quads
            .iter()
            .filter(|q| q.kind == KIND_GLYPH)
            .map(|q| q.rect[1] as i64)
            .collect();
        assert_eq!(rows.len(), 2);
    }
}
