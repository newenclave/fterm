//! A scene: the canvas and the commands that drew it. When the pane gets a new size (zoom, a split, the
//! window), the commands are drawn again for that size, so the picture is not cut and gets sharper.

use std::collections::VecDeque;

use crate::canvas::Canvas;
use crate::ops::Op;

/// The most commands a scene keeps (a live chart with no `clear` must not grow forever).
pub const MAX_OPS: usize = 100_000;

pub struct Scene {
    canvas: Canvas,
    /// The commands since the last `clear`, each batch with the size (cols, rows) it was drawn at.
    batches: VecDeque<((usize, usize), Vec<Op>)>,
    ops: usize,
    /// The size changed, and the API has not said it yet.
    resized: bool,
}

impl Scene {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            canvas: Canvas::new(cols, rows),
            batches: VecDeque::new(),
            ops: 0,
            resized: false,
        }
    }

    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    pub fn set_aspect(&mut self, aspect: f32) {
        self.canvas.set_aspect(aspect);
    }

    /// Draws the commands (all or nothing) and keeps them for a new size.
    pub fn draw(&mut self, ops: &[Op]) -> Result<(), String> {
        crate::ops::colors(ops)?;
        let size = (self.canvas.cols(), self.canvas.rows());
        let mut batch = Vec::new();
        for op in ops {
            if *op == Op::Clear {
                // The old commands are gone; the color stays for the next ones.
                self.batches.clear();
                self.ops = 0;
                batch.clear();
                if let Some(pen) = self.canvas.pen() {
                    batch.push(Op::Color {
                        color: Some(pen.hex()),
                    });
                }
            } else {
                batch.push(op.clone());
            }
            crate::ops::apply(&mut self.canvas, std::slice::from_ref(op))?;
        }
        if !batch.is_empty() {
            self.ops += batch.len();
            self.batches.push_back((size, batch));
        }
        while self.ops > MAX_OPS {
            match self.batches.pop_front() {
                Some((_, old)) => self.ops -= old.len(),
                None => break,
            }
        }
        Ok(())
    }

    /// A new size: the kept commands are drawn again for it, each batch from the size it was drawn at.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        if (cols, rows) == (self.canvas.cols(), self.canvas.rows()) {
            return;
        }
        let mut canvas = Canvas::new(cols, rows);
        canvas.set_aspect(self.canvas.aspect());
        for (from, batch) in &self.batches {
            let scaled: Vec<Op> = batch
                .iter()
                .map(|op| scale(op, *from, (cols, rows)))
                .collect();
            // The commands were good when they came, so they are good now.
            let _ = crate::ops::apply(&mut canvas, &scaled);
        }
        self.canvas = canvas;
        self.resized = true;
    }

    /// True once after each change of the size (for the `scene_resized` event).
    pub fn take_resized(&mut self) -> bool {
        std::mem::take(&mut self.resized)
    }

    /// How many commands are kept.
    pub fn kept(&self) -> usize {
        self.ops
    }
}

/// A command drawn at the size `from` (cols, rows), for the size `to`.
fn scale(op: &Op, from: (usize, usize), to: (usize, usize)) -> Op {
    let (kx, ky) = (
        to.0 as f64 / from.0.max(1) as f64,
        to.1 as f64 / from.1.max(1) as f64,
    );
    let (w0, h0) = ((from.0 * 2) as f64, (from.1 * 4) as f64);
    let (w1, h1) = ((to.0 * 2) as f64, (to.1 * 4) as f64);
    let to_i32 = |v: f64| v.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    // A point: the first dot stays the first, the last stays the last.
    let px = |x: i32| {
        if w0 > 1.0 {
            to_i32((f64::from(x) * (w1 - 1.0) / (w0 - 1.0)).round())
        } else {
            x
        }
    };
    let py = |y: i32| {
        if h0 > 1.0 {
            to_i32((f64::from(y) * (h1 - 1.0) / (h0 - 1.0)).round())
        } else {
            y
        }
    };
    // A box: the dots from `x` to `x + w` cover the same part of the scene.
    let span = |x: i32, w: i32, k: f64| {
        let start = to_i32((f64::from(x) * k).floor());
        let end = to_i32(((f64::from(x) + f64::from(w)) * k).ceil()) - 1;
        (start, (end - start + 1).max(1))
    };
    match op.clone() {
        Op::Dot { x, y } => Op::Dot { x: px(x), y: py(y) },
        Op::Undot { x, y } => Op::Undot { x: px(x), y: py(y) },
        Op::Line { x0, y0, x1, y1 } => Op::Line {
            x0: px(x0),
            y0: py(y0),
            x1: px(x1),
            y1: py(y1),
        },
        Op::Rect { x, y, w, h, fill } if w > 0 && h > 0 => {
            let ((x, w), (y, h)) = (span(x, w, kx), span(y, h, ky));
            Op::Rect { x, y, w, h, fill }
        }
        Op::Circle { x, y, r, fill } => Op::Circle {
            x: px(x),
            y: py(y),
            r: to_i32((f64::from(r) * ky).round()),
            fill,
        },
        Op::Text { col, row, text } => Op::Text {
            col: to_i32((f64::from(col) * kx).floor()),
            row: to_i32((f64::from(row) * ky).floor()),
            text,
        },
        Op::Plot {
            values,
            min,
            max,
            bars,
            x,
            y,
            w,
            h,
        } => {
            // No box = the whole scene, in any size.
            let (x, w) = match (x, w) {
                (Some(x), Some(w)) if w > 0 => {
                    let (x, w) = span(x, w, kx);
                    (Some(x), Some(w))
                }
                (x, w) => (x.map(px), w),
            };
            let (y, h) = match (y, h) {
                (Some(y), Some(h)) if h > 0 => {
                    let (y, h) = span(y, h, ky);
                    (Some(y), Some(h))
                }
                (y, h) => (y.map(py), h),
            };
            Op::Plot {
                values,
                min,
                max,
                bars,
                x,
                y,
                w,
                h,
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Rgb;
    use crate::ops::parse_ops;
    use serde_json::json;

    fn draw(s: &mut Scene, json: serde_json::Value) {
        s.draw(&parse_ops(&json).unwrap()).unwrap();
    }

    #[test]
    fn zoom_makes_the_picture_bigger_and_back() {
        // 8x4 cells = 16x16 dots: a line from corner to corner.
        let mut s = Scene::new(8, 4);
        draw(
            &mut s,
            json!({ "op": "line", "x0": 0, "y0": 0, "x1": 15, "y1": 15 }),
        );
        s.resize(16, 8);
        let c = s.canvas();
        assert_eq!((c.width(), c.height()), (32, 32));
        assert!(
            c.get(0, 0) && c.get(31, 31) && c.get(16, 16),
            "the line goes to the new corner"
        );
        s.resize(8, 4);
        let c = s.canvas();
        assert!(c.get(0, 0) && c.get(15, 15), "and back: nothing is cut");
    }

    #[test]
    fn shapes_and_text_follow_the_size() {
        let mut s = Scene::new(8, 4);
        draw(
            &mut s,
            json!([
                { "op": "rect", "x": 8, "y": 8, "w": 8, "h": 8, "fill": true },
                { "op": "text", "col": 4, "row": 2, "text": "hi" },
            ]),
        );
        s.resize(16, 8);
        let c = s.canvas();
        // The rect is the bottom right quarter of the scene, in any size.
        assert!(c.get(16, 16) && c.get(31, 31) && !c.get(14, 14));
        assert_eq!(c.cell(8, 4).ch, 'h', "the text goes to the same place");
    }

    #[test]
    fn a_plot_with_no_box_fills_the_new_size() {
        let mut s = Scene::new(4, 2);
        draw(&mut s, json!({ "op": "plot", "values": [0, 1] }));
        s.resize(8, 4);
        assert!(s.canvas().get(0, 15) && s.canvas().get(15, 0));
    }

    #[test]
    fn clear_forgets_the_old_commands_but_not_the_color() {
        let red = Rgb::new(255, 0, 0);
        let mut s = Scene::new(4, 2);
        draw(
            &mut s,
            json!([{ "op": "dot", "x": 7, "y": 7 }, { "op": "color", "color": "#ff0000" }, { "op": "clear" }]),
        );
        draw(&mut s, json!({ "op": "dot", "x": 0, "y": 0 }));
        assert_eq!(s.kept(), 2, "the color, then the dot");
        s.resize(8, 4);
        assert!(!s.canvas().get(15, 15), "the dot before clear is gone");
        assert!(s.canvas().get(0, 0));
        assert_eq!(
            s.canvas().cell(0, 0).color,
            Some(red),
            "the color from before clear is kept"
        );
    }

    #[test]
    fn a_bad_batch_is_not_kept() {
        let mut s = Scene::new(4, 2);
        let bad = parse_ops(
            &json!([{ "op": "dot", "x": 1, "y": 1 }, { "op": "color", "color": "pink" }]),
        )
        .unwrap();
        assert!(s.draw(&bad).is_err());
        assert_eq!(s.kept(), 0);
        s.resize(8, 4);
        assert!(!s.canvas().get(2, 2));
    }

    #[test]
    fn a_long_live_chart_keeps_only_the_newest_commands() {
        let mut s = Scene::new(4, 2);
        let batch = parse_ops(&json!(
            (0..1000)
                .map(|i| json!({ "op": "dot", "x": i % 8, "y": 0 }))
                .collect::<Vec<_>>()
        ))
        .unwrap();
        for _ in 0..150 {
            s.draw(&batch).unwrap();
        }
        assert!(s.kept() <= MAX_OPS, "{}", s.kept());
        assert!(s.kept() >= MAX_OPS - 1000);
    }

    #[test]
    fn the_size_change_is_told_once() {
        let mut s = Scene::new(4, 2);
        assert!(!s.take_resized());
        s.resize(4, 2);
        assert!(!s.take_resized(), "the same size is no change");
        s.resize(5, 2);
        assert!(s.take_resized());
        assert!(!s.take_resized());
    }
}
