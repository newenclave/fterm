//! The dock: an area at a window edge with service panels (Events, Agents).
//! It is not a terminal. Each panel is a list of rows with two lines of text.

use fterm_term::alacritty_terminal::vte::ansi::Rgb;

use crate::atlas::{AtlasFull, AtlasGlyph, GlyphKey};
use crate::font::CellMetrics;
use crate::frame::{Instance, Rect};
use crate::overlay::{BOX_TEXT, HINT_TEXT};
use crate::tabbar::{
    ACCENT, ACTIVE_BG, ACTIVE_TEXT, BAR_BG, BAR_PADDING, HOVER_BG, TEXT, char_cells, fit_title,
    push_text, solid,
};

/// The background of the selected row.
pub const SELECTED_BG: Rgb = Rgb {
    r: 0x45,
    g: 0x47,
    b: 0x5a,
};

/// The dock background (a bit darker than the terminal).
pub const DOCK_BG: Rgb = Rgb {
    r: 0x18,
    g: 0x18,
    b: 0x25,
};
/// The line between the dock and the terminal, and the scroll bar.
pub const EDGE: Rgb = Rgb {
    r: 0x31,
    g: 0x32,
    b: 0x44,
};
/// A row is two lines of text and some space.
pub const ROW_LINES: f32 = 2.5;

/// Where the dock is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DockSide {
    Left,
    #[default]
    Right,
    Bottom,
}

/// The smallest dock and the smallest terminal area next to it, in cells.
pub const MIN_CELLS: f32 = 20.0;
/// The smallest bottom dock and terminal area, in lines.
pub const MIN_LINES: f32 = 5.0;
/// The part of the dock edge you can drag, in pixels.
pub const GRAB: f32 = 5.0;

/// Splits `area` into (the terminal area, the dock). `ratio` is the dock part (0.1 ..= 0.9).
pub fn split_area(area: Rect, side: DockSide, ratio: f32, cell: CellMetrics) -> (Rect, Rect) {
    let ratio = ratio.clamp(0.1, 0.9);
    let size_of = |total: f32, min: f32| (total * ratio).round().max(min).min(total - min).max(0.0);
    match side {
        DockSide::Left | DockSide::Right => {
            let size = size_of(area.width, MIN_CELLS * cell.width);
            let rest = area.width - size;
            if side == DockSide::Left {
                (
                    Rect::new(area.x + size, area.y, rest, area.height),
                    Rect::new(area.x, area.y, size, area.height),
                )
            } else {
                (
                    Rect::new(area.x, area.y, rest, area.height),
                    Rect::new(area.x + rest, area.y, size, area.height),
                )
            }
        }
        DockSide::Bottom => {
            let size = size_of(area.height, MIN_LINES * cell.height);
            let rest = area.height - size;
            (
                Rect::new(area.x, area.y, area.width, rest),
                Rect::new(area.x, area.y + rest, area.width, size),
            )
        }
    }
}

/// One row of a panel.
#[derive(Clone, Debug, PartialEq)]
pub struct DockRow {
    /// The color bar on the left (the level or the agent state).
    pub marker: Rgb,
    pub title: String,
    pub detail: String,
    /// Short text on the right of the title, for example "2 m".
    pub right: String,
    /// New (not read yet): the title is bright.
    pub new: bool,
}

/// What the dock shows.
pub struct DockView<'a> {
    /// The panel tabs at the top, for example "Events 3".
    pub tabs: &'a [String],
    pub active: usize,
    pub rows: &'a [DockRow],
    pub selected: Option<usize>,
    /// The first row on the screen.
    pub scroll: usize,
    /// The keyboard is in the dock: the selection is bright and the key hints show.
    pub focused: bool,
    /// The text when there are no rows.
    pub empty: &'a str,
    /// Key hints at the bottom (only when focused).
    pub hints: &'a str,
}

/// The places of the dock parts.
#[derive(Clone, Debug, PartialEq)]
pub struct DockLayout {
    pub rect: Rect,
    pub side: DockSide,
    pub tabs: Vec<Rect>,
    /// The area of the rows.
    pub list: Rect,
    pub row_height: f32,
    /// The edge you drag to change the size.
    pub grab: Rect,
}

impl DockLayout {
    /// How many rows fit.
    pub fn visible_rows(&self) -> usize {
        (self.list.height / self.row_height).floor().max(0.0) as usize
    }
}

pub fn layout_dock(rect: Rect, side: DockSide, tabs: &[String], cell: CellMetrics) -> DockLayout {
    let header = cell.height + 2.0 * BAR_PADDING;
    let mut x = rect.x + GRAB;
    let tabs = tabs
        .iter()
        .map(|label| {
            let cells: usize = label.chars().map(char_cells).sum();
            let width = (cells + 2) as f32 * cell.width;
            let tab = Rect::new(x, rect.y, width, header);
            x += width;
            tab
        })
        .collect();
    let footer = cell.height * 1.25;
    let list = Rect::new(
        rect.x + GRAB,
        rect.y + header + 1.0,
        (rect.width - 2.0 * GRAB).max(0.0),
        (rect.height - header - 1.0 - footer).max(0.0),
    );
    let grab = match side {
        DockSide::Right => Rect::new(rect.x, rect.y, GRAB, rect.height),
        DockSide::Left => Rect::new(rect.x + rect.width - GRAB, rect.y, GRAB, rect.height),
        DockSide::Bottom => Rect::new(rect.x, rect.y, rect.width, GRAB),
    };
    DockLayout {
        rect,
        side,
        tabs,
        list,
        row_height: ROW_LINES * cell.height,
        grab,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockHit {
    Tab(usize),
    /// A row, counted from the first row (with the scroll).
    Row(usize),
    Grab,
    /// In the dock, but not on a tab or a row.
    Inside,
    Outside,
}

pub fn dock_hit(layout: &DockLayout, scroll: usize, rows: usize, x: f32, y: f32) -> DockHit {
    if !layout.rect.contains(x, y) {
        return DockHit::Outside;
    }
    if layout.grab.contains(x, y) {
        return DockHit::Grab;
    }
    if let Some(i) = layout.tabs.iter().position(|t| t.contains(x, y)) {
        return DockHit::Tab(i);
    }
    if layout.list.contains(x, y) {
        let row = ((y - layout.list.y) / layout.row_height) as usize;
        let index = scroll + row;
        if row < layout.visible_rows() && index < rows {
            return DockHit::Row(index);
        }
    }
    DockHit::Inside
}

pub fn build_dock(
    view: &DockView,
    layout: &DockLayout,
    cell: CellMetrics,
    glyph: &mut dyn FnMut(&GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
) -> Result<Vec<Instance>, AtlasFull> {
    let rect = layout.rect;
    let list = layout.list;
    let mut quads = vec![solid(rect, DOCK_BG)];
    // A line between the dock and the terminal.
    let edge = match layout.side {
        DockSide::Right => Rect::new(rect.x, rect.y, 1.0, rect.height),
        DockSide::Left => Rect::new(rect.x + rect.width - 1.0, rect.y, 1.0, rect.height),
        DockSide::Bottom => Rect::new(rect.x, rect.y, rect.width, 1.0),
    };
    quads.push(solid(edge, EDGE));

    // The panel tabs.
    let header = cell.height + 2.0 * BAR_PADDING;
    quads.push(solid(
        Rect::new(rect.x + 1.0, rect.y, rect.width - 1.0, header),
        BAR_BG,
    ));
    let mut text = Vec::new();
    for (i, (tab, label)) in layout.tabs.iter().zip(view.tabs).enumerate() {
        if tab.x + tab.width > rect.x + rect.width {
            break;
        }
        let active = i == view.active;
        if active {
            quads.push(solid(*tab, ACTIVE_BG));
            let accent = if view.focused { ACCENT } else { HINT_TEXT };
            quads.push(solid(Rect::new(tab.x, tab.y, tab.width, 2.0), accent));
        }
        let color = if active { ACTIVE_TEXT } else { TEXT };
        push_text(
            &mut text,
            label,
            tab.x + cell.width,
            tab.y + BAR_PADDING,
            cell,
            color,
            glyph,
        )?;
    }
    quads.push(solid(
        Rect::new(rect.x + 1.0, rect.y + header, rect.width - 1.0, 1.0),
        EDGE,
    ));

    let cells = (list.width / cell.width) as usize;
    if view.rows.is_empty() {
        let shown = fit_title(view.empty, cells.saturating_sub(2));
        push_text(
            &mut text,
            &shown,
            list.x + cell.width,
            list.y + cell.height * 0.5,
            cell,
            HINT_TEXT,
            glyph,
        )?;
    }
    let visible = layout.visible_rows();
    for (n, (index, row)) in view
        .rows
        .iter()
        .enumerate()
        .skip(view.scroll)
        .take(visible)
        .enumerate()
    {
        let y = list.y + n as f32 * layout.row_height;
        if view.selected == Some(index) {
            let bg = if view.focused { SELECTED_BG } else { HOVER_BG };
            quads.push(solid(
                Rect::new(list.x, y, list.width, layout.row_height),
                bg,
            ));
        }
        let pad = cell.height * 0.25;
        quads.push(solid(
            Rect::new(list.x + 2.0, y + pad, 3.0, layout.row_height - 2.0 * pad),
            row.marker,
        ));
        let right_cells: usize = row.right.chars().map(char_cells).sum();
        let title_cells = cells.saturating_sub(3 + right_cells);
        let title_color = if row.new { ACTIVE_TEXT } else { BOX_TEXT };
        let title = fit_title(&row.title, title_cells);
        push_text(
            &mut text,
            &title,
            list.x + cell.width,
            y + pad,
            cell,
            title_color,
            glyph,
        )?;
        if right_cells > 0 {
            let x = list.x + list.width - (1 + right_cells) as f32 * cell.width;
            push_text(&mut text, &row.right, x, y + pad, cell, HINT_TEXT, glyph)?;
        }
        let detail = fit_title(&row.detail, cells.saturating_sub(2));
        push_text(
            &mut text,
            &detail,
            list.x + cell.width,
            y + pad + cell.height,
            cell,
            HINT_TEXT,
            glyph,
        )?;
    }

    // A scroll bar when not all rows fit.
    if view.rows.len() > visible && visible > 0 {
        let total = view.rows.len() as f32;
        let height = (list.height * visible as f32 / total).max(cell.height);
        let top = list.y
            + (list.height - height) * view.scroll as f32 / (total - visible as f32).max(1.0);
        quads.push(solid(
            Rect::new(list.x + list.width - 3.0, top, 3.0, height),
            EDGE,
        ));
    }

    if view.focused && !view.hints.is_empty() {
        let y = list.y + list.height + cell.height * 0.125;
        let shown = fit_title(view.hints, cells.saturating_sub(2));
        push_text(
            &mut text,
            &shown,
            list.x + cell.width,
            y,
            cell,
            HINT_TEXT,
            glyph,
        )?;
    }
    quads.extend(text);
    Ok(quads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::linear as linear_color;
    use crate::frame::KIND_SOLID;

    const CELL: CellMetrics = CellMetrics {
        width: 10.0,
        height: 20.0,
        baseline: 15.0,
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
    const AREA: Rect = Rect {
        x: 0.0,
        y: 30.0,
        width: 1000.0,
        height: 600.0,
    };

    fn tabs() -> Vec<String> {
        vec!["Events 3".to_owned(), "Agents".to_owned()]
    }

    fn row(marker: Rgb) -> DockRow {
        DockRow {
            marker,
            title: "Build".into(),
            detail: "All tests pass".into(),
            right: "2 m".into(),
            new: false,
        }
    }

    #[test]
    fn a_right_dock_takes_its_part_on_the_right() {
        let (terminal, dock) = split_area(AREA, DockSide::Right, 0.3, CELL);
        assert_eq!(dock, Rect::new(700.0, 30.0, 300.0, 600.0));
        assert_eq!(terminal, Rect::new(0.0, 30.0, 700.0, 600.0));
    }

    #[test]
    fn left_and_bottom_docks() {
        let (terminal, dock) = split_area(AREA, DockSide::Left, 0.3, CELL);
        assert_eq!(dock, Rect::new(0.0, 30.0, 300.0, 600.0));
        assert_eq!(terminal, Rect::new(300.0, 30.0, 700.0, 600.0));
        let (terminal, dock) = split_area(AREA, DockSide::Bottom, 0.25, CELL);
        assert_eq!(dock, Rect::new(0.0, 480.0, 1000.0, 150.0));
        assert_eq!(terminal, Rect::new(0.0, 30.0, 1000.0, 450.0));
    }

    #[test]
    fn the_dock_and_the_terminal_keep_a_minimum_size() {
        // 0.05 of 1000 px is 50 px, but the dock needs 20 cells = 200 px.
        let (_, dock) = split_area(AREA, DockSide::Right, 0.05, CELL);
        assert_eq!(dock.width, 200.0);
        // 0.95 leaves 50 px, but the terminal needs 20 cells too.
        let (terminal, dock) = split_area(AREA, DockSide::Right, 0.95, CELL);
        assert_eq!(terminal.width, 200.0);
        assert_eq!(dock.width, 800.0);
        // A window that is too small: the terminal wins, the dock gets what is left.
        let small = Rect::new(0.0, 0.0, 300.0, 100.0);
        let (terminal, dock) = split_area(small, DockSide::Right, 0.5, CELL);
        assert_eq!(terminal.width + dock.width, 300.0);
        assert!(terminal.width >= 200.0);
    }

    #[test]
    fn layout_has_tabs_on_top_and_rows_below() {
        let (_, rect) = split_area(AREA, DockSide::Right, 0.3, CELL);
        let layout = layout_dock(rect, DockSide::Right, &tabs(), CELL);
        assert_eq!(layout.tabs.len(), 2);
        assert!(layout.tabs[0].x < layout.tabs[1].x);
        assert_eq!(layout.tabs[0].y, rect.y);
        assert!(layout.list.y >= layout.tabs[0].y + layout.tabs[0].height);
        assert_eq!(layout.row_height, 2.5 * CELL.height);
        assert_eq!(layout.visible_rows(), (layout.list.height / 50.0) as usize);
        // A right dock is dragged on its left edge.
        assert_eq!(layout.grab.x, rect.x);
        assert_eq!(layout.grab.width, GRAB);
        let bottom = layout_dock(
            Rect::new(0.0, 400.0, 1000.0, 200.0),
            DockSide::Bottom,
            &tabs(),
            CELL,
        );
        assert_eq!(bottom.grab.y, 400.0);
        assert_eq!(bottom.grab.height, GRAB);
    }

    #[test]
    fn hits() {
        let (_, rect) = split_area(AREA, DockSide::Right, 0.3, CELL);
        let layout = layout_dock(rect, DockSide::Right, &tabs(), CELL);
        let t1 = layout.tabs[1];
        assert_eq!(
            dock_hit(&layout, 0, 10, t1.x + 2.0, t1.y + 2.0),
            DockHit::Tab(1)
        );
        let first = layout.list.y + 1.0;
        let x = rect.x + 50.0;
        assert_eq!(dock_hit(&layout, 0, 10, x, first), DockHit::Row(0));
        assert_eq!(
            dock_hit(&layout, 3, 10, x, first + layout.row_height),
            DockHit::Row(4)
        );
        // Under the last row.
        assert_eq!(
            dock_hit(&layout, 0, 1, x, first + layout.row_height),
            DockHit::Inside
        );
        assert_eq!(dock_hit(&layout, 0, 10, rect.x + 1.0, first), DockHit::Grab);
        assert_eq!(dock_hit(&layout, 0, 10, 10.0, 100.0), DockHit::Outside);
    }

    fn colors(quads: &[Instance]) -> Vec<[f32; 4]> {
        quads
            .iter()
            .filter(|q| q.kind == KIND_SOLID)
            .map(|q| q.color)
            .collect()
    }

    #[test]
    fn rows_show_their_markers_and_the_selection() {
        let (_, rect) = split_area(AREA, DockSide::Right, 0.3, CELL);
        let tabs = tabs();
        let layout = layout_dock(rect, DockSide::Right, &tabs, CELL);
        let red = Rgb { r: 255, g: 0, b: 0 };
        let green = Rgb { r: 0, g: 255, b: 0 };
        let rows = [row(red), row(green)];
        let mut view = DockView {
            tabs: &tabs,
            active: 0,
            rows: &rows,
            selected: None,
            scroll: 0,
            focused: false,
            empty: "Nothing yet",
            hints: "Enter go",
        };
        let build =
            |view: &DockView| build_dock(view, &layout, CELL, &mut |_| Ok(Some(GLYPH))).unwrap();
        let quads = build(&view);
        let found = colors(&quads);
        assert!(found.contains(&linear_color(red)));
        assert!(found.contains(&linear_color(green)));
        assert!(!found.contains(&linear_color(SELECTED_BG)), "no selection");
        let glyphs_unfocused = quads.len();

        view.selected = Some(1);
        view.focused = true;
        let quads = build(&view);
        assert!(colors(&quads).contains(&linear_color(SELECTED_BG)));
        assert!(
            quads.len() > glyphs_unfocused,
            "the key hints show when focused"
        );
    }

    #[test]
    fn rows_above_the_scroll_are_not_drawn() {
        let (_, rect) = split_area(AREA, DockSide::Right, 0.3, CELL);
        let tabs = tabs();
        let layout = layout_dock(rect, DockSide::Right, &tabs, CELL);
        let red = Rgb { r: 255, g: 0, b: 0 };
        let green = Rgb { r: 0, g: 255, b: 0 };
        let rows = [row(red), row(green)];
        let view = DockView {
            tabs: &tabs,
            active: 0,
            rows: &rows,
            selected: None,
            scroll: 1,
            focused: false,
            empty: "",
            hints: "",
        };
        let quads = build_dock(&view, &layout, CELL, &mut |_| Ok(Some(GLYPH))).unwrap();
        let found = colors(&quads);
        assert!(!found.contains(&linear_color(red)));
        assert!(found.contains(&linear_color(green)));
    }

    #[test]
    fn many_rows_stay_inside_the_list() {
        let (_, rect) = split_area(AREA, DockSide::Right, 0.3, CELL);
        let tabs = tabs();
        let layout = layout_dock(rect, DockSide::Right, &tabs, CELL);
        let rows: Vec<DockRow> = (0..100).map(|_| row(Rgb { r: 1, g: 2, b: 3 })).collect();
        let view = DockView {
            tabs: &tabs,
            active: 1,
            rows: &rows,
            selected: Some(0),
            scroll: 0,
            focused: false,
            empty: "",
            hints: "",
        };
        let quads = build_dock(&view, &layout, CELL, &mut |_| Ok(Some(GLYPH))).unwrap();
        for q in &quads {
            assert!(
                q.rect[1] + q.rect[3] <= rect.y + rect.height + 0.5,
                "{:?}",
                q.rect
            );
            assert!(q.rect[0] >= rect.x - 0.5);
        }
    }
}
