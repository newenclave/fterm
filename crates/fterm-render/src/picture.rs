//! Pictures on the cells of a pane, with no GPU code: where a picture is drawn, the part of the pane it may cover,
//! and the Braille form of a picture (2×4 dots and one color per cell).

use fterm_term::alacritty_terminal::event::EventListener;
use fterm_term::alacritty_terminal::grid::Dimensions;
use fterm_term::alacritty_terminal::term::Term;

use crate::font::CellMetrics;
use crate::frame::Rect;

/// How pictures are drawn (`pictures` in the config).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PictureMode {
    /// Real pixels (a texture).
    #[default]
    Pixels,
    /// Braille dots in the cells of the picture.
    Braille,
    /// Not drawn.
    Off,
}

/// The pictures whose cells are on the screen now: (id, screen row of the top-left cell, column). The row can be
/// negative (the top of a picture is scrolled away); `max_rows` is the height of the highest picture.
pub fn visible_marks<T: EventListener>(term: &Term<T>, max_rows: usize) -> Vec<(u32, i32, usize)> {
    let offset = term.grid().display_offset() as i32;
    let screen = term.screen_lines() as i32;
    // Screen row = grid line + offset.
    let top = -offset - max_rows as i32 + 1;
    let bottom = screen - offset;
    fterm_term::picture::marks(term, top..bottom)
        .into_iter()
        .map(|(id, line, column)| (id, line + offset, column))
        .collect()
}

/// The rect of a picture (`columns`×`rows` cells, its top-left cell at screen `row`, `column`) in a pane drawn in
/// `area`, and the clip: the part of the pane with cells. `None` when no part of it is on the screen.
pub fn place(
    row: i32,
    column: usize,
    (columns, rows): (usize, usize),
    cell: CellMetrics,
    area: Rect,
    padding: f32,
    screen: (usize, usize),
) -> Option<(Rect, Rect)> {
    let clip = Rect::new(
        area.x + padding,
        area.y + padding,
        (screen.0 as f32 * cell.width).min(area.width - padding),
        (screen.1 as f32 * cell.height).min(area.height - padding),
    );
    let rect = Rect::new(
        clip.x + column as f32 * cell.width,
        clip.y + row as f32 * cell.height,
        columns as f32 * cell.width,
        rows as f32 * cell.height,
    );
    let visible = rect.x < clip.x + clip.width
        && rect.x + rect.width > clip.x
        && rect.y < clip.y + clip.height
        && rect.y + rect.height > clip.y;
    visible.then_some((rect, clip))
}

/// A clip rect in whole pixels for `set_scissor_rect`, inside a `view` of `size`. `None` when it is empty.
pub fn scissor(clip: Rect, size: (u32, u32)) -> Option<[u32; 4]> {
    let x0 = clip.x.max(0.0).floor() as u32;
    let y0 = clip.y.max(0.0).floor() as u32;
    let x1 = ((clip.x + clip.width).ceil().max(0.0) as u32).min(size.0);
    let y1 = ((clip.y + clip.height).ceil().max(0.0) as u32).min(size.1);
    (x1 > x0 && y1 > y0).then(|| [x0, y0, x1 - x0, y1 - y0])
}

/// One cell of the Braille form: its Braille char and its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrailleCell {
    pub column: usize,
    pub row: usize,
    pub c: char,
    pub color: [u8; 3],
}

/// The bits of the 8 dots of a Braille char: (x, y) -> bit (U+2800 + bits).
const DOTS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// The Braille form of an RGBA picture on `columns`×`rows` cells. In each cell a dot is on when its part of the
/// picture is brighter than the mean of the cell, or when the cell has little contrast (and the dot is not too dark
/// or too clear); the cell gets the mean color of its dots that are on. Empty cells are left out.
pub fn braille(
    rgba: &[u8],
    (width, height): (u32, u32),
    (columns, rows): (usize, usize),
) -> Vec<BrailleCell> {
    let mut out = Vec::new();
    if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
        return out;
    }
    let (dots_w, dots_h) = (columns * 2, rows * 4);
    // The mean color (and alpha) of the part of the picture under dot (dx, dy).
    let dot = |dx: usize, dy: usize| -> [f32; 4] {
        let x0 = dx as u64 * u64::from(width) / dots_w as u64;
        let x1 = ((dx as u64 + 1) * u64::from(width) / dots_w as u64).max(x0 + 1);
        let y0 = dy as u64 * u64::from(height) / dots_h as u64;
        let y1 = ((dy as u64 + 1) * u64::from(height) / dots_h as u64).max(y0 + 1);
        let mut sum = [0f32; 4];
        let mut n = 0f32;
        for y in y0..y1.min(u64::from(height)) {
            for x in x0..x1.min(u64::from(width)) {
                let i = ((y * u64::from(width) + x) * 4) as usize;
                let a = f32::from(rgba[i + 3]) / 255.0;
                for (c, s) in sum.iter_mut().take(3).enumerate() {
                    *s += f32::from(rgba[i + c]) * a;
                }
                sum[3] += a;
                n += 1.0;
            }
        }
        if n == 0.0 {
            return [0.0; 4];
        }
        // Premultiplied sums back to a color; alpha as the mean.
        let a = sum[3];
        if a == 0.0 {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            [sum[0] / a, sum[1] / a, sum[2] / a, a / n]
        }
    };
    let luma = |c: [f32; 4]| (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]) / 255.0 * c[3];
    for row in 0..rows {
        for column in 0..columns {
            let mut cell = [[[0f32; 4]; 4]; 2];
            let (mut mean, mut low, mut high) = (0.0, f32::MAX, f32::MIN);
            for (x, col) in cell.iter_mut().enumerate() {
                for (y, d) in col.iter_mut().enumerate() {
                    *d = dot(column * 2 + x, row * 4 + y);
                    let l = luma(*d);
                    mean += l;
                    low = low.min(l);
                    high = high.max(l);
                }
            }
            mean /= 8.0;
            // Little contrast (a flat part or a soft gradient): all dots, so there are no stripes.
            let flat = high - low < 0.15;
            let (mut bits, mut color, mut lit) = (0u32, [0f32; 3], 0f32);
            for (x, col) in cell.iter().enumerate() {
                for (y, d) in col.iter().enumerate() {
                    let l = luma(*d);
                    if (flat || l >= mean) && l > 0.06 && d[3] > 0.25 {
                        bits |= DOTS[x][y];
                        for (c, v) in color.iter_mut().enumerate() {
                            *v += d[c];
                        }
                        lit += 1.0;
                    }
                }
            }
            if bits == 0 {
                continue;
            }
            out.push(BrailleCell {
                column,
                row,
                c: char::from_u32(0x2800 + bits).unwrap_or(' '),
                color: color.map(|v| (v / lit).round().clamp(0.0, 255.0) as u8),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: CellMetrics = CellMetrics {
        width: 10.0,
        height: 20.0,
        baseline: 15.0,
    };
    const AREA: Rect = Rect {
        x: 100.0,
        y: 50.0,
        width: 400.0,
        height: 200.0,
    };

    #[test]
    fn a_picture_on_the_screen() {
        // Padding 4: the cells start at (104, 54). The picture is 5×2 cells at row 3, column 2.
        let (rect, clip) = place(3, 2, (5, 2), CELL, AREA, 4.0, (39, 9)).unwrap();
        assert_eq!(rect, Rect::new(124.0, 114.0, 50.0, 40.0));
        assert_eq!(clip, Rect::new(104.0, 54.0, 390.0, 180.0));
    }

    #[test]
    fn a_picture_half_scrolled_away_is_still_drawn_and_cut() {
        // Its top row is one row above the screen.
        let (rect, clip) = place(-1, 0, (5, 3), CELL, AREA, 0.0, (40, 10)).unwrap();
        assert_eq!(rect.y, 30.0, "above the pane");
        assert_eq!(clip.y, 50.0, "the clip cuts it at the top of the pane");
    }

    #[test]
    fn a_picture_out_of_the_screen_is_not_drawn() {
        assert_eq!(place(-3, 0, (5, 3), CELL, AREA, 0.0, (40, 10)), None);
        assert_eq!(place(10, 0, (5, 3), CELL, AREA, 0.0, (40, 10)), None);
        assert!(place(-2, 0, (5, 3), CELL, AREA, 0.0, (40, 10)).is_some());
        assert!(place(9, 0, (5, 3), CELL, AREA, 0.0, (40, 10)).is_some());
    }

    #[test]
    fn the_scissor_is_inside_the_view() {
        assert_eq!(
            scissor(Rect::new(10.5, 20.0, 100.0, 50.2), (800, 600)),
            Some([10, 20, 101, 51])
        );
        assert_eq!(
            scissor(Rect::new(-5.0, 590.0, 100.0, 50.0), (800, 600)),
            Some([0, 590, 95, 10])
        );
        assert_eq!(scissor(Rect::new(900.0, 0.0, 10.0, 10.0), (800, 600)), None);
    }

    fn picture(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        for y in 0..h {
            for x in 0..w {
                out.extend(px(x, y));
            }
        }
        out
    }

    #[test]
    fn a_white_picture_is_full_braille_cells() {
        let rgba = picture(4, 8, |_, _| [255, 255, 255, 255]);
        let cells = braille(&rgba, (4, 8), (2, 2));
        assert_eq!(cells.len(), 4);
        assert!(
            cells
                .iter()
                .all(|c| c.c == '⣿' && c.color == [255, 255, 255])
        );
    }

    #[test]
    fn a_black_or_clear_picture_has_no_dots() {
        assert!(braille(&picture(4, 8, |_, _| [0, 0, 0, 255]), (4, 8), (2, 2)).is_empty());
        assert!(braille(&picture(4, 8, |_, _| [255, 255, 255, 0]), (4, 8), (2, 2)).is_empty());
    }

    #[test]
    fn the_bright_half_of_a_cell_is_on() {
        // One cell: the left half is red, the right half is dark.
        let rgba = picture(2, 4, |x, _| {
            if x == 0 {
                [255, 0, 0, 255]
            } else {
                [10, 10, 10, 255]
            }
        });
        let cells = braille(&rgba, (2, 4), (1, 1));
        assert_eq!(cells.len(), 1);
        // The left column of dots: 1, 2, 3, 7 = 0x47.
        assert_eq!(cells[0].c, '\u{2847}');
        assert_eq!(
            cells[0].color,
            [255, 0, 0],
            "the color of the dots that are on"
        );
    }

    #[test]
    fn a_soft_gradient_fills_its_cells() {
        // A cell with little contrast (a smooth gradient) is full, not half: no stripes.
        let rgba = picture(2, 4, |_, y| [100 + y as u8 * 8, 100, 200, 255]);
        let cells = braille(&rgba, (2, 4), (1, 1));
        assert_eq!(cells[0].c, '⣿');
    }

    #[test]
    fn a_big_picture_on_few_cells() {
        // 100×100 px on 3×2 cells: each dot is a mean of its part.
        let rgba = picture(100, 100, |_, y| {
            if y < 50 {
                [0, 200, 0, 255]
            } else {
                [0, 0, 0, 255]
            }
        });
        let cells = braille(&rgba, (100, 100), (3, 2));
        assert_eq!(cells.len(), 3, "only the top row has dots");
        assert!(cells.iter().all(|c| c.row == 0 && c.c == '⣿'));
    }

    #[test]
    fn marks_on_the_screen_and_in_the_scrolled_back_view() {
        use fterm_term::alacritty_terminal::event::VoidListener;
        use fterm_term::alacritty_terminal::grid::Scroll;
        use fterm_term::alacritty_terminal::term::Config;
        use fterm_term::alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
        use fterm_term::size::GridSize;
        let mut term = Term::new(
            Config {
                scrolling_history: 100,
                ..Config::default()
            },
            &GridSize::new(20, 5),
            VoidListener,
        );
        let mark = format!(
            "\x1b]8;id=a;{}\x1b\\P\x1b]8;;\x1b\\",
            fterm_term::osc::picture_uri(7)
        );
        let mut parser = Processor::<StdSyncHandler>::new();
        parser.advance(&mut term, format!("{mark}\r\n").as_bytes());
        parser.advance(&mut term, "x\r\n".repeat(6).as_bytes());
        // The mark is 3 lines above the screen now (grid line -3).
        assert_eq!(visible_marks(&term, 1), []);
        assert_eq!(visible_marks(&term, 3), []);
        assert_eq!(
            visible_marks(&term, 4),
            [(7, -3, 0)],
            "a picture of 4 rows still shows its last row"
        );
        // Scrolled back by 3 lines: it is on the top row.
        term.scroll_display(Scroll::Delta(3));
        assert_eq!(visible_marks(&term, 1), [(7, 0, 0)]);
    }

    #[test]
    fn a_bad_buffer_gives_nothing() {
        assert!(braille(&[1, 2, 3], (4, 8), (2, 2)).is_empty());
        assert!(braille(&[], (0, 0), (2, 2)).is_empty());
    }
}
