//! Chars that we draw ourselves, not with the font: box lines, blocks, and Braille.
//! They fill the whole cell, so neighbor cells meet with no gaps.

use crate::font::{CellMetrics, GlyphImage, ImageKind};

/// How to draw Braille dots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BrailleStyle {
    /// Each dot fills its 1/8 of the cell. Braille graphs look like real pixels.
    #[default]
    Pixels,
    /// Round dots with space around them, like in a font.
    Dots,
}

/// A Braille char (U+2800..U+28FF).
pub fn is_braille(c: char) -> bool {
    matches!(c, '\u{2800}'..='\u{28FF}')
}

/// True when fterm draws this char itself.
pub fn is_builtin(c: char) -> bool {
    matches!(c, '\u{2500}'..='\u{259F}' | '\u{2800}'..='\u{28FF}')
}

/// Draws a builtin char as an alpha mask of the full cell size.
/// `left` is 0 and `top` is the baseline, so the image starts at the top-left corner of the cell.
pub fn builtin_glyph(c: char, cell: CellMetrics, braille: BrailleStyle) -> Option<GlyphImage> {
    if !is_builtin(c) {
        return None;
    }
    let mut canvas = Canvas::new(cell.width as u32, cell.height as u32);
    let code = c as u32;
    match code {
        0x2800..=0x28FF => braille_dots(&mut canvas, (code - 0x2800) as u8, braille),
        0x2580..=0x259F => block(&mut canvas, code),
        _ => box_drawing(&mut canvas, code),
    }
    Some(canvas.into_image(cell.baseline))
}

/// A cell-sized alpha canvas. Each pixel keeps its coverage (0.0..=1.0).
/// Shapes are joined with `max`, so lines that cross do not get darker.
struct Canvas {
    width: u32,
    height: u32,
    coverage: Vec<f32>,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            coverage: vec![0.0; (width * height) as usize],
        }
    }

    fn w(&self) -> f32 {
        self.width as f32
    }

    fn h(&self) -> f32 {
        self.height as f32
    }

    fn put(&mut self, x: u32, y: u32, value: f32) {
        let slot = &mut self.coverage[(y * self.width + x) as usize];
        *slot = slot.max(value);
    }

    /// Fills a rectangle. Pixels at the border get the part that the rect covers.
    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, alpha: f32) {
        let (x0, y0) = (x0.max(0.0), y0.max(0.0));
        let (x1, y1) = (x1.min(self.w()), y1.min(self.h()));
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        for py in y0.floor() as u32..y1.ceil() as u32 {
            let cover_y = (y1.min(py as f32 + 1.0) - y0.max(py as f32)).max(0.0);
            for px in x0.floor() as u32..x1.ceil() as u32 {
                let cover_x = (x1.min(px as f32 + 1.0) - x0.max(px as f32)).max(0.0);
                self.put(px, py, cover_x * cover_y * alpha);
            }
        }
    }

    /// Fills the pixels inside `inside(x, y)`, with 4x4 samples per pixel for smooth edges.
    fn shape(&mut self, inside: impl Fn(f32, f32) -> bool) {
        const N: u32 = 4;
        for py in 0..self.height {
            for px in 0..self.width {
                let mut hits = 0;
                for sy in 0..N {
                    for sx in 0..N {
                        let x = px as f32 + (sx as f32 + 0.5) / N as f32;
                        let y = py as f32 + (sy as f32 + 0.5) / N as f32;
                        if inside(x, y) {
                            hits += 1;
                        }
                    }
                }
                if hits > 0 {
                    self.put(px, py, hits as f32 / (N * N) as f32);
                }
            }
        }
    }

    fn into_image(self, baseline: f32) -> GlyphImage {
        GlyphImage {
            width: self.width,
            height: self.height,
            left: 0,
            top: baseline as i32,
            kind: ImageKind::Mask,
            data: self
                .coverage
                .iter()
                .map(|&c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect(),
        }
    }
}

// ---- Braille -------------------------------------------------------------------------------

/// Dot bit for (column, row), the same layout as tank_rs `braille.rs`.
const BRAILLE_BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

fn braille_dots(canvas: &mut Canvas, bits: u8, style: BrailleStyle) {
    let (w, h) = (canvas.w(), canvas.h());
    // Edges of the 2x4 sub-cells in whole pixels, so neighbor dots and cells touch.
    let xs = [0.0, (w / 2.0).round(), w];
    let ys = [
        0.0,
        (h / 4.0).round(),
        (h / 2.0).round(),
        (h * 3.0 / 4.0).round(),
        h,
    ];
    for (col, rows) in BRAILLE_BITS.iter().enumerate() {
        for (row, &bit) in rows.iter().enumerate() {
            if bits & bit == 0 {
                continue;
            }
            let (x0, x1, y0, y1) = (xs[col], xs[col + 1], ys[row], ys[row + 1]);
            match style {
                BrailleStyle::Pixels => canvas.rect(x0, y0, x1, y1, 1.0),
                BrailleStyle::Dots => {
                    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                    let r = 0.3 * (x1 - x0).min(y1 - y0);
                    canvas.shape(|x, y| (x - cx).powi(2) + (y - cy).powi(2) <= r * r);
                }
            }
        }
    }
}

// ---- Blocks U+2580..U+259F -----------------------------------------------------------------

fn block(canvas: &mut Canvas, code: u32) {
    let (w, h) = (canvas.w(), canvas.h());
    // Part of the height or width in whole pixels: `eighths` of 8.
    let ry = |eighths: f32| (h * eighths / 8.0).round();
    let rx = |eighths: f32| (w * eighths / 8.0).round();
    let (hw, hh) = ((w / 2.0).round(), (h / 2.0).round());
    let mut quads = |ul: bool, ur: bool, ll: bool, lr: bool| {
        for (on, x0, y0, x1, y1) in [
            (ul, 0.0, 0.0, hw, hh),
            (ur, hw, 0.0, w, hh),
            (ll, 0.0, hh, hw, h),
            (lr, hw, hh, w, h),
        ] {
            if on {
                canvas.rect(x0, y0, x1, y1, 1.0);
            }
        }
    };
    match code {
        0x2580 => quads(true, true, false, false),
        // Lower 1/8 .. full.
        0x2581..=0x2588 => {
            let eighths = (code - 0x2580) as f32;
            canvas.rect(0.0, h - ry(eighths), w, h, 1.0);
        }
        // Left 7/8 .. 1/8.
        0x2589..=0x258F => {
            let eighths = (0x2590 - code) as f32;
            canvas.rect(0.0, 0.0, rx(eighths), h, 1.0);
        }
        0x2590 => quads(false, true, false, true),
        0x2591..=0x2593 => {
            let alpha = (code - 0x2590) as f32 * 0.25;
            canvas.rect(0.0, 0.0, w, h, alpha);
        }
        0x2594 => canvas.rect(0.0, 0.0, w, ry(1.0), 1.0),
        0x2595 => canvas.rect(w - rx(1.0), 0.0, w, h, 1.0),
        0x2596 => quads(false, false, true, false),
        0x2597 => quads(false, false, false, true),
        0x2598 => quads(true, false, false, false),
        0x2599 => quads(true, false, true, true),
        0x259A => quads(true, false, false, true),
        0x259B => quads(true, true, true, false),
        0x259C => quads(true, true, false, true),
        0x259D => quads(false, true, false, false),
        0x259E => quads(false, true, true, false),
        0x259F => quads(false, true, true, true),
        _ => {}
    }
}

// ---- Box drawing U+2500..U+257F ------------------------------------------------------------

/// Line weight of one arm.
const NONE: u8 = 0;
const LIGHT: u8 = 1;
const HEAVY: u8 = 2;
const DOUBLE: u8 = 3;

/// Arms of the box chars: up, right, down, left. 0 none, 1 light, 2 heavy, 3 double.
/// `----` marks chars with their own code (dashes, arcs, diagonals).
const ARMS: [&str; 128] = [
    "0101", "0202", "1010", "2020", "----", "----", "----", "----", // 2500
    "----", "----", "----", "----", "0110", "0210", "0120", "0220", // 2508
    "0011", "0012", "0021", "0022", "1100", "1200", "2100", "2200", // 2510
    "1001", "1002", "2001", "2002", "1110", "1210", "2110", "1120", // 2518
    "2120", "2210", "1220", "2220", "1011", "1012", "2011", "1021", // 2520
    "2021", "2012", "1022", "2022", "0111", "0112", "0211", "0212", // 2528
    "0121", "0122", "0221", "0222", "1101", "1102", "1201", "1202", // 2530
    "2101", "2102", "2201", "2202", "1111", "1112", "1211", "1212", // 2538
    "2111", "1121", "2121", "2112", "2211", "1122", "1221", "2212", // 2540
    "1222", "2122", "2221", "2222", "----", "----", "----", "----", // 2548
    "0303", "3030", "0310", "0130", "0330", "0013", "0031", "0033", // 2550
    "1300", "3100", "3300", "1003", "3001", "3003", "1310", "3130", // 2558
    "3330", "1013", "3031", "3033", "0313", "0131", "0333", "1303", // 2560
    "3101", "3303", "1313", "3131", "3333", "----", "----", "----", // 2568
    "----", "----", "----", "----", "0001", "1000", "0100", "0010", // 2570
    "0002", "2000", "0200", "0020", "0201", "1020", "0102", "2010", // 2578
];

/// Positions of lines in the cell, in pixels.
struct Lines {
    /// Light line width.
    t: f32,
    /// Left edge of a light vertical line and top edge of a light horizontal line.
    vx: f32,
    hy: f32,
}

impl Lines {
    fn new(canvas: &Canvas) -> Self {
        let t = (canvas.w() / 8.0).round().max(1.0);
        Self {
            t,
            vx: ((canvas.w() - t) / 2.0).floor(),
            hy: ((canvas.h() - t) / 2.0).floor(),
        }
    }

    /// Left and right edges of a vertical line with this weight.
    fn v_span(&self, weight: u8) -> (f32, f32) {
        let width = if weight == HEAVY {
            2.0 * self.t
        } else {
            self.t
        };
        let x0 = self.vx - ((width - self.t) / 2.0).floor();
        (x0, x0 + width)
    }

    /// Top and bottom edges of a horizontal line with this weight.
    fn h_span(&self, weight: u8) -> (f32, f32) {
        let width = if weight == HEAVY {
            2.0 * self.t
        } else {
            self.t
        };
        let y0 = self.hy - ((width - self.t) / 2.0).floor();
        (y0, y0 + width)
    }

    /// x of the left (c) and right (d) lines of a double vertical line.
    fn double_x(&self) -> (f32, f32) {
        (self.vx - self.t, self.vx + self.t)
    }

    /// y of the top (a) and bottom (b) lines of a double horizontal line.
    fn double_y(&self) -> (f32, f32) {
        (self.hy - self.t, self.hy + self.t)
    }
}

fn box_drawing(canvas: &mut Canvas, code: u32) {
    let lines = Lines::new(canvas);
    match code {
        0x2504..=0x250B | 0x254C..=0x254F => dashes(canvas, &lines, code),
        0x256D..=0x2570 => arc(canvas, &lines, code),
        0x2571..=0x2573 => diagonals(canvas, &lines, code),
        _ => {
            let arms = ARMS[(code - 0x2500) as usize].as_bytes();
            let [up, right, down, left] = [0, 1, 2, 3].map(|i| arms[i] - b'0');
            straight_arms(canvas, &lines, up, right, down, left);
        }
    }
}

fn straight_arms(canvas: &mut Canvas, l: &Lines, up: u8, right: u8, down: u8, left: u8) {
    let (w, h) = (canvas.w(), canvas.h());
    let t = l.t;
    let (c, d) = l.double_x();
    let (a, b) = l.double_y();
    let vertical_double = up == DOUBLE || down == DOUBLE;
    let horizontal_double = left == DOUBLE || right == DOUBLE;
    // The vertical line that the horizontal arms meet (the thickest one).
    let (vx0, vx1) = l.v_span(up.max(down).min(HEAVY));
    let (hy0, hy1) = l.h_span(left.max(right).min(HEAVY));

    // Right arm.
    match right {
        NONE => {}
        DOUBLE => {
            // Where each of the two lines starts depends on the vertical arms.
            let (top, bottom) = match (up == DOUBLE, down == DOUBLE, up | down) {
                (true, true, _) => (d, d),
                (false, true, _) => (c, d),
                (true, false, _) => (d, c),
                (false, false, NONE) => (l.vx, l.vx),
                _ => (vx0, vx0),
            };
            canvas.rect(top, a, w, a + t, 1.0);
            canvas.rect(bottom, b, w, b + t, 1.0);
        }
        weight => {
            let start = if vertical_double { c } else { vx0 };
            let (y0, y1) = l.h_span(weight);
            canvas.rect(start, y0, w, y1, 1.0);
        }
    }
    // Left arm.
    match left {
        NONE => {}
        DOUBLE => {
            let (top, bottom) = match (up == DOUBLE, down == DOUBLE, up | down) {
                (true, true, _) => (c, c),
                (false, true, _) => (d, c),
                (true, false, _) => (c, d),
                (false, false, NONE) => (l.vx + t, l.vx + t),
                _ => (vx1, vx1),
            };
            canvas.rect(0.0, a, top + t, a + t, 1.0);
            canvas.rect(0.0, b, bottom + t, b + t, 1.0);
        }
        weight => {
            let end = if vertical_double { d + t } else { vx1 };
            let (y0, y1) = l.h_span(weight);
            canvas.rect(0.0, y0, end, y1, 1.0);
        }
    }
    // Down arm.
    match down {
        NONE => {}
        DOUBLE => {
            let (left_line, right_line) = match (left == DOUBLE, right == DOUBLE, left | right) {
                (true, true, _) => (b, b),
                (false, true, _) => (a, b),
                (true, false, _) => (b, a),
                (false, false, NONE) => (l.hy, l.hy),
                _ => (hy0, hy0),
            };
            canvas.rect(c, left_line, c + t, h, 1.0);
            canvas.rect(d, right_line, d + t, h, 1.0);
        }
        weight => {
            let start = if horizontal_double { a } else { hy0 };
            let (x0, x1) = l.v_span(weight);
            canvas.rect(x0, start, x1, h, 1.0);
        }
    }
    // Up arm.
    match up {
        NONE => {}
        DOUBLE => {
            let (left_line, right_line) = match (left == DOUBLE, right == DOUBLE, left | right) {
                (true, true, _) => (a, a),
                (false, true, _) => (b, a),
                (true, false, _) => (a, b),
                (false, false, NONE) => (l.hy + t, l.hy + t),
                _ => (hy1, hy1),
            };
            canvas.rect(c, 0.0, c + t, left_line + t, 1.0);
            canvas.rect(d, 0.0, d + t, right_line + t, 1.0);
        }
        weight => {
            let end = if horizontal_double { b + t } else { hy1 };
            let (x0, x1) = l.v_span(weight);
            canvas.rect(x0, 0.0, x1, end, 1.0);
        }
    }
}

fn dashes(canvas: &mut Canvas, l: &Lines, code: u32) {
    // (count, heavy, vertical)
    let (count, heavy, vertical) = match code {
        0x2504 => (3, false, false),
        0x2505 => (3, true, false),
        0x2506 => (3, false, true),
        0x2507 => (3, true, true),
        0x2508 => (4, false, false),
        0x2509 => (4, true, false),
        0x250A => (4, false, true),
        0x250B => (4, true, true),
        0x254C => (2, false, false),
        0x254D => (2, true, false),
        0x254E => (2, false, true),
        _ => (2, true, true),
    };
    let weight = if heavy { HEAVY } else { LIGHT };
    let length = if vertical { canvas.h() } else { canvas.w() };
    let step = length / count as f32;
    for i in 0..count {
        let start = i as f32 * step + step * 0.2;
        let end = i as f32 * step + step * 0.8;
        if vertical {
            let (x0, x1) = l.v_span(weight);
            canvas.rect(x0, start, x1, end, 1.0);
        } else {
            let (y0, y1) = l.h_span(weight);
            canvas.rect(start, y0, end, y1, 1.0);
        }
    }
}

/// Rounded corners ╭ ╮ ╯ ╰: a quarter circle plus short straight parts to the cell edges.
fn arc(canvas: &mut Canvas, l: &Lines, code: u32) {
    let (w, h) = (canvas.w(), canvas.h());
    // Which way the two arms go: +1 = right/down, -1 = left/up.
    let (sx, sy) = match code {
        0x256D => (1.0, 1.0),
        0x256E => (-1.0, 1.0),
        0x256F => (-1.0, -1.0),
        _ => (1.0, -1.0),
    };
    let t = l.t;
    // Centers of the light lines.
    let (lx, ly) = (l.vx + t / 2.0, l.hy + t / 2.0);
    let r = lx.min(w - lx).min(ly).min(h - ly);
    let (cx, cy) = (lx + sx * r, ly + sy * r);
    canvas.shape(|x, y| {
        let in_quarter = (x - cx) * sx <= 0.0 && (y - cy) * sy <= 0.0;
        let dist = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        in_quarter && (dist - r).abs() <= t / 2.0
    });
    // Straight part from the end of the arc to the cell edge.
    let (x0, x1) = (l.vx, l.vx + t);
    if sy > 0.0 {
        canvas.rect(x0, cy, x1, h, 1.0);
    } else {
        canvas.rect(x0, 0.0, x1, cy, 1.0);
    }
    let (y0, y1) = (l.hy, l.hy + t);
    if sx > 0.0 {
        canvas.rect(cx, y0, w, y1, 1.0);
    } else {
        canvas.rect(0.0, y0, cx, y1, 1.0);
    }
}

/// ╱ ╲ ╳: lines from corner to corner.
fn diagonals(canvas: &mut Canvas, l: &Lines, code: u32) {
    let (w, h) = (canvas.w(), canvas.h());
    let half = l.t * 1.25 / 2.0;
    let len = (w * w + h * h).sqrt();
    // Distance from (x, y) to the line from (0, h) to (w, 0), and to the line from (0, 0) to (w, h).
    let rising = move |x: f32, y: f32| (h * x + w * y - w * h).abs() / len <= half;
    let falling = move |x: f32, y: f32| (h * x - w * y).abs() / len <= half;
    match code {
        0x2571 => canvas.shape(rising),
        0x2572 => canvas.shape(falling),
        _ => canvas.shape(|x, y| rising(x, y) || falling(x, y)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: CellMetrics = CellMetrics {
        width: 8.0,
        height: 16.0,
        baseline: 12.0,
    };

    fn draw(c: char) -> Mask {
        draw_with(c, BrailleStyle::Pixels)
    }

    fn draw_with(c: char, style: BrailleStyle) -> Mask {
        let image = builtin_glyph(c, CELL, style).unwrap_or_else(|| panic!("no image for {c:?}"));
        assert_eq!((image.width, image.height), (8, 16), "{c:?}");
        assert_eq!((image.left, image.top), (0, 12), "{c:?}");
        Mask(image)
    }

    struct Mask(GlyphImage);

    impl Mask {
        fn at(&self, x: u32, y: u32) -> u8 {
            self.0.data[(y * self.0.width + x) as usize]
        }
        fn on(&self, x: u32, y: u32) -> bool {
            self.at(x, y) > 127
        }
        fn row_full(&self, y: u32) -> bool {
            (0..self.0.width).all(|x| self.on(x, y))
        }
        fn col_full(&self, x: u32) -> bool {
            (0..self.0.height).all(|y| self.on(x, y))
        }
        fn any_row_full(&self) -> bool {
            (0..self.0.height).any(|y| self.row_full(y))
        }
        fn any_col_full(&self) -> bool {
            (0..self.0.width).any(|x| self.col_full(x))
        }
        fn count_on(&self) -> usize {
            self.0.data.iter().filter(|&&a| a > 127).count()
        }
        /// Pixels that are on in the rect x0..x1, y0..y1.
        fn count_in(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
            (y0..y1)
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter(|&(x, y)| self.on(x, y))
                .count()
        }
    }

    #[test]
    fn builtin_ranges() {
        assert!(is_builtin('─') && is_builtin('╳') && is_builtin('█') && is_builtin('⣿'));
        assert!(!is_builtin('a') && !is_builtin('✓') && !is_builtin('▲'));
    }

    #[test]
    fn every_builtin_char_has_an_image_of_the_cell_size() {
        for code in (0x2500..=0x259F).chain(0x2800..=0x28FF) {
            let c = char::from_u32(code).unwrap();
            for style in [BrailleStyle::Pixels, BrailleStyle::Dots] {
                draw_with(c, style);
            }
        }
    }

    #[test]
    fn horizontal_line_goes_edge_to_edge_in_the_middle() {
        let m = draw('─');
        assert!(m.any_row_full());
        assert!(!m.on(4, 0) && !m.on(4, 15), "only the middle is drawn");
        let rows: Vec<u32> = (0..16).filter(|&y| m.row_full(y)).collect();
        assert!(rows.iter().all(|&y| (6..=9).contains(&y)), "{rows:?}");
    }

    #[test]
    fn vertical_line_goes_top_to_bottom() {
        let m = draw('│');
        assert!(m.any_col_full());
        assert!(!m.on(0, 8) && !m.on(7, 8));
    }

    #[test]
    fn heavy_line_is_thicker() {
        assert!(draw('━').count_on() > draw('─').count_on());
    }

    #[test]
    fn cross_is_both_lines() {
        let cross = draw('┼');
        let h = draw('─');
        let v = draw('│');
        for y in 0..16 {
            for x in 0..8 {
                assert_eq!(cross.on(x, y), h.on(x, y) || v.on(x, y), "({x}, {y})");
            }
        }
    }

    #[test]
    fn corner_reaches_only_its_edges() {
        // ┌ goes right and down.
        let m = draw('┌');
        assert!(m.on(7, 7) || m.on(7, 8), "reaches the right edge");
        assert!(m.on(3, 15) || m.on(4, 15), "reaches the bottom edge");
        assert!(!m.on(0, 7) && !m.on(0, 8), "does not reach the left edge");
        assert!(!m.on(3, 0) && !m.on(4, 0), "does not reach the top edge");
    }

    #[test]
    fn double_line_has_two_lines_and_a_gap() {
        let m = draw('═');
        let full: Vec<u32> = (0..16).filter(|&y| m.row_full(y)).collect();
        assert!(full.len() >= 2, "{full:?}");
        let first = full[0];
        let last = *full.last().unwrap();
        assert!((first..=last).any(|y| !m.row_full(y)), "no gap: {full:?}");
    }

    #[test]
    fn double_corner_has_no_lines_inside() {
        // ╔: nothing above the top line or left of the left line.
        let m = draw('╔');
        assert_eq!(m.count_in(0, 0, 8, 5), 0);
        assert_eq!(m.count_in(0, 0, 2, 16), 0);
        // The inner corner is open: the pixel near the right-bottom of the corner is empty.
        assert!(m.on(7, 5) || m.on(7, 6));
    }

    #[test]
    fn rounded_corner_touches_right_and_bottom_edges() {
        let m = draw('╭');
        assert!((0..16).any(|y| m.on(7, y)), "right edge");
        assert!((0..8).any(|x| m.on(x, 15)), "bottom edge");
        assert!(!(0..16).any(|y| m.on(0, y)), "left edge is empty");
        assert!(!(0..8).any(|x| m.on(x, 0)), "top edge is empty");
    }

    #[test]
    fn rounded_corners_are_mirrors_and_meet_lines() {
        // The vertical part of ╰ is in the same column as │.
        let v = draw('│');
        let arc = draw('╰');
        assert!((0..8).any(|x| v.on(x, 0) && arc.on(x, 0)));
        // The horizontal part of ╮ is in the same row as ─.
        let h = draw('─');
        let arc = draw('╮');
        assert!((0..16).any(|y| h.on(0, y) && arc.on(0, y)));
    }

    #[test]
    fn dashed_line_has_gaps() {
        let m = draw('┄');
        assert!(!m.any_row_full());
        assert!(m.count_on() > 0);
    }

    #[test]
    fn diagonal_goes_corner_to_corner() {
        let m = draw('╱');
        assert!(m.on(7, 0) || m.on(6, 0), "top right");
        assert!(m.on(0, 15) || m.on(1, 15), "bottom left");
        assert!(!m.on(0, 0) && !m.on(7, 15));
    }

    #[test]
    fn full_block_is_all_on() {
        let m = draw('█');
        assert!(m.0.data.iter().all(|&a| a == 255));
    }

    #[test]
    fn half_blocks() {
        let top = draw('▀');
        assert_eq!(top.count_in(0, 0, 8, 8), 64);
        assert_eq!(top.count_in(0, 8, 8, 16), 0);
        let right = draw('▐');
        assert_eq!(right.count_in(4, 0, 8, 16), 64);
        assert_eq!(right.count_in(0, 0, 4, 16), 0);
    }

    #[test]
    fn eighth_blocks_grow() {
        let lower: Vec<usize> = "▁▂▃▄▅▆▇█".chars().map(|c| draw(c).count_on()).collect();
        assert!(lower.windows(2).all(|w| w[0] < w[1]), "{lower:?}");
        let left: Vec<usize> = "▏▎▍▌▋▊▉█".chars().map(|c| draw(c).count_on()).collect();
        assert!(left.windows(2).all(|w| w[0] <= w[1]), "{left:?}");
        assert_eq!(draw('▁').count_in(0, 14, 8, 16), 16);
    }

    #[test]
    fn quadrants() {
        let m = draw('▚'); // upper left + lower right
        assert_eq!(m.count_in(0, 0, 4, 8), 32);
        assert_eq!(m.count_in(4, 8, 8, 16), 32);
        assert_eq!(m.count_in(4, 0, 8, 8), 0);
        assert_eq!(m.count_in(0, 8, 4, 16), 0);
        assert_eq!(draw('▟').count_on(), 3 * 32);
    }

    #[test]
    fn shades_have_partial_alpha() {
        let mean = |c| {
            let m = draw(c);
            m.0.data.iter().map(|&a| f32::from(a)).sum::<f32>() / m.0.data.len() as f32 / 255.0
        };
        assert!((mean('░') - 0.25).abs() < 0.05);
        assert!((mean('▒') - 0.50).abs() < 0.05);
        assert!((mean('▓') - 0.75).abs() < 0.05);
    }

    #[test]
    fn braille_full_pixels_fill_the_cell() {
        let m = draw('⣿');
        assert!(m.0.data.iter().all(|&a| a == 255));
    }

    #[test]
    fn braille_bits_follow_tank_rs_layout() {
        // 0x01 = left column, row 0: top-left 1/8 only (4x4 pixels).
        let m = draw('⠁');
        assert_eq!(m.count_in(0, 0, 4, 4), 16);
        assert_eq!(m.count_on(), 16);
        // 0x40 = left column, row 3.
        assert_eq!(draw('⡀').count_in(0, 12, 4, 16), 16);
        // 0x08 = right column, row 0.
        assert_eq!(draw('⠈').count_in(4, 0, 8, 4), 16);
        // 0x80 = right column, row 3.
        let m = draw('⢀');
        assert_eq!(m.count_in(4, 12, 8, 16), 16);
        assert_eq!(m.count_on(), 16);
    }

    #[test]
    fn braille_pixels_have_no_gap_at_the_cell_edges() {
        // ⣿ in every cell: the edge columns and rows are full, so neighbor cells touch.
        let m = draw('⣿');
        assert!(m.col_full(0) && m.col_full(7) && m.row_full(0) && m.row_full(15));
    }

    #[test]
    fn braille_dots_are_round_with_space_around() {
        let m = draw_with('⠁', BrailleStyle::Dots);
        assert!(m.count_on() > 0);
        assert!(!m.on(0, 0), "the corner of the sub-cell is empty");
        assert_eq!(m.count_in(4, 0, 8, 16), 0, "only the left column");
        assert!(m.count_on() < 16, "smaller than a full sub-cell");
    }

    #[test]
    fn empty_braille_is_empty() {
        assert_eq!(draw('\u{2800}').count_on(), 0);
    }

    #[test]
    fn other_cell_sizes_work() {
        let cell = CellMetrics {
            width: 11.0,
            height: 23.0,
            baseline: 17.0,
        };
        let image = builtin_glyph('⣿', cell, BrailleStyle::Pixels).unwrap();
        assert_eq!((image.width, image.height), (11, 23));
        assert!(image.data.iter().all(|&a| a == 255));
        let image = builtin_glyph('─', cell, BrailleStyle::Pixels).unwrap();
        assert!(image.data.contains(&255));
    }
}
