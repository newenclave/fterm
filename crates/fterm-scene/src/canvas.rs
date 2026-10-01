//! The canvas: dots, shapes, text, and colors.

use serde::{Deserialize, Serialize};

/// A color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// `#rrggbb` (or `rrggbb`).
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#').unwrap_or(text);
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self::new(byte(0)?, byte(2)?, byte(4)?))
    }
}

/// What one cell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// A Braille char (`U+2800 + dots`), a space for no dots, or the text char.
    pub ch: char,
    /// The color of the cell (`None` = the terminal's text color).
    pub color: Option<Rgb>,
}

/// The dot bits of a cell (tank_rs): left column `0x01 0x02 0x04 0x40`, right column `0x08 0x10 0x20 0x80`.
const BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canvas {
    cols: usize,
    rows: usize,
    dots: Vec<u8>,
    text: Vec<Option<char>>,
    colors: Vec<Option<Rgb>>,
    /// The color for the next shapes and text.
    pen: Option<Rgb>,
}

impl Canvas {
    pub fn new(cols: usize, rows: usize) -> Self {
        let n = cols * rows;
        Self {
            cols,
            rows,
            dots: vec![0; n],
            text: vec![None; n],
            colors: vec![None; n],
            pen: None,
        }
    }

    /// The cell and the bit of a dot, when the dot is on the canvas.
    fn place(&self, x: i32, y: i32) -> Option<(usize, u8)> {
        let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        if x >= self.width() || y >= self.height() {
            return None;
        }
        Some(((y / 4) * self.cols + x / 2, BITS[x % 2][y % 4]))
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Width in dots.
    pub fn width(&self) -> usize {
        self.cols * 2
    }

    /// Height in dots.
    pub fn height(&self) -> usize {
        self.rows * 4
    }

    /// The color for the next shapes and text (`None` = the terminal's text color).
    pub fn set_pen(&mut self, color: Option<Rgb>) {
        self.pen = color;
    }

    /// Puts a dot. A dot outside the canvas is not drawn.
    pub fn set(&mut self, x: i32, y: i32) {
        if let Some((i, bit)) = self.place(x, y) {
            self.dots[i] |= bit;
            self.colors[i] = self.pen;
        }
    }

    /// Takes a dot away.
    pub fn unset(&mut self, x: i32, y: i32) {
        if let Some((i, bit)) = self.place(x, y) {
            self.dots[i] &= !bit;
        }
    }

    pub fn get(&self, x: i32, y: i32) -> bool {
        self.place(x, y)
            .is_some_and(|(i, bit)| self.dots[i] & bit != 0)
    }

    /// A line from one dot to another (both ends too), with Bresenham's steps.
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        // Only the part on the canvas: a line from the API can be very long.
        let Some((x0, y0, x1, y1)) = self.clip_line(x0, y0, x1, y1) else {
            return;
        };
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        loop {
            self.set(clamp(x), clamp(y));
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// The part of a line that is on the canvas (Liang–Barsky), or `None`.
    fn clip_line(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> Option<(i64, i64, i64, i64)> {
        if self.width() == 0 || self.height() == 0 {
            return None;
        }
        let (fx0, fy0) = (f64::from(x0), f64::from(y0));
        let (dx, dy) = (f64::from(x1) - fx0, f64::from(y1) - fy0);
        let (max_x, max_y) = ((self.width() - 1) as f64, (self.height() - 1) as f64);
        let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
        for (p, q) in [(-dx, fx0), (dx, max_x - fx0), (-dy, fy0), (dy, max_y - fy0)] {
            if p == 0.0 {
                if q < 0.0 {
                    return None;
                }
            } else {
                let t = q / p;
                if p < 0.0 {
                    t0 = t0.max(t);
                } else {
                    t1 = t1.min(t);
                }
            }
        }
        if t0 > t1 {
            return None;
        }
        let at = |t: f64| ((fx0 + t * dx).round() as i64, (fy0 + t * dy).round() as i64);
        let ((ax, ay), (bx, by)) = (at(t0), at(t1));
        Some((ax, ay, bx, by))
    }

    /// A rectangle `w`×`h` dots with its top left corner at `x`, `y`.
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, fill: bool) {
        if w <= 0 || h <= 0 {
            return;
        }
        let (x1, y1) = (x.saturating_add(w - 1), y.saturating_add(h - 1));
        // Only the rows on the canvas.
        let top = y.max(0);
        let bottom = y1.min(self.height() as i32 - 1);
        for py in top..=bottom {
            if fill || py == y || py == y1 {
                self.line(x, py, x1, py);
            } else {
                self.set(x, py);
                self.set(x1, py);
            }
        }
    }

    /// A circle around `cx`, `cy` with the radius `r` (in dots).
    pub fn circle(&mut self, cx: i32, cy: i32, r: i32, fill: bool) {
        if r < 0 {
            return;
        }
        let (w, h) = (self.width() as i64, self.height() as i64);
        let (cx64, cy64, r64) = (i64::from(cx), i64::from(cy), i64::from(r));
        // Only the dots of the canvas that the circle can touch.
        let dots_in_box = || {
            let (x0, x1) = ((cx64 - r64).max(0), (cx64 + r64).min(w - 1));
            let (y0, y1) = ((cy64 - r64).max(0), (cy64 + r64).min(h - 1));
            (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (x, y)))
        };
        if fill {
            let r2 = r64 * r64 + r64;
            for (x, y) in dots_in_box() {
                let (dx, dy) = (x - cx64, y - cy64);
                if dx * dx + dy * dy <= r2 {
                    self.set(x as i32, y as i32);
                }
            }
            return;
        }
        if r64 > 2 * (w + h) {
            // A big circle: each dot of the canvas is on it when its distance rounds to `r`.
            // In i128: (2r)² does not fit in i64 for a big `r`.
            let r128 = i128::from(r64);
            let (inner, outer) = ((2 * r128 - 1).pow(2), (2 * r128 + 1).pow(2));
            for (x, y) in dots_in_box() {
                let (dx, dy) = (i128::from(x - cx64), i128::from(y - cy64));
                let d4 = 4 * (dx * dx + dy * dy);
                if (inner..outer).contains(&d4) {
                    self.set(x as i32, y as i32);
                }
            }
            return;
        }
        // The midpoint circle: one eighth, mirrored.
        let (mut x, mut y, mut err) = (r, 0, 1 - r);
        while x >= y {
            for (px, py) in [
                (x, y),
                (y, x),
                (-y, x),
                (-x, y),
                (-x, -y),
                (-y, -x),
                (y, -x),
                (x, -y),
            ] {
                self.set(cx.saturating_add(px), cy.saturating_add(py));
            }
            y += 1;
            if err < 0 {
                err += 2 * y + 1;
            } else {
                x -= 1;
                err += 2 * (y - x) + 1;
            }
        }
    }

    /// Text in cells (not dots), from the cell `col`, `row`. It covers the dots of those cells.
    pub fn text(&mut self, col: i32, row: i32, text: &str) {
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        if row >= self.rows {
            return;
        }
        for (i, ch) in text.chars().enumerate() {
            let Ok(c) = usize::try_from(i64::from(col) + i as i64) else {
                continue;
            };
            if c >= self.cols {
                break;
            }
            let at = row * self.cols + c;
            self.text[at] = Some(safe_char(ch));
            self.colors[at] = self.pen;
        }
    }

    /// No dots, no text, no colors.
    pub fn clear(&mut self) {
        *self = Self {
            pen: self.pen,
            ..Self::new(self.cols, self.rows)
        };
    }

    /// A new size; what fits stays.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let mut new = Self {
            pen: self.pen,
            ..Self::new(cols, rows)
        };
        for row in 0..rows.min(self.rows) {
            for col in 0..cols.min(self.cols) {
                let (from, to) = (row * self.cols + col, row * cols + col);
                new.dots[to] = self.dots[from];
                new.text[to] = self.text[from];
                new.colors[to] = self.colors[from];
            }
        }
        *self = new;
    }

    pub fn cell(&self, col: usize, row: usize) -> Cell {
        if col >= self.cols || row >= self.rows {
            return Cell {
                ch: ' ',
                color: None,
            };
        }
        let i = row * self.cols + col;
        let ch = match (self.text[i], self.dots[i]) {
            (Some(ch), _) => ch,
            (None, 0) => ' ',
            (None, bits) => char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' '),
        };
        Cell {
            ch,
            color: self.colors[i],
        }
    }

    /// The chars of each row (for a test, or a plain text copy).
    pub fn rows_text(&self) -> Vec<String> {
        (0..self.rows)
            .map(|row| (0..self.cols).map(|col| self.cell(col, row).ch).collect())
            .collect()
    }
}

/// A char that is safe in one cell: a control char (it could start an escape sequence) is a space,
/// and a char of two cells (CJK, emoji) is `?`, so the row does not move.
fn safe_char(ch: char) -> char {
    use unicode_width::UnicodeWidthChar;
    if ch.is_control() {
        ' '
    } else if ch.width() == Some(1) {
        ch
    } else {
        '?'
    }
}

/// A line far outside the canvas still ends: its dots go to the edge of `i32`.
fn clamp(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dots(c: &Canvas) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for y in 0..c.height() as i32 {
            for x in 0..c.width() as i32 {
                if c.get(x, y) {
                    out.push((x, y));
                }
            }
        }
        out
    }

    #[test]
    fn a_cell_has_two_by_four_dots() {
        let c = Canvas::new(3, 2);
        assert_eq!((c.cols(), c.rows(), c.width(), c.height()), (3, 2, 6, 8));
    }

    #[test]
    fn each_dot_has_its_bit() {
        for (x, y, bit) in [
            (0, 0, 0x01),
            (0, 1, 0x02),
            (0, 2, 0x04),
            (0, 3, 0x40),
            (1, 0, 0x08),
            (1, 1, 0x10),
            (1, 2, 0x20),
            (1, 3, 0x80),
        ] {
            let mut c = Canvas::new(1, 1);
            c.set(x, y);
            assert_eq!(
                c.cell(0, 0).ch,
                char::from_u32(0x2800 + bit).unwrap(),
                "{x},{y}"
            );
        }
        let mut c = Canvas::new(2, 1);
        for y in 0..4 {
            c.set(2, y);
            c.set(3, y);
        }
        assert_eq!(c.cell(1, 0).ch, '⣿');
        // A cell with no dots is a space (it copies well and draws nothing).
        assert_eq!(c.cell(0, 0).ch, ' ');
    }

    #[test]
    fn dots_outside_are_not_drawn() {
        let mut c = Canvas::new(2, 2);
        for (x, y) in [(-1, 0), (0, -1), (4, 0), (0, 8), (i32::MAX, i32::MIN)] {
            c.set(x, y);
            assert!(!c.get(x, y));
        }
        assert!(dots(&c).is_empty());
    }

    #[test]
    fn a_dot_can_go_away() {
        let mut c = Canvas::new(1, 1);
        c.set(1, 2);
        c.set(0, 0);
        c.unset(1, 2);
        assert_eq!(dots(&c), [(0, 0)]);
    }

    #[test]
    fn lines_have_both_ends() {
        let mut c = Canvas::new(4, 2);
        c.line(0, 0, 5, 0);
        assert_eq!(dots(&c), (0..=5).map(|x| (x, 0)).collect::<Vec<_>>());
        // The top row of dots in the first three cells.
        assert_eq!(c.rows_text()[0], "⠉⠉⠉ ");

        let mut c = Canvas::new(4, 2);
        c.line(7, 7, 0, 0);
        assert_eq!(dots(&c), (0..8).map(|i| (i, i)).collect::<Vec<_>>());

        // A steep line has one dot in each row.
        let mut c = Canvas::new(4, 2);
        c.line(0, 0, 2, 7);
        let d = dots(&c);
        assert_eq!(d.len(), 8);
        assert_eq!((d[0], d[7]), ((0, 0), (2, 7)));
    }

    #[test]
    fn a_rect_with_and_without_fill() {
        let mut c = Canvas::new(4, 2);
        c.rect(1, 1, 4, 3, false);
        // Two rows of 4 and one dot on each side between them.
        assert_eq!(dots(&c).len(), 10, "the border only");
        assert!(c.get(1, 1) && c.get(4, 3) && !c.get(2, 2));
        let mut c = Canvas::new(4, 2);
        c.rect(1, 1, 4, 3, true);
        assert_eq!(dots(&c).len(), 12);
        let mut c = Canvas::new(4, 2);
        c.rect(1, 1, 0, 3, true);
        assert!(dots(&c).is_empty(), "no width, no rect");
    }

    #[test]
    fn a_circle_with_and_without_fill() {
        let mut c = Canvas::new(8, 4);
        c.circle(7, 7, 0, false);
        assert_eq!(dots(&c), [(7, 7)]);
        let mut c = Canvas::new(8, 4);
        c.circle(7, 7, 4, false);
        for p in [(11, 7), (3, 7), (7, 3), (7, 11)] {
            assert!(c.get(p.0, p.1), "{p:?}");
        }
        assert!(!c.get(7, 7), "the center of an outline is empty");
        // The same on all sides.
        for (x, y) in dots(&c) {
            assert!(c.get(14 - x, y) && c.get(x, 14 - y), "{x},{y}");
        }
        let mut c = Canvas::new(8, 4);
        c.circle(7, 7, 4, true);
        assert!(c.get(7, 7) && c.get(9, 9));
        assert!(!c.get(11, 11));
    }

    #[test]
    fn text_covers_the_dots() {
        let mut c = Canvas::new(4, 1);
        c.line(0, 0, 7, 0);
        c.text(1, 0, "hi!!");
        assert_eq!(c.rows_text(), ["⠉hi!"]);
        c.text(-1, 0, "ab");
        assert_eq!(c.rows_text(), ["bhi!"], "text left of the canvas is cut");
    }

    #[test]
    fn text_is_safe_for_a_terminal() {
        // Text comes from the API: no escape sequences, and one cell for each char.
        let mut c = Canvas::new(6, 1);
        c.text(0, 0, "a\x1b[2Jb\tc");
        assert_eq!(c.rows_text(), ["a [2Jb"]);
        let mut c = Canvas::new(4, 1);
        c.text(0, 0, "日x😀é");
        assert_eq!(c.rows_text(), ["?x?é"]);
    }

    #[test]
    fn the_pen_colors_the_cells() {
        let red = Rgb::new(255, 0, 0);
        let mut c = Canvas::new(3, 1);
        c.set(0, 0);
        c.set_pen(Some(red));
        c.set(2, 0);
        c.text(2, 0, "x");
        assert_eq!(c.cell(0, 0).color, None);
        assert_eq!(c.cell(1, 0).color, Some(red));
        assert_eq!(
            c.cell(2, 0),
            Cell {
                ch: 'x',
                color: Some(red)
            }
        );
    }

    #[test]
    fn clear_takes_all_away() {
        let mut c = Canvas::new(2, 1);
        c.set_pen(Some(Rgb::new(1, 2, 3)));
        c.rect(0, 0, 4, 4, true);
        c.text(0, 0, "a");
        c.clear();
        assert_eq!(c.rows_text(), ["  "]);
        assert_eq!(c.cell(1, 0).color, None);
    }

    #[test]
    fn resize_keeps_what_fits() {
        let mut c = Canvas::new(2, 2);
        c.set(0, 0);
        c.set(3, 7);
        c.text(1, 0, "z");
        c.resize(3, 1);
        assert_eq!((c.width(), c.height()), (6, 4));
        assert!(c.get(0, 0) && !c.get(3, 7));
        assert_eq!(c.rows_text(), ["⠁z "]);
    }

    #[test]
    fn huge_shapes_are_quick_and_cut_at_the_edge() {
        // Commands come from the API: a huge shape must not freeze the window.
        let start = std::time::Instant::now();
        let mut c = Canvas::new(4, 2);
        c.line(-1_000_000_000, 3, 2_000_000_000, 3);
        assert_eq!(dots(&c), (0..8).map(|x| (x, 3)).collect::<Vec<_>>());
        let mut c = Canvas::new(4, 2);
        c.line(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
        assert_eq!(dots(&c), (0..8).map(|i| (i, i)).collect::<Vec<_>>());
        let mut c = Canvas::new(4, 2);
        c.rect(-5, -5, i32::MAX, i32::MAX, true);
        assert_eq!(dots(&c).len(), 8 * 8);
        let mut c = Canvas::new(4, 2);
        c.rect(-5, -5, 1_000_000_000, 1_000_000_000, false);
        assert!(dots(&c).is_empty(), "the border is outside");
        let mut c = Canvas::new(4, 2);
        c.circle(3, 3, i32::MAX, true);
        assert_eq!(dots(&c).len(), 8 * 8, "the canvas is inside the circle");
        let mut c = Canvas::new(4, 2);
        c.circle(3, 3, 2_000_000_000, false);
        assert!(dots(&c).is_empty(), "the outline is far outside");
        let mut c = Canvas::new(4, 2);
        c.circle(i32::MIN, i32::MIN, i32::MAX, true);
        assert!(
            dots(&c).is_empty(),
            "a far center: no overflow, and too far to reach"
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn colors_from_text() {
        assert_eq!(Rgb::parse("#ff8000"), Some(Rgb::new(255, 128, 0)));
        assert_eq!(Rgb::parse("00ff00"), Some(Rgb::new(0, 255, 0)));
        for bad in ["", "#fff", "#gg0000", "red", "#ff00001"] {
            assert_eq!(Rgb::parse(bad), None, "{bad}");
        }
    }
}
