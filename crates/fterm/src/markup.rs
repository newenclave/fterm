//! Marking up a picture (a picture pane): a pen, boxes, arrows, text, hidden parts (big pixels), and a crop. The
//! marks are kept as a list (undo, redo) and drawn on the CPU over the picture. No GPU code.
//!
//! Points are in pixels of the picture as it is shown now (after a crop); inside they are kept in pixels of the
//! first picture, so a crop does not move the marks.

use std::sync::Arc;

use fterm_term::picture::Picture;

/// A point in pixels.
pub type Point = (f32, f32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Pen,
    Rect,
    Arrow,
    Text,
    /// Big pixels over a part (to hide a secret: a blur can sometimes be undone, big pixels cannot).
    Hide,
    /// Keep only a part.
    Crop,
}

/// The colors of the marks: red, yellow, green, blue.
pub const COLORS: [[u8; 3]; 4] = [[235, 64, 52], [245, 200, 40], [60, 200, 90], [60, 140, 240]];

/// A text mark, drawn by the app with a font: one alpha byte per pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextMask {
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
}

/// One mark. Points are in pixels of the first picture.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Pen {
        points: Vec<Point>,
        color: usize,
        width: f32,
    },
    Rect {
        a: Point,
        b: Point,
        color: usize,
        width: f32,
    },
    Arrow {
        a: Point,
        b: Point,
        color: usize,
        width: f32,
    },
    /// The top-left corner of the text is `at`.
    Text {
        at: Point,
        text: String,
        mask: Arc<TextMask>,
        color: usize,
    },
    Hide {
        a: Point,
        b: Point,
    },
    Crop {
        a: Point,
        b: Point,
    },
}

/// A picture and its marks.
pub struct Markup {
    base: Picture,
    ops: Vec<Op>,
    undone: Vec<Op>,
    /// The mark that the mouse draws now (not in `ops` yet).
    draft: Option<Op>,
    /// The first picture with all marks of `ops`, not cropped (so a drag draws only its draft on a copy).
    cache: Arc<Vec<u8>>,
    pub tool: Tool,
    pub color: usize,
    /// Grows with each change (the texture of the picture is made again).
    version: u64,
}

impl Markup {
    pub fn new(base: Picture) -> Self {
        Self {
            cache: base.rgba.clone(),
            base,
            ops: Vec::new(),
            undone: Vec::new(),
            draft: None,
            tool: Tool::default(),
            color: 0,
            version: 0,
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// The width of lines: it grows with the picture, so marks are seen on a big screenshot too.
    pub fn line_width(&self) -> f32 {
        (self.base.width.min(self.base.height) as f32 / 200.0).max(3.0)
    }

    /// The size of text in pixels.
    pub fn text_size(&self) -> f32 {
        (self.base.height as f32 / 30.0).max(16.0)
    }

    /// The part of the first picture that is kept (all crops): x0, y0, x1, y1.
    fn view(&self) -> (u32, u32, u32, u32) {
        let (mut x0, mut y0, mut x1, mut y1) = (
            0.0f32,
            0.0f32,
            self.base.width as f32,
            self.base.height as f32,
        );
        for op in &self.ops {
            if let Op::Crop { a, b } = op {
                x0 = x0.max(a.0.min(b.0));
                y0 = y0.max(a.1.min(b.1));
                x1 = x1.min(a.0.max(b.0));
                y1 = y1.min(a.1.max(b.1));
            }
        }
        let (x0, y0) = (x0.floor().max(0.0) as u32, y0.floor().max(0.0) as u32);
        let x1 = (x1.ceil() as u32).clamp(x0 + 1, self.base.width.max(1));
        let y1 = (y1.ceil() as u32).clamp(y0 + 1, self.base.height.max(1));
        (x0, y0, x1, y1)
    }

    /// The size of the picture as it is shown (after the crops).
    pub fn size(&self) -> (u32, u32) {
        let (x0, y0, x1, y1) = self.view();
        (x1 - x0, y1 - y0)
    }

    /// A point of the shown picture in pixels of the first picture.
    fn to_base(&self, p: Point) -> Point {
        let (x0, y0, _, _) = self.view();
        (p.0 + x0 as f32, p.1 + y0 as f32)
    }

    /// The mouse goes down at `p` (pixels of the shown picture): a new mark of the tool starts.
    pub fn press(&mut self, p: Point) {
        let p = self.to_base(p);
        let (color, width) = (self.color, self.line_width());
        self.draft = match self.tool {
            Tool::Pen => Some(Op::Pen {
                points: vec![p],
                color,
                width,
            }),
            Tool::Rect => Some(Op::Rect {
                a: p,
                b: p,
                color,
                width,
            }),
            Tool::Arrow => Some(Op::Arrow {
                a: p,
                b: p,
                color,
                width,
            }),
            Tool::Hide => Some(Op::Hide { a: p, b: p }),
            Tool::Crop => Some(Op::Crop { a: p, b: p }),
            // Text starts with a click and the keys (the app adds it with `add_text`).
            Tool::Text => None,
        };
        self.version += 1;
    }

    /// The mouse moves to `p` with the button down.
    pub fn drag(&mut self, p: Point) {
        let p = self.to_base(p);
        match &mut self.draft {
            Some(Op::Pen { points, .. }) => points.push(p),
            Some(
                Op::Rect { b, .. } | Op::Arrow { b, .. } | Op::Hide { b, .. } | Op::Crop { b, .. },
            ) => *b = p,
            _ => return,
        }
        self.version += 1;
    }

    /// The mouse goes up: the mark is done. A box, an arrow, a hidden part, or a crop of (almost) no size is dropped.
    pub fn release(&mut self, p: Point) {
        self.drag(p);
        let Some(op) = self.draft.take() else {
            return;
        };
        let tiny = |a: Point, b: Point| (a.0 - b.0).abs() < 3.0 && (a.1 - b.1).abs() < 3.0;
        let keep = match &op {
            Op::Rect { a, b, .. }
            | Op::Arrow { a, b, .. }
            | Op::Hide { a, b }
            | Op::Crop { a, b } => !tiny(*a, *b),
            _ => true,
        };
        if keep {
            self.commit(op);
        }
        self.version += 1;
    }

    /// Drops the mark that the mouse draws now. `true` when there was one.
    pub fn cancel(&mut self) -> bool {
        let had = self.draft.take().is_some();
        if had {
            self.version += 1;
        }
        had
    }

    /// A text mark at `at` (pixels of the shown picture), drawn by the app as `mask`.
    pub fn add_text(&mut self, at: Point, text: String, mask: TextMask) {
        if text.trim().is_empty() {
            return;
        }
        let at = self.to_base(at);
        self.commit(Op::Text {
            at,
            text,
            mask: Arc::new(mask),
            color: self.color,
        });
        self.version += 1;
    }

    fn commit(&mut self, op: Op) {
        let mut rgba = (*self.cache).clone();
        draw(&mut rgba, self.base.width, self.base.height, &op);
        self.cache = Arc::new(rgba);
        self.ops.push(op);
        self.undone.clear();
    }

    /// Takes the last mark back. `false` when there is none.
    pub fn undo(&mut self) -> bool {
        let Some(op) = self.ops.pop() else {
            return false;
        };
        self.undone.push(op);
        self.redraw();
        true
    }

    /// Puts back the mark that `undo` took. `false` when there is none.
    pub fn redo(&mut self) -> bool {
        let Some(op) = self.undone.pop() else {
            return false;
        };
        let undone = std::mem::take(&mut self.undone);
        self.commit(op);
        self.undone = undone;
        self.version += 1;
        true
    }

    /// Draws the cache again from the first picture (after an undo).
    fn redraw(&mut self) {
        let mut rgba = (*self.base.rgba).clone();
        for op in &self.ops {
            draw(&mut rgba, self.base.width, self.base.height, op);
        }
        self.cache = Arc::new(rgba);
        self.version += 1;
    }

    /// There are marks (something to save).
    pub fn changed(&self) -> bool {
        !self.ops.is_empty()
    }

    /// The picture as it is shown now: with the marks and the draft, cut to the crops. The draft of a crop shows
    /// its frame.
    pub fn render(&self) -> Picture {
        let (w, h) = (self.base.width, self.base.height);
        let rgba = match &self.draft {
            None => self.cache.clone(),
            Some(op) => {
                let mut rgba = (*self.cache).clone();
                match op {
                    Op::Crop { a, b } => {
                        // White and dark lines, so the frame is seen on any picture.
                        let width = self.line_width().max(2.0);
                        draw_rect(&mut rgba, w, h, *a, *b, width + 2.0, [20, 20, 20]);
                        draw_rect(&mut rgba, w, h, *a, *b, width, [255, 255, 255]);
                    }
                    op => draw(&mut rgba, w, h, op),
                }
                Arc::new(rgba)
            }
        };
        let (x0, y0, x1, y1) = self.view();
        if (x0, y0, x1, y1) == (0, 0, w, h) {
            return Picture {
                width: w,
                height: h,
                rgba,
            };
        }
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut out = Vec::with_capacity((cw * ch * 4) as usize);
        for y in y0..y1 {
            let start = ((y * w + x0) * 4) as usize;
            out.extend_from_slice(&rgba[start..start + (cw * 4) as usize]);
        }
        Picture {
            width: cw,
            height: ch,
            rgba: Arc::new(out),
        }
    }
}

/// Draws one mark on an RGBA picture of `w`×`h`.
fn draw(rgba: &mut [u8], w: u32, h: u32, op: &Op) {
    match op {
        Op::Pen {
            points,
            color,
            width,
        } => {
            let color = COLORS[*color % COLORS.len()];
            if let [p] = points.as_slice() {
                disc(rgba, w, h, *p, width / 2.0, color);
            }
            for pair in points.windows(2) {
                line(rgba, w, h, pair[0], pair[1], *width, color);
            }
        }
        Op::Rect { a, b, color, width } => {
            draw_rect(rgba, w, h, *a, *b, *width, COLORS[*color % COLORS.len()])
        }
        Op::Arrow { a, b, color, width } => {
            let color = COLORS[*color % COLORS.len()];
            line(rgba, w, h, *a, *b, *width, color);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let length = (dx * dx + dy * dy).sqrt();
            if length > 0.0 {
                // Two lines back from the tip, at 28° to the arrow.
                let head = (width * 5.0).max(12.0).min(length * 0.6);
                let (ux, uy) = (dx / length, dy / length);
                for angle in [0.5f32, -0.5] {
                    let (s, c) = angle.sin_cos();
                    let (rx, ry) = (ux * c - uy * s, ux * s + uy * c);
                    line(
                        rgba,
                        w,
                        h,
                        *b,
                        (b.0 - rx * head, b.1 - ry * head),
                        *width,
                        color,
                    );
                }
            }
        }
        Op::Text {
            at, mask, color, ..
        } => {
            let color = COLORS[*color % COLORS.len()];
            let (x0, y0) = (at.0.round() as i64, at.1.round() as i64);
            for my in 0..mask.height {
                for mx in 0..mask.width {
                    let alpha = mask.alpha[(my * mask.width + mx) as usize];
                    let (x, y) = (x0 + i64::from(mx), y0 + i64::from(my));
                    if alpha > 0 {
                        blend(rgba, w, h, x, y, color, alpha);
                    }
                }
            }
        }
        Op::Hide { a, b } => hide(rgba, w, h, *a, *b),
        // A crop cuts the picture when it is shown (`render`); it draws nothing.
        Op::Crop { .. } => {}
    }
}

/// A pixel of `color` over (`x`, `y`), with `alpha` (0..255).
fn blend(rgba: &mut [u8], w: u32, h: u32, x: i64, y: i64, color: [u8; 3], alpha: u8) {
    if x < 0 || y < 0 || x >= i64::from(w) || y >= i64::from(h) {
        return;
    }
    let i = ((y as u64 * u64::from(w) + x as u64) * 4) as usize;
    let a = u32::from(alpha);
    for c in 0..3 {
        let old = u32::from(rgba[i + c]);
        rgba[i + c] = ((u32::from(color[c]) * a + old * (255 - a)) / 255) as u8;
    }
    rgba[i + 3] = rgba[i + 3].max(alpha);
}

/// A filled circle (soft at its edge).
fn disc(rgba: &mut [u8], w: u32, h: u32, c: Point, r: f32, color: [u8; 3]) {
    let r = r.max(0.5);
    let (x0, x1) = (
        (c.0 - r - 1.0).floor() as i64,
        (c.0 + r + 1.0).ceil() as i64,
    );
    let (y0, y1) = (
        (c.1 - r - 1.0).floor() as i64,
        (c.1 + r + 1.0).ceil() as i64,
    );
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = ((x as f32 + 0.5 - c.0).powi(2) + (y as f32 + 0.5 - c.1).powi(2)).sqrt();
            let cover = (r + 0.5 - d).clamp(0.0, 1.0);
            if cover > 0.0 {
                blend(rgba, w, h, x, y, color, (cover * 255.0) as u8);
            }
        }
    }
}

/// A thick line: circles close to each other along it.
fn line(rgba: &mut [u8], w: u32, h: u32, a: Point, b: Point, width: f32, color: [u8; 3]) {
    let r = width / 2.0;
    let length = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let steps = (length / (r / 2.0).max(0.5)).ceil().max(1.0) as usize;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        disc(
            rgba,
            w,
            h,
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t),
            r,
            color,
        );
    }
}

fn draw_rect(rgba: &mut [u8], w: u32, h: u32, a: Point, b: Point, width: f32, color: [u8; 3]) {
    let (c, d) = ((b.0, a.1), (a.0, b.1));
    for (p, q) in [(a, c), (c, b), (b, d), (d, a)] {
        line(rgba, w, h, p, q, width, color);
    }
}

/// Big pixels over the rect `a`–`b`: each block gets the mean color of its pixels.
fn hide(rgba: &mut [u8], w: u32, h: u32, a: Point, b: Point) {
    let block = (w.min(h) / 40).clamp(8, 32);
    let x0 = (a.0.min(b.0).floor().max(0.0) as u32).min(w);
    let y0 = (a.1.min(b.1).floor().max(0.0) as u32).min(h);
    let x1 = (a.0.max(b.0).ceil().max(0.0) as u32).min(w);
    let y1 = (a.1.max(b.1).ceil().max(0.0) as u32).min(h);
    let mut by = y0;
    while by < y1 {
        let mut bx = x0;
        let ey = (by + block).min(y1);
        while bx < x1 {
            let ex = (bx + block).min(x1);
            let mut sum = [0u64; 4];
            for y in by..ey {
                for x in bx..ex {
                    let i = ((y * w + x) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += u64::from(rgba[i + c]);
                    }
                }
            }
            let n = u64::from((ex - bx) * (ey - by)).max(1);
            for y in by..ey {
                for x in bx..ex {
                    let i = ((y * w + x) * 4) as usize;
                    for c in 0..4 {
                        rgba[i + c] = (sum[c] / n) as u8;
                    }
                }
            }
            bx = ex;
        }
        by = ey;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grey picture, or one with its own pixels.
    fn picture(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 4]) -> Picture {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend(px(x, y));
            }
        }
        Picture {
            width: w,
            height: h,
            rgba: Arc::new(rgba),
        }
    }

    fn grey(w: u32, h: u32) -> Picture {
        picture(w, h, |_, _| [100, 100, 100, 255])
    }

    fn at(p: &Picture, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * p.width + x) * 4) as usize;
        [p.rgba[i], p.rgba[i + 1], p.rgba[i + 2]]
    }

    const GREY: [u8; 3] = [100, 100, 100];

    fn mark(m: &mut Markup, tool: Tool, a: Point, b: Point) {
        m.tool = tool;
        m.press(a);
        m.drag(((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0));
        m.release(b);
    }

    #[test]
    fn the_pen_draws_where_the_mouse_went() {
        let mut m = Markup::new(grey(100, 100));
        mark(&mut m, Tool::Pen, (10.0, 50.0), (90.0, 50.0));
        let p = m.render();
        assert_eq!(at(&p, 50, 50), COLORS[0], "on the line");
        assert_eq!(at(&p, 50, 10), GREY, "far from it");
        assert!(m.changed());
    }

    #[test]
    fn a_box_has_only_its_frame() {
        let mut m = Markup::new(grey(100, 100));
        m.color = 3;
        mark(&mut m, Tool::Rect, (20.0, 20.0), (80.0, 80.0));
        let p = m.render();
        assert_eq!(at(&p, 50, 20), COLORS[3], "the top side");
        assert_eq!(at(&p, 80, 50), COLORS[3], "the right side");
        assert_eq!(at(&p, 50, 50), GREY, "the inside");
    }

    #[test]
    fn an_arrow_has_a_head_at_its_end() {
        let mut m = Markup::new(grey(200, 200));
        mark(&mut m, Tool::Arrow, (20.0, 100.0), (180.0, 100.0));
        let p = m.render();
        assert_eq!(at(&p, 100, 100), COLORS[0]);
        // The head goes back and away from the line near the tip.
        assert_eq!(at(&p, 171, 104), COLORS[0], "a head line");
        assert_eq!(at(&p, 30, 110), GREY, "no head at the start");
    }

    #[test]
    fn hide_makes_big_pixels_only_in_its_part() {
        // A picture of thin stripes: hidden, a block is one color (the mean).
        let base = picture(160, 160, |x, _| {
            if x % 2 == 0 {
                [0, 0, 0, 255]
            } else {
                [200, 200, 200, 255]
            }
        });
        let mut m = Markup::new(base);
        mark(&mut m, Tool::Hide, (0.0, 0.0), (80.0, 80.0));
        let p = m.render();
        assert_eq!(at(&p, 0, 0), [100, 100, 100]);
        assert_eq!(at(&p, 1, 0), [100, 100, 100], "the stripes are gone");
        assert_eq!(at(&p, 120, 120), [0, 0, 0], "outside it stays");
        assert_eq!(at(&p, 121, 120), [200, 200, 200]);
    }

    #[test]
    fn a_crop_keeps_a_part_and_later_marks_go_to_the_right_place() {
        let mut m = Markup::new(grey(200, 100));
        mark(&mut m, Tool::Crop, (50.0, 20.0), (150.0, 80.0));
        assert_eq!(m.size(), (100, 60));
        let p = m.render();
        assert_eq!((p.width, p.height), (100, 60));
        // A box in the cropped picture is drawn there (and in the right place of the first picture).
        mark(&mut m, Tool::Rect, (10.0, 10.0), (90.0, 50.0));
        let p = m.render();
        assert_eq!(at(&p, 50, 10), COLORS[0]);
        assert_eq!(at(&p, 50, 30), GREY);
        // A second crop cuts more.
        mark(&mut m, Tool::Crop, (0.0, 0.0), (50.0, 30.0));
        assert_eq!(m.size(), (50, 30));
    }

    #[test]
    fn the_draft_of_a_crop_shows_its_frame_and_cuts_nothing() {
        let mut m = Markup::new(grey(100, 100));
        m.tool = Tool::Crop;
        m.press((10.0, 10.0));
        m.drag((60.0, 60.0));
        let p = m.render();
        assert_eq!((p.width, p.height), (100, 100));
        assert_ne!(at(&p, 35, 10), GREY, "its frame");
        assert!(m.cancel());
        assert_eq!(m.render().width, 100);
        assert!(!m.changed());
    }

    #[test]
    fn undo_and_redo() {
        let mut m = Markup::new(grey(100, 100));
        mark(&mut m, Tool::Rect, (20.0, 20.0), (80.0, 80.0));
        mark(&mut m, Tool::Crop, (0.0, 0.0), (50.0, 50.0));
        assert!(m.undo());
        assert_eq!(m.size(), (100, 100), "the crop is gone");
        assert_eq!(at(&m.render(), 50, 20), COLORS[0], "the box stays");
        assert!(m.undo());
        assert_eq!(
            *m.render().rgba,
            *grey(100, 100).rgba,
            "the first picture again"
        );
        assert!(!m.undo());
        assert!(m.redo());
        assert_eq!(at(&m.render(), 50, 20), COLORS[0]);
        // A new mark: the crop cannot come back.
        mark(&mut m, Tool::Pen, (5.0, 95.0), (95.0, 95.0));
        assert!(!m.redo());
    }

    #[test]
    fn a_click_is_not_a_box() {
        let mut m = Markup::new(grey(100, 100));
        mark(&mut m, Tool::Rect, (50.0, 50.0), (51.0, 50.0));
        mark(&mut m, Tool::Crop, (50.0, 50.0), (50.0, 51.0));
        assert!(!m.changed());
        // A click of the pen is a dot.
        m.tool = Tool::Pen;
        m.press((30.0, 30.0));
        m.release((30.0, 30.0));
        assert_eq!(at(&m.render(), 30, 30), COLORS[0]);
    }

    #[test]
    fn text_is_drawn_from_its_mask() {
        let mut m = Markup::new(grey(100, 100));
        m.color = 2;
        let mask = TextMask {
            width: 3,
            height: 2,
            alpha: vec![255, 0, 255, 0, 255, 0],
        };
        m.add_text((10.0, 20.0), "x".into(), mask.clone());
        let p = m.render();
        assert_eq!(at(&p, 10, 20), COLORS[2]);
        assert_eq!(at(&p, 11, 20), GREY);
        assert_eq!(at(&p, 11, 21), COLORS[2]);
        // Empty text adds nothing.
        m.add_text((0.0, 0.0), "  ".into(), mask);
        assert!(m.undo() && !m.undo());
    }

    #[test]
    fn each_change_has_a_new_version() {
        let mut m = Markup::new(grey(100, 100));
        let v0 = m.version();
        m.press((1.0, 1.0));
        let v1 = m.version();
        m.drag((20.0, 20.0));
        let v2 = m.version();
        m.release((30.0, 30.0));
        let v3 = m.version();
        assert!(v0 < v1 && v1 < v2 && v2 < v3);
        m.undo();
        assert!(m.version() > v3);
    }

    #[test]
    fn marks_off_the_picture_do_not_break_it() {
        let mut m = Markup::new(grey(50, 50));
        mark(&mut m, Tool::Pen, (-100.0, -100.0), (500.0, 500.0));
        mark(&mut m, Tool::Hide, (-10.0, -10.0), (1000.0, 1000.0));
        mark(&mut m, Tool::Crop, (-10.0, -10.0), (1000.0, 1000.0));
        let p = m.render();
        assert_eq!((p.width, p.height), (50, 50));
    }

    #[test]
    fn marks_grow_with_the_picture() {
        assert_eq!(Markup::new(grey(100, 100)).line_width(), 3.0);
        assert_eq!(Markup::new(grey(2000, 1000)).line_width(), 5.0);
        assert_eq!(Markup::new(grey(100, 100)).text_size(), 16.0);
        assert_eq!(Markup::new(grey(1920, 1080)).text_size(), 36.0);
    }
}
