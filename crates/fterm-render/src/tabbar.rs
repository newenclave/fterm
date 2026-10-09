//! The tab bar at the top of the window: where the tabs are, what is under the mouse,
//! and the quads to draw it. It has no GPU code, so it is easy to test.

use fterm_term::alacritty_terminal::vte::ansi::Rgb;
use unicode_width::UnicodeWidthChar;

use crate::atlas::{AtlasFull, AtlasGlyph, GlyphKey};
use crate::color::linear;
use crate::font::CellMetrics;
use crate::frame::{Instance, KIND_COLOR_GLYPH, KIND_GLYPH, KIND_SOLID, Rect};
use crate::theme::UiColors;

/// Space above and below the text, in pixels (physical).
pub const BAR_PADDING: f32 = 4.0;
/// The height of the color line at the top of a colored tab.
pub const TAB_COLOR_LINE: f32 = 3.0;
/// Tab width limits, in cells.
pub const MIN_TAB_CELLS: f32 = 8.0;
pub const MAX_TAB_CELLS: f32 = 30.0;
/// The `+` button width, in cells.
pub const NEW_TAB_CELLS: f32 = 3.0;

pub fn bar_height(cell: CellMetrics) -> f32 {
    cell.height + 2.0 * BAR_PADDING
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabRect {
    pub rect: Rect,
    /// The `×` button.
    pub close: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabBarLayout {
    pub tabs: Vec<TabRect>,
    pub new_tab: Rect,
    pub height: f32,
}

/// What is under the mouse in the tab bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hit {
    Tab(usize),
    Close(usize),
    NewTab,
    /// In the bar, but not on a button.
    Bar,
    /// Not in the bar.
    #[default]
    None,
}

/// Places `count` tabs in a bar of `width` pixels.
pub fn layout_tabs(count: usize, width: f32, cell: CellMetrics) -> TabBarLayout {
    let height = bar_height(cell);
    let new_tab_width = NEW_TAB_CELLS * cell.width;
    let space = (width - new_tab_width).max(0.0);
    let per_tab = if count == 0 {
        0.0
    } else {
        (space / count as f32).floor()
    };
    let tab_width = per_tab
        .clamp(MIN_TAB_CELLS * cell.width, MAX_TAB_CELLS * cell.width)
        .floor();
    let close_width = 2.0 * cell.width;
    let tabs = (0..count)
        .map(|i| {
            let x = i as f32 * tab_width;
            TabRect {
                rect: Rect::new(x, 0.0, tab_width, height),
                close: Rect::new(x + tab_width - close_width, 0.0, close_width, height),
            }
        })
        .collect();
    TabBarLayout {
        tabs,
        new_tab: Rect::new(count as f32 * tab_width, 0.0, new_tab_width, height),
        height,
    }
}

pub fn hit(layout: &TabBarLayout, x: f32, y: f32) -> Hit {
    if y < 0.0 || y >= layout.height {
        return Hit::None;
    }
    for (i, tab) in layout.tabs.iter().enumerate() {
        if tab.close.contains(x, y) {
            return Hit::Close(i);
        }
        if tab.rect.contains(x, y) {
            return Hit::Tab(i);
        }
    }
    if layout.new_tab.contains(x, y) {
        return Hit::NewTab;
    }
    Hit::Bar
}

pub(crate) fn char_cells(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Cuts `title` to `cells` cells. A cut title ends with `…`.
pub fn fit_title(title: &str, cells: usize) -> String {
    let total: usize = title.chars().map(char_cells).sum();
    if total <= cells {
        return title.to_owned();
    }
    if cells == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in title.chars() {
        let w = char_cells(c);
        if used + w > cells - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

pub struct TabBarInput<'a> {
    pub layout: &'a TabBarLayout,
    pub titles: &'a [String],
    pub active: usize,
    pub hover: Hit,
    /// The tab being renamed and the text typed so far.
    pub editing: Option<(usize, &'a str)>,
    /// A colored dot before the title of a tab (for example, the agent state). Empty = no dots.
    pub badges: &'a [Option<Rgb>],
    /// The color of each tab: a line at its top. Empty = no colors.
    pub colors: &'a [Option<Rgb>],
    /// A colored dot and a short text at the right end (for example, unread events). Give `layout_tabs`
    /// the width without `corner_rect`, so the tabs do not cover it.
    pub corner: Option<(&'a str, Rgb)>,
    pub cell: CellMetrics,
    pub width: f32,
}

/// The quads of the tab bar.
pub fn build_tab_bar(
    input: &TabBarInput,
    ui: &UiColors,
    glyph: &mut dyn FnMut(&GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
) -> Result<Vec<Instance>, AtlasFull> {
    let cell = input.cell;
    let layout = input.layout;
    let mut quads = vec![solid(
        Rect::new(0.0, 0.0, input.width, layout.height),
        ui.surface,
    )];
    let mut text = Vec::new();
    let text_y = BAR_PADDING;

    for (i, tab) in layout.tabs.iter().enumerate() {
        let rect = tab.rect;
        let active = i == input.active;
        let hovered = matches!(input.hover, Hit::Tab(h) | Hit::Close(h) if h == i);
        let tab_color = input.colors.get(i).copied().flatten();
        if active {
            quads.push(solid(rect, ui.surface_active));
            if tab_color.is_none() {
                quads.push(solid(Rect::new(rect.x, 0.0, rect.width, 2.0), ui.accent));
            }
        } else if hovered {
            quads.push(solid(rect, ui.overlay));
        }
        if let Some(color) = tab_color {
            quads.push(solid(
                Rect::new(rect.x, 0.0, rect.width, TAB_COLOR_LINE),
                color,
            ));
        }

        // Title: 1 cell of space on the left, the × button on the right.
        let title_cells = ((rect.width / cell.width) as usize).saturating_sub(3);
        let color = if active { ui.text } else { ui.text_dim };
        let x0 = rect.x + cell.width;
        match input.editing {
            Some((edited, typed)) if edited == i => {
                let shown = fit_title(typed, title_cells.saturating_sub(1));
                let used = push_text(&mut text, &shown, x0, text_y, cell, ui.text, glyph)?;
                // A text cursor after the typed text.
                quads.push(solid(
                    Rect::new(x0 + used as f32 * cell.width, text_y, 2.0, cell.height),
                    ui.text,
                ));
            }
            _ => match input.badges.get(i).copied().flatten() {
                Some(badge) => {
                    // A round-ish dot in the first title cell, then the title.
                    let size = (cell.width * 0.5).round().max(3.0);
                    quads.push(solid(
                        Rect::new(
                            x0 + (cell.width - size) / 2.0,
                            text_y + (cell.height - size) / 2.0,
                            size,
                            size,
                        ),
                        badge,
                    ));
                    let title = fit_title(&input.titles[i], title_cells.saturating_sub(1));
                    push_text(
                        &mut text,
                        &title,
                        x0 + cell.width,
                        text_y,
                        cell,
                        color,
                        glyph,
                    )?;
                }
                None => {
                    let title = fit_title(&input.titles[i], title_cells);
                    push_text(&mut text, &title, x0, text_y, cell, color, glyph)?;
                }
            },
        }

        let close_color = if input.hover == Hit::Close(i) {
            ui.text
        } else {
            ui.text_dim
        };
        let close_x = tab.close.x + (tab.close.width - cell.width) / 2.0;
        push_text(&mut text, "×", close_x, text_y, cell, close_color, glyph)?;
    }

    if let Some((label, color)) = input.corner {
        let rect = corner_rect(label, input.width, cell);
        let size = (cell.width * 0.5).round().max(3.0);
        quads.push(solid(
            Rect::new(
                rect.x + (cell.width - size) / 2.0,
                text_y + (cell.height - size) / 2.0,
                size,
                size,
            ),
            color,
        ));
        push_text(
            &mut text,
            label,
            rect.x + 2.0 * cell.width,
            text_y,
            cell,
            ui.text_dim,
            glyph,
        )?;
    }

    let plus = layout.new_tab;
    if input.hover == Hit::NewTab {
        quads.push(solid(plus, ui.overlay));
    }
    let plus_x = plus.x + (plus.width - cell.width) / 2.0;
    push_text(&mut text, "+", plus_x, text_y, cell, ui.text_dim, glyph)?;

    quads.extend(text);
    Ok(quads)
}

/// Adds glyphs for `text` from `(x, y)` (the top-left of the first cell). Returns the cells used.
/// The place of the corner text (`TabBarInput::corner`): a dot, a space, the text, and a space.
pub fn corner_rect(label: &str, width: f32, cell: CellMetrics) -> Rect {
    let cells: usize = label.chars().map(char_cells).sum();
    let w = (cells + 3) as f32 * cell.width;
    Rect::new(width - w, 0.0, w, bar_height(cell))
}

pub(crate) fn push_text(
    out: &mut Vec<Instance>,
    text: &str,
    x: f32,
    y: f32,
    cell: CellMetrics,
    color: Rgb,
    glyph: &mut dyn FnMut(&GlyphKey) -> Result<Option<AtlasGlyph>, AtlasFull>,
) -> Result<usize, AtlasFull> {
    let mut col = 0;
    for c in text.chars() {
        let width = char_cells(c);
        if c != ' ' {
            let key = GlyphKey {
                c,
                extra: None,
                bold: false,
                italic: false,
                wide: width == 2,
                dots: false,
            };
            if let Some(g) = glyph(&key)? {
                let cx = x + col as f32 * cell.width;
                out.push(Instance {
                    rect: [
                        cx + g.left as f32,
                        y + cell.baseline - g.top as f32,
                        g.width as f32,
                        g.height as f32,
                    ],
                    uv: [g.x as f32, g.y as f32, g.width as f32, g.height as f32],
                    color: linear(color),
                    kind: if g.color {
                        KIND_COLOR_GLYPH
                    } else {
                        KIND_GLYPH
                    },
                    _pad: [0; 3],
                });
            }
        }
        col += width.max(1);
    }
    Ok(col)
}

pub(crate) fn solid(rect: Rect, color: Rgb) -> Instance {
    Instance {
        rect: [rect.x, rect.y, rect.width, rect.height],
        uv: [0.0; 4],
        color: linear(color),
        kind: KIND_SOLID,
        _pad: [0; 3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> UiColors {
        UiColors::default()
    }

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

    #[test]
    fn bar_is_one_cell_plus_padding() {
        assert_eq!(bar_height(CELL), 20.0 + 2.0 * BAR_PADDING);
    }

    #[test]
    fn few_tabs_get_the_max_width() {
        let layout = layout_tabs(2, 1000.0, CELL);
        assert_eq!(layout.tabs.len(), 2);
        assert_eq!(layout.tabs[0].rect.width, MAX_TAB_CELLS * CELL.width);
        assert_eq!(layout.tabs[1].rect.x, layout.tabs[0].rect.width);
        // The + button is right after the last tab.
        assert_eq!(layout.new_tab.x, 2.0 * MAX_TAB_CELLS * CELL.width);
        assert_eq!(layout.new_tab.width, NEW_TAB_CELLS * CELL.width);
    }

    #[test]
    fn many_tabs_get_narrower_but_not_below_the_min() {
        let layout = layout_tabs(10, 1000.0, CELL);
        let w = layout.tabs[0].rect.width;
        assert!(w < MAX_TAB_CELLS * CELL.width);
        // All tabs and the + button fit in the bar.
        assert!(layout.new_tab.x + layout.new_tab.width <= 1000.0);
        let crowded = layout_tabs(100, 1000.0, CELL);
        assert_eq!(crowded.tabs[0].rect.width, MIN_TAB_CELLS * CELL.width);
    }

    #[test]
    fn tab_widths_are_whole_pixels() {
        let layout = layout_tabs(7, 997.0, CELL);
        for tab in &layout.tabs {
            assert_eq!(tab.rect.width.fract(), 0.0);
            assert_eq!(tab.rect.x.fract(), 0.0);
        }
    }

    #[test]
    fn close_button_is_at_the_right_end_of_the_tab() {
        let layout = layout_tabs(1, 1000.0, CELL);
        let tab = &layout.tabs[0];
        assert_eq!(tab.close.x + tab.close.width, tab.rect.x + tab.rect.width);
        assert_eq!(tab.close.width, 2.0 * CELL.width);
    }

    #[test]
    fn hit_test() {
        let layout = layout_tabs(2, 1000.0, CELL);
        let y = 10.0;
        assert_eq!(hit(&layout, 5.0, y), Hit::Tab(0));
        assert_eq!(hit(&layout, 305.0, y), Hit::Tab(1));
        let close = layout.tabs[1].close;
        assert_eq!(hit(&layout, close.x + 1.0, y), Hit::Close(1));
        assert_eq!(hit(&layout, layout.new_tab.x + 1.0, y), Hit::NewTab);
        assert_eq!(hit(&layout, 900.0, y), Hit::Bar);
        assert_eq!(hit(&layout, 5.0, layout.height + 1.0), Hit::None);
    }

    #[test]
    fn titles_are_cut_with_an_ellipsis() {
        assert_eq!(fit_title("pwsh", 10), "pwsh");
        assert_eq!(fit_title("a very long title", 8), "a very …");
        assert_eq!(fit_title("abc", 0), "");
        // Wide chars take 2 cells.
        assert_eq!(fit_title("界界界界", 5), "界界…");
    }

    fn build(
        titles: &[&str],
        active: usize,
        hover: Hit,
        editing: Option<(usize, &str)>,
    ) -> Vec<Instance> {
        let layout = layout_tabs(titles.len(), 1000.0, CELL);
        let titles: Vec<String> = titles.iter().map(|t| t.to_string()).collect();
        let input = TabBarInput {
            layout: &layout,
            titles: &titles,
            active,
            hover,
            editing,
            badges: &[],
            colors: &[],
            corner: None,
            cell: CELL,
            width: 1000.0,
        };
        build_tab_bar(&input, &ui(), &mut |_| Ok(Some(GLYPH))).unwrap()
    }

    fn solid_with(quads: &[Instance], color: Rgb) -> Vec<[f32; 4]> {
        quads
            .iter()
            .filter(|q| q.kind == KIND_SOLID && q.color == linear(color))
            .map(|q| q.rect)
            .collect()
    }

    #[test]
    fn bar_background_covers_the_width() {
        let quads = build(&["a"], 0, Hit::None, None);
        assert_eq!(
            solid_with(&quads, ui().surface)[0],
            [0.0, 0.0, 1000.0, bar_height(CELL)]
        );
    }

    #[test]
    fn active_tab_has_its_own_background_and_an_accent_line() {
        let quads = build(&["a", "b"], 1, Hit::None, None);
        let active = solid_with(&quads, ui().surface_active);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0][0], MAX_TAB_CELLS * CELL.width);
        let accent = solid_with(&quads, ui().accent);
        assert_eq!(accent.len(), 1);
        assert_eq!(accent[0][1], 0.0, "on top");
        assert!(accent[0][3] <= 3.0, "thin");
    }

    #[test]
    fn hovered_tab_is_lighter() {
        let quads = build(&["a", "b"], 0, Hit::Tab(1), None);
        assert_eq!(solid_with(&quads, ui().overlay).len(), 1);
    }

    #[test]
    fn titles_and_buttons_are_glyphs() {
        // "ab" + "cd" + 2 × + 1 + = 7 glyphs.
        let quads = build(&["ab", "cd"], 0, Hit::None, None);
        let glyphs: Vec<&Instance> = quads.iter().filter(|q| q.kind == KIND_GLYPH).collect();
        assert_eq!(glyphs.len(), 7);
        // Text of the active tab is brighter.
        assert_eq!(glyphs[0].color, linear(ui().text));
    }

    #[test]
    fn editing_shows_the_typed_text_and_a_cursor() {
        let quads = build(&["old"], 0, Hit::None, Some((0, "new name")));
        let glyphs = quads.iter().filter(|q| q.kind == KIND_GLYPH).count();
        // "new name" has 7 non-space chars, plus × and +.
        assert_eq!(glyphs, 7 + 2);
        assert_eq!(solid_with(&quads, ui().text).len(), 1, "a text cursor");
    }

    #[test]
    fn a_badge_is_a_dot_and_moves_the_title() {
        let layout = layout_tabs(2, 1000.0, CELL);
        let titles = vec!["ab".to_owned(), "cd".to_owned()];
        let red = Rgb { r: 255, g: 0, b: 0 };
        let badges = [None, Some(red)];
        let input = TabBarInput {
            layout: &layout,
            titles: &titles,
            active: 0,
            hover: Hit::None,
            editing: None,
            badges: &badges,
            colors: &[],
            corner: None,
            cell: CELL,
            width: 1000.0,
        };
        let quads = build_tab_bar(&input, &ui(), &mut |_| Ok(Some(GLYPH))).unwrap();
        let dots = solid_with(&quads, red);
        assert_eq!(dots.len(), 1);
        let tab1 = layout.tabs[1].rect;
        assert!(dots[0][0] >= tab1.x && dots[0][0] < tab1.x + CELL.width * 2.0);
        assert!(dots[0][2] < CELL.width, "a small dot");
        // The title of tab 2 starts one cell later than the title of tab 1.
        let glyphs: Vec<&Instance> = quads.iter().filter(|q| q.kind == KIND_GLYPH).collect();
        let first_tab_title = glyphs[0].rect[0] - layout.tabs[0].rect.x;
        let second_tab_title = glyphs
            .iter()
            .map(|g| g.rect[0] - tab1.x)
            .find(|x| *x > 0.0 && *x < 3.0 * CELL.width)
            .unwrap();
        assert_eq!(second_tab_title - first_tab_title, CELL.width);
    }

    #[test]
    fn a_colored_tab_has_a_line_at_the_top() {
        let layout = layout_tabs(3, 1000.0, CELL);
        let titles = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let red = Rgb {
            r: 243,
            g: 139,
            b: 168,
        };
        let green = Rgb {
            r: 166,
            g: 227,
            b: 161,
        };
        let colors = [Some(red), None, Some(green)];
        let input = TabBarInput {
            layout: &layout,
            titles: &titles,
            active: 0,
            hover: Hit::None,
            editing: None,
            badges: &[],
            colors: &colors,
            corner: None,
            cell: CELL,
            width: 1000.0,
        };
        let quads = build_tab_bar(&input, &ui(), &mut |_| Ok(Some(GLYPH))).unwrap();
        // An inactive tab: a line of its color across its top.
        let tab2 = layout.tabs[2].rect;
        assert_eq!(
            solid_with(&quads, green),
            vec![[tab2.x, 0.0, tab2.width, TAB_COLOR_LINE]]
        );
        // The active tab: its color takes the place of the accent line.
        let tab0 = layout.tabs[0].rect;
        assert_eq!(
            solid_with(&quads, red),
            vec![[tab0.x, 0.0, tab0.width, TAB_COLOR_LINE]]
        );
        assert!(solid_with(&quads, ui().accent).is_empty());
        // The text stays as it was.
        assert!(solid_with(&quads, ui().surface_active).len() == 1);
    }

    #[test]
    fn the_corner_text_is_on_the_right() {
        let rect = corner_rect("3", 1000.0, CELL);
        assert_eq!(rect.x + rect.width, 1000.0);
        assert_eq!(
            rect.width,
            4.0 * CELL.width,
            "a dot, a space, the text, and some space"
        );
        let layout = layout_tabs(1, 1000.0 - rect.width, CELL);
        let titles = vec!["ab".to_owned()];
        let pink = Rgb {
            r: 250,
            g: 0,
            b: 250,
        };
        let input = TabBarInput {
            layout: &layout,
            titles: &titles,
            active: 0,
            hover: Hit::None,
            editing: None,
            badges: &[],
            colors: &[],
            corner: Some(("3", pink)),
            cell: CELL,
            width: 1000.0,
        };
        let quads = build_tab_bar(&input, &ui(), &mut |_| Ok(Some(GLYPH))).unwrap();
        let dots = solid_with(&quads, pink);
        assert_eq!(dots.len(), 1);
        assert!(dots[0][0] >= rect.x);
        let glyph_xs: Vec<f32> = quads
            .iter()
            .filter(|q| q.kind == KIND_GLYPH)
            .map(|q| q.rect[0])
            .collect();
        assert!(
            glyph_xs.iter().any(|x| *x >= rect.x),
            "the number is drawn in the corner"
        );
    }

    #[test]
    fn a_custom_theme_colors_the_bar() {
        let red = Rgb { r: 255, g: 0, b: 0 };
        let custom = UiColors {
            surface: red,
            ..UiColors::default()
        };
        let layout = layout_tabs(1, 1000.0, CELL);
        let titles = vec!["ab".to_owned()];
        let input = TabBarInput {
            layout: &layout,
            titles: &titles,
            active: 0,
            hover: Hit::None,
            editing: None,
            badges: &[],
            colors: &[],
            corner: None,
            cell: CELL,
            width: 1000.0,
        };
        let quads = build_tab_bar(&input, &custom, &mut |_| Ok(Some(GLYPH))).unwrap();
        assert_eq!(solid_with(&quads, red).len(), 1, "the bar background");
        assert!(solid_with(&quads, ui().surface).is_empty());
    }
}
