//! The dock: an area at a window edge with service panels (Events, Agents).
//! It is not a terminal. Each panel is a list of rows with two lines of text.

use fterm_term::alacritty_terminal::vte::ansi::Rgb;

use crate::atlas::{AtlasFull, AtlasGlyph, GlyphKey};
use crate::font::CellMetrics;
use crate::frame::{Instance, Rect};
use crate::tabbar::{BAR_PADDING, char_cells, fit_title, push_text, solid};
use crate::theme::UiColors;

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

/// How a line of the AI chat looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatStyle {
    /// What the user asked.
    User,
    /// The text of an answer.
    Answer,
    /// The language name over a code block.
    CodeHeader,
    /// A line of a code block.
    Code,
    Error,
    /// Quiet text (an empty line between turns, "Thinking…").
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatLine {
    pub text: String,
    pub style: ChatStyle,
}

/// The input box grows up to this many lines.
pub const MAX_INPUT_ROWS: usize = 6;

/// The AI chat in the dock (instead of the rows).
pub struct ChatView<'a> {
    pub lines: &'a [ChatLine],
    /// How many lines the view is up from the end (0 = the newest lines).
    pub scroll: usize,
    /// The input lines (from `InputBox::layout`).
    pub input: &'a [String],
    /// The text cursor in the input (line, cell), when the dock has the keyboard.
    pub cursor: Option<(usize, usize)>,
    /// The provider and the model, on the right of the panel tabs.
    pub title: &'a str,
    /// What goes with the next question ("output (42 lines)"), over the input.
    pub chips: &'a [String],
}

/// The height of the input box for this many lines.
fn input_height(cell: CellMetrics, input_lines: usize) -> f32 {
    (input_lines.clamp(1, MAX_INPUT_ROWS) as f32 + 0.5) * cell.height
}

/// How many chat lines fit above an input box with `input_lines` lines.
pub fn chat_rows(layout: &DockLayout, cell: CellMetrics, input_lines: usize) -> usize {
    // One line more for the chips (it is always kept, so the chat does not jump).
    let free = layout.list.height - input_height(cell, input_lines) - cell.height * 1.5;
    (free / cell.height).floor().max(0.0) as usize
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
    /// The AI chat: drawn instead of the rows.
    pub chat: Option<ChatView<'a>>,
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
    ui: &UiColors,
    cell: CellMetrics,
    glyph: &mut dyn FnMut(&GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
) -> Result<Vec<Instance>, AtlasFull> {
    let rect = layout.rect;
    let list = layout.list;
    let mut quads = vec![solid(rect, ui.surface)];
    // A line between the dock and the terminal.
    let edge = match layout.side {
        DockSide::Right => Rect::new(rect.x, rect.y, 1.0, rect.height),
        DockSide::Left => Rect::new(rect.x + rect.width - 1.0, rect.y, 1.0, rect.height),
        DockSide::Bottom => Rect::new(rect.x, rect.y, rect.width, 1.0),
    };
    quads.push(solid(edge, ui.overlay));

    // The panel tabs.
    let header = cell.height + 2.0 * BAR_PADDING;
    quads.push(solid(
        Rect::new(rect.x + 1.0, rect.y, rect.width - 1.0, header),
        ui.surface,
    ));
    let mut text = Vec::new();
    for (i, (tab, label)) in layout.tabs.iter().zip(view.tabs).enumerate() {
        if tab.x + tab.width > rect.x + rect.width {
            break;
        }
        let active = i == view.active;
        if active {
            quads.push(solid(*tab, ui.surface_active));
            let accent = if view.focused { ui.accent } else { ui.text_dim };
            quads.push(solid(Rect::new(tab.x, tab.y, tab.width, 2.0), accent));
        }
        let color = if active { ui.text } else { ui.text_dim };
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
        ui.overlay,
    ));

    let cells = (list.width / cell.width) as usize;
    if let Some(chat) = &view.chat {
        build_chat(chat, layout, ui, cell, &mut quads, &mut text, glyph)?;
        if view.focused && !view.hints.is_empty() {
            let y = list.y + list.height + cell.height * 0.125;
            let shown = fit_title(view.hints, cells.saturating_sub(2));
            push_text(
                &mut text,
                &shown,
                list.x + cell.width,
                y,
                cell,
                ui.text_dim,
                glyph,
            )?;
        }
        quads.extend(text);
        return Ok(quads);
    }
    if view.rows.is_empty() {
        let shown = fit_title(view.empty, cells.saturating_sub(2));
        push_text(
            &mut text,
            &shown,
            list.x + cell.width,
            list.y + cell.height * 0.5,
            cell,
            ui.text_dim,
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
            let bg = if view.focused {
                ui.selected
            } else {
                ui.overlay
            };
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
        let title_color = ui.text;
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
            push_text(&mut text, &row.right, x, y + pad, cell, ui.text_dim, glyph)?;
        }
        let detail = fit_title(&row.detail, cells.saturating_sub(2));
        push_text(
            &mut text,
            &detail,
            list.x + cell.width,
            y + pad + cell.height,
            cell,
            ui.text_dim,
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
            ui.overlay,
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
            ui.text_dim,
            glyph,
        )?;
    }
    quads.extend(text);
    Ok(quads)
}

/// The chat lines (the newest at the bottom), the input box, and the title.
fn build_chat(
    chat: &ChatView,
    layout: &DockLayout,
    ui: &UiColors,
    cell: CellMetrics,
    quads: &mut Vec<Instance>,
    text: &mut Vec<Instance>,
    glyph: &mut dyn FnMut(&GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
) -> Result<(), AtlasFull> {
    let list = layout.list;
    let cells = (list.width / cell.width) as usize;
    // The title on the right of the panel tabs.
    if !chat.title.is_empty() {
        let tabs_end = layout.tabs.last().map_or(list.x, |t| t.x + t.width);
        let room = ((list.x + list.width - tabs_end) / cell.width) as usize;
        let shown = fit_title(chat.title, room.saturating_sub(2));
        let w: usize = shown.chars().map(char_cells).sum();
        if w > 0 {
            let x = list.x + list.width - (w + 1) as f32 * cell.width;
            push_text(
                text,
                &shown,
                x,
                layout.rect.y + BAR_PADDING,
                cell,
                ui.text_dim,
                glyph,
            )?;
        }
    }
    // The lines: the last ones that fit, moved up by `scroll`.
    let rows = chat_rows(layout, cell, chat.input.len());
    let end = chat.lines.len().saturating_sub(chat.scroll);
    let start = end.saturating_sub(rows);
    for (n, line) in chat.lines[start..end].iter().enumerate() {
        let y = list.y + cell.height * 0.25 + n as f32 * cell.height;
        let color = match line.style {
            ChatStyle::User => ui.accent,
            ChatStyle::Answer => ui.text,
            ChatStyle::Code => ui.text,
            ChatStyle::CodeHeader | ChatStyle::Note => ui.text_dim,
            ChatStyle::Error => ui.error,
        };
        if matches!(line.style, ChatStyle::Code | ChatStyle::CodeHeader) {
            quads.push(solid(
                Rect::new(list.x + 2.0, y, list.width - 4.0, cell.height),
                ui.code_bg,
            ));
        }
        let shown = fit_title(&line.text, cells.saturating_sub(2));
        push_text(text, &shown, list.x + cell.width, y, cell, color, glyph)?;
    }
    // A scroll bar when the chat does not fit.
    if chat.lines.len() > rows && rows > 0 {
        let area = rows as f32 * cell.height;
        let total = chat.lines.len() as f32;
        let height = (area * rows as f32 / total).max(cell.height);
        let top_share = start as f32 / (total - rows as f32).max(1.0);
        let top = list.y + cell.height * 0.25 + (area - height) * top_share;
        quads.push(solid(
            Rect::new(list.x + list.width - 3.0, top, 3.0, height),
            ui.overlay,
        ));
    }
    // The input box at the bottom.
    let height = input_height(cell, chat.input.len());
    let input = Rect::new(
        list.x + 2.0,
        list.y + list.height - height,
        list.width - 4.0,
        height,
    );
    quads.push(solid(input, ui.input_bg));
    // The chips over it, side by side.
    let mut x = list.x + 2.0;
    let chip_y = input.y - cell.height - 2.0;
    let end = list.x + list.width;
    for (i, chip) in chat.chips.iter().enumerate() {
        let w = (chip.chars().map(char_cells).sum::<usize>() + 2) as f32 * cell.width;
        let rest = chat.chips.len() - i - 1;
        // Keep room for a `+N` chip when more chips come after this one.
        let room_after = if rest > 0 { 5.0 * cell.width } else { 0.0 };
        if x + w + room_after > end {
            let more = format!("+{}", chat.chips.len() - i);
            let w = (more.chars().count() + 2) as f32 * cell.width;
            if x + w <= end {
                quads.push(solid(Rect::new(x, chip_y, w, cell.height), ui.chip_bg));
                push_text(text, &more, x + cell.width, chip_y, cell, ui.text, glyph)?;
            }
            break;
        }
        quads.push(solid(Rect::new(x, chip_y, w, cell.height), ui.chip_bg));
        push_text(text, chip, x + cell.width, chip_y, cell, ui.text, glyph)?;
        x += w + cell.width * 0.5;
    }
    let first = chat.input.len().saturating_sub(MAX_INPUT_ROWS);
    for (n, line) in chat.input.iter().skip(first).enumerate() {
        let y = input.y + cell.height * 0.25 + n as f32 * cell.height;
        push_text(text, line, list.x + cell.width, y, cell, ui.text, glyph)?;
    }
    if let Some((row, col)) = chat.cursor
        && row >= first
    {
        let y = input.y + cell.height * 0.25 + (row - first) as f32 * cell.height;
        let x = list.x + cell.width + col as f32 * cell.width;
        quads.push(solid(Rect::new(x, y, 2.0, cell.height), ui.text));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> UiColors {
        UiColors::default()
    }
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
            chat: None,
        };
        let build = |view: &DockView| {
            build_dock(view, &layout, &ui(), CELL, &mut |_| Ok(Some(GLYPH))).unwrap()
        };
        let quads = build(&view);
        let found = colors(&quads);
        assert!(found.contains(&linear_color(red)));
        assert!(found.contains(&linear_color(green)));
        assert!(
            !found.contains(&linear_color(ui().selected)),
            "no selection"
        );
        let glyphs_unfocused = quads.len();

        view.selected = Some(1);
        view.focused = true;
        let quads = build(&view);
        assert!(colors(&quads).contains(&linear_color(ui().selected)));
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
            chat: None,
        };
        let quads = build_dock(&view, &layout, &ui(), CELL, &mut |_| Ok(Some(GLYPH))).unwrap();
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
            chat: None,
        };
        let quads = build_dock(&view, &layout, &ui(), CELL, &mut |_| Ok(Some(GLYPH))).unwrap();
        for q in &quads {
            assert!(
                q.rect[1] + q.rect[3] <= rect.y + rect.height + 0.5,
                "{:?}",
                q.rect
            );
            assert!(q.rect[0] >= rect.x - 0.5);
        }
    }

    fn chat_line(text: &str, style: ChatStyle) -> ChatLine {
        ChatLine {
            text: text.into(),
            style,
        }
    }

    fn chat_quads(chat: ChatView) -> (DockLayout, Vec<Instance>) {
        let (_, rect) = split_area(AREA, DockSide::Right, 0.4, CELL);
        let tabs = tabs();
        let layout = layout_dock(rect, DockSide::Right, &tabs, CELL);
        let view = DockView {
            tabs: &tabs,
            active: 0,
            rows: &[],
            selected: None,
            scroll: 0,
            focused: true,
            empty: "",
            hints: "Enter send",
            chat: Some(chat),
        };
        let quads = build_dock(&view, &layout, &ui(), CELL, &mut |_| Ok(Some(GLYPH))).unwrap();
        (layout, quads)
    }

    #[test]
    fn a_chat_has_lines_code_and_an_input_with_a_cursor() {
        let lines = [
            chat_line("hi", ChatStyle::User),
            chat_line("bash", ChatStyle::CodeHeader),
            chat_line("ls", ChatStyle::Code),
            chat_line("no key", ChatStyle::Error),
        ];
        let input = ["ab".to_owned()];
        let (layout, quads) = chat_quads(ChatView {
            lines: &lines,
            scroll: 0,
            input: &input,
            cursor: Some((0, 2)),
            title: "anthropic · haiku",
            chips: &[],
        });
        let found = colors(&quads);
        assert!(
            found.contains(&linear_color(ui().code_bg)),
            "code lines have a background"
        );
        assert!(
            found.contains(&linear_color(ui().input_bg)),
            "the input box"
        );
        let cursor = quads
            .iter()
            .find(|q| {
                q.kind == KIND_SOLID && q.color == linear_color(ui().text) && q.rect[2] <= 2.0
            })
            .expect("a text cursor");
        assert!(
            cursor.rect[1] > layout.list.y + layout.list.height / 2.0,
            "the input is at the bottom"
        );
        let red = quads
            .iter()
            .filter(|q| q.kind == crate::frame::KIND_GLYPH && q.color == linear_color(ui().error))
            .count();
        assert_eq!(red, 5, "`no key` is red (5 glyphs, no space)");
        for q in &quads {
            assert!(
                q.rect[1] >= layout.rect.y - 0.5
                    && q.rect[1] + q.rect[3] <= layout.rect.y + layout.rect.height + 0.5
            );
        }
    }

    #[test]
    fn a_long_chat_shows_its_end_and_scrolls_up() {
        let lines: Vec<ChatLine> = (0..200)
            .map(|i| {
                chat_line(
                    if i == 0 { "FIRST" } else { "x" },
                    if i == 0 {
                        ChatStyle::Error
                    } else {
                        ChatStyle::Answer
                    },
                )
            })
            .collect();
        let input = [String::new()];
        let view = |scroll| ChatView {
            lines: &lines,
            scroll,
            input: &input,
            cursor: None,
            title: "",
            chips: &[],
        };
        let red = |quads: &[Instance]| {
            quads
                .iter()
                .any(|q| q.kind == crate::frame::KIND_GLYPH && q.color == linear_color(ui().error))
        };
        let (layout, quads) = chat_quads(view(0));
        assert!(!red(&quads), "the bottom: the first line is far above");
        let rows = chat_rows(&layout, CELL, 1);
        assert!(rows > 5 && rows < 200);
        let (_, quads) = chat_quads(view(200 - rows));
        assert!(red(&quads), "scrolled to the top: the first line shows");
    }

    #[test]
    fn chips_show_over_the_input() {
        let input = [String::new()];
        let chips = ["output (42 lines)".to_owned(), "selection".to_owned()];
        let (layout, quads) = chat_quads(ChatView {
            lines: &[],
            scroll: 0,
            input: &input,
            cursor: None,
            title: "",
            chips: &chips,
        });
        let chip_bgs: Vec<&Instance> = quads
            .iter()
            .filter(|q| q.kind == KIND_SOLID && q.color == linear_color(ui().chip_bg))
            .collect();
        assert_eq!(chip_bgs.len(), 2, "one box for each chip");
        assert!(chip_bgs[0].rect[0] < chip_bgs[1].rect[0], "side by side");
        let input_top = quads
            .iter()
            .find(|q| q.kind == KIND_SOLID && q.color == linear_color(ui().input_bg))
            .unwrap()
            .rect[1];
        assert!(
            chip_bgs[0].rect[1] + chip_bgs[0].rect[3] <= input_top + 0.5,
            "over the input"
        );
        assert!(chip_bgs[0].rect[1] >= layout.list.y);
    }

    #[test]
    fn chips_that_do_not_fit_show_as_a_count() {
        let input = [String::new()];
        let chips = [
            "output (42 lines)".to_owned(),
            "last command (exit 1)".to_owned(),
            "selection (3 lines)".to_owned(),
        ];
        let (_, quads) = chat_quads(ChatView {
            lines: &[],
            scroll: 0,
            input: &input,
            cursor: None,
            title: "",
            chips: &chips,
        });
        let boxes = quads
            .iter()
            .filter(|q| q.kind == KIND_SOLID && q.color == linear_color(ui().chip_bg))
            .count();
        assert_eq!(boxes, 2, "the first chip and a `+2` chip");
    }
}
