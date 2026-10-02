//! Drawing commands as JSON, for the API and MCP: `[{"op":"line","x0":0,"y0":0,"x1":9,"y1":5}, …]`.

use serde::Deserialize;

use crate::canvas::{Canvas, Rgb};

/// One drawing command. Coordinates are dots, except `text` (cells).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// `"color": "#rrggbb"` for the next commands; no color = the terminal's text color.
    Color {
        #[serde(default)]
        color: Option<String>,
    },
    Dot {
        x: i32,
        y: i32,
    },
    Undot {
        x: i32,
        y: i32,
    },
    Line {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
    },
    Rect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        #[serde(default)]
        fill: bool,
    },
    Circle {
        x: i32,
        y: i32,
        r: i32,
        #[serde(default)]
        fill: bool,
    },
    Text {
        col: i32,
        row: i32,
        text: String,
    },
    Clear,
    /// A chart of `values` in a box (no box = the whole scene): a line from left to right, or bars.
    /// The scale is from `min` to `max` (no value = from the data); a bigger value is higher.
    Plot {
        values: Vec<f64>,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        #[serde(default)]
        bars: bool,
        #[serde(default)]
        x: Option<i32>,
        #[serde(default)]
        y: Option<i32>,
        #[serde(default)]
        w: Option<i32>,
        #[serde(default)]
        h: Option<i32>,
    },
}

/// The commands from JSON: one command or a list.
pub fn parse_ops(json: &serde_json::Value) -> Result<Vec<Op>, String> {
    let items = match json {
        serde_json::Value::Array(items) => items.as_slice(),
        one => std::slice::from_ref(one),
    };
    items
        .iter()
        .enumerate()
        .map(|(i, item)| Op::deserialize(item).map_err(|err| format!("[{i}]: {err}")))
        .collect()
}

/// The colors of the commands, in order. A bad one is an error (it names its place).
pub fn colors(ops: &[Op]) -> Result<Vec<Rgb>, String> {
    let mut colors = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        if let Op::Color { color: Some(text) } = op {
            match Rgb::parse(text) {
                Some(rgb) => colors.push(rgb),
                None => return Err(format!("[{i}]: `{text}` is not a color (use #rrggbb)")),
            }
        }
    }
    Ok(colors)
}

/// Draws the commands. A bad color stops before anything is drawn.
pub fn apply(canvas: &mut Canvas, ops: &[Op]) -> Result<(), String> {
    // The colors first, so a bad one draws nothing.
    let mut colors = colors(ops)?.into_iter();
    for op in ops {
        match op {
            Op::Color { color } => canvas.set_pen(color.as_ref().and_then(|_| colors.next())),
            Op::Dot { x, y } => canvas.set(*x, *y),
            Op::Undot { x, y } => canvas.unset(*x, *y),
            Op::Line { x0, y0, x1, y1 } => canvas.line(*x0, *y0, *x1, *y1),
            Op::Rect { x, y, w, h, fill } => canvas.rect(*x, *y, *w, *h, *fill),
            Op::Circle { x, y, r, fill } => canvas.circle(*x, *y, *r, *fill),
            Op::Text { col, row, text } => canvas.text(*col, *row, text),
            Op::Clear => canvas.clear(),
            Op::Plot {
                values,
                min,
                max,
                bars,
                x,
                y,
                w,
                h,
            } => plot(canvas, values, (*min, *max), *bars, (*x, *y, *w, *h)),
        }
    }
    Ok(())
}

/// `plot`: the values from left to right in the box, the smallest at its bottom.
fn plot(
    canvas: &mut Canvas,
    values: &[f64],
    (min, max): (Option<f64>, Option<f64>),
    bars: bool,
    (x, y, w, h): (Option<i32>, Option<i32>, Option<i32>, Option<i32>),
) {
    let values: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if values.is_empty() {
        return;
    }
    let (x0, y0) = (i64::from(x.unwrap_or(0)), i64::from(y.unwrap_or(0)));
    let w = w.map_or(canvas.width() as i64 - x0, i64::from);
    let h = h.map_or(canvas.height() as i64 - y0, i64::from);
    if w <= 0 || h <= 0 {
        return;
    }
    let lo = min.unwrap_or_else(|| values.iter().copied().fold(f64::INFINITY, f64::min));
    let hi = max.unwrap_or_else(|| values.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    let n = values.len() as i64;
    let point = |i: i64, v: f64| {
        let px = if n == 1 {
            x0
        } else {
            x0 + ((i * (w - 1)) as f64 / (n - 1) as f64).round() as i64
        };
        // How high in the box: 0 at the bottom; all values the same = the middle.
        let level = if hi > lo {
            ((v.clamp(lo, hi) - lo) / (hi - lo) * (h - 1) as f64).round() as i64
        } else {
            (h - 1) / 2
        };
        (px, y0 + (h - 1) - level)
    };
    let to_i32 = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    let bottom = to_i32(y0 + h - 1);
    let mut last = None;
    for (i, v) in values.iter().enumerate() {
        let (px, py) = point(i as i64, *v);
        let (px, py) = (to_i32(px), to_i32(py));
        if bars {
            // Each bar has its part of the box, with one empty dot after it (when there is room).
            let i = i as i64;
            let left = x0 + i * w / n;
            let right = (x0 + (i + 1) * w / n - 2).max(left);
            canvas.rect(
                to_i32(left),
                py,
                to_i32(right - left + 1),
                bottom - py + 1,
                true,
            );
        } else {
            let (lx, ly) = last.unwrap_or((px, py));
            canvas.line(lx, ly, px, py);
            last = Some((px, py));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn commands_come_from_json() {
        let ops = parse_ops(&json!([
            { "op": "color", "color": "#ff0000" },
            { "op": "line", "x0": 0, "y0": 0, "x1": 3, "y1": 0 },
            { "op": "rect", "x": 0, "y": 0, "w": 2, "h": 2 },
            { "op": "text", "col": 1, "row": 0, "text": "ok" },
            { "op": "clear" },
        ]))
        .unwrap();
        assert_eq!(ops.len(), 5);
        assert_eq!(
            ops[2],
            Op::Rect {
                x: 0,
                y: 0,
                w: 2,
                h: 2,
                fill: false
            }
        );
        // One command alone is a list of one.
        assert_eq!(
            parse_ops(&json!({ "op": "dot", "x": 1, "y": 2 })).unwrap(),
            [Op::Dot { x: 1, y: 2 }]
        );
    }

    #[test]
    fn a_wrong_command_says_what_is_wrong() {
        let err = parse_ops(&json!([{ "op": "fly" }])).unwrap_err();
        assert!(err.contains("fly"), "{err}");
        let err = parse_ops(&json!([{ "op": "dot", "x": 1 }])).unwrap_err();
        assert!(err.contains('y'), "{err}");
        let err = parse_ops(&json!([{ "op": "dot", "x": 1, "y": 1, "z": 3 }])).unwrap_err();
        assert!(err.contains('z'), "{err}");
        // The place of the bad command.
        let err = parse_ops(&json!([{ "op": "clear" }, { "op": "fly" }])).unwrap_err();
        assert!(err.contains("[1]"), "{err}");
    }

    #[test]
    fn commands_draw() {
        let mut c = Canvas::new(3, 1);
        let ops = parse_ops(&json!([
            { "op": "line", "x0": 0, "y0": 0, "x1": 5, "y1": 0 },
            { "op": "undot", "x": 5, "y": 0 },
            { "op": "color", "color": "#00ff00" },
            { "op": "text", "col": 2, "row": 0, "text": "!" },
        ]))
        .unwrap();
        apply(&mut c, &ops).unwrap();
        assert_eq!(c.rows_text(), ["⠉⠉!"]);
        assert_eq!(c.cell(2, 0).color, Some(Rgb::new(0, 255, 0)));
        assert_eq!(c.cell(0, 0).color, None);
    }

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

    fn plot(c: &mut Canvas, json: serde_json::Value) {
        apply(c, &parse_ops(&json).unwrap()).unwrap();
    }

    #[test]
    fn a_line_chart_goes_up_with_the_values() {
        // A 12x12 dot scene; the box is 11x11 from (0, 0).
        let mut c = Canvas::new(6, 3);
        plot(
            &mut c,
            json!({ "op": "plot", "values": [0, 10], "x": 0, "y": 0, "w": 11, "h": 11 }),
        );
        let d = dots(&c);
        // The smallest value is at the bottom of the box, the biggest at the top.
        assert!(d.contains(&(0, 10)) && d.contains(&(10, 0)), "{d:?}");
        assert_eq!(d.len(), 11);
        // Three values: the middle one in the middle of the box.
        let mut c = Canvas::new(6, 3);
        plot(
            &mut c,
            json!({ "op": "plot", "values": [0, 5, 10], "x": 0, "y": 0, "w": 11, "h": 11 }),
        );
        assert!(c.get(5, 5));
    }

    #[test]
    fn the_box_is_the_whole_scene_by_default() {
        let mut c = Canvas::new(4, 2);
        plot(&mut c, json!({ "op": "plot", "values": [1, 2] }));
        assert!(c.get(0, 7) && c.get(7, 0), "{:?}", dots(&c));
    }

    #[test]
    fn min_and_max_set_the_scale() {
        let mut c = Canvas::new(6, 3);
        // 5 on a scale of 0..10 is in the middle; values out of the scale stay at its edges.
        plot(
            &mut c,
            json!({ "op": "plot", "values": [5, 5], "min": 0, "max": 10, "x": 0, "y": 0, "w": 11, "h": 11 }),
        );
        assert_eq!(dots(&c), (0..=10).map(|x| (x, 5)).collect::<Vec<_>>());
        let mut c = Canvas::new(6, 3);
        plot(
            &mut c,
            json!({ "op": "plot", "values": [-50, 99], "min": 0, "max": 10, "x": 0, "y": 0, "w": 11, "h": 11 }),
        );
        assert!(c.get(0, 10) && c.get(10, 0));
        // All values the same, no scale: a line in the middle.
        let mut c = Canvas::new(6, 3);
        plot(
            &mut c,
            json!({ "op": "plot", "values": [3, 3, 3], "x": 0, "y": 0, "w": 11, "h": 11 }),
        );
        assert!(c.get(0, 5) && c.get(10, 5));
    }

    #[test]
    fn bars_stand_on_the_bottom() {
        let mut c = Canvas::new(2, 3);
        plot(
            &mut c,
            json!({ "op": "plot", "values": [0, 5, 10], "bars": true, "x": 0, "y": 0, "w": 3, "h": 11 }),
        );
        let column = |x: i32| (0..12).filter(|&y| c.get(x, y)).collect::<Vec<_>>();
        assert_eq!(column(0), [10]);
        assert_eq!(column(1), (5..=10).collect::<Vec<_>>());
        assert_eq!(column(2), (0..=10).collect::<Vec<_>>());

        // With room, a bar fills its part of the box, with one empty dot between bars.
        let mut c = Canvas::new(5, 1);
        plot(
            &mut c,
            json!({ "op": "plot", "values": [1, 1], "bars": true, "min": 0, "max": 1, "x": 0, "y": 0, "w": 10, "h": 4 }),
        );
        let filled: Vec<i32> = (0..10).filter(|&x| c.get(x, 0)).collect();
        assert_eq!(filled, [0, 1, 2, 3, 5, 6, 7, 8]);
    }

    #[test]
    fn more_values_than_dots_and_no_values() {
        let mut c = Canvas::new(2, 1);
        let many: Vec<f64> = (0..1000).map(|i| (i % 7) as f64).collect();
        plot(&mut c, json!({ "op": "plot", "values": many }));
        assert!(!dots(&c).is_empty());
        let mut c = Canvas::new(2, 1);
        plot(&mut c, json!({ "op": "plot", "values": [] }));
        assert!(dots(&c).is_empty());
        // One value: one dot at the left.
        let mut c = Canvas::new(2, 1);
        plot(
            &mut c,
            json!({ "op": "plot", "values": [4], "min": 0, "max": 4 }),
        );
        assert_eq!(dots(&c), [(0, 0)]);
    }

    #[test]
    fn a_bad_color_draws_nothing() {
        let mut c = Canvas::new(2, 1);
        let ops = parse_ops(&json!([
            { "op": "dot", "x": 0, "y": 0 },
            { "op": "color", "color": "pink" },
        ]))
        .unwrap();
        let err = apply(&mut c, &ops).unwrap_err();
        assert!(err.contains("pink"), "{err}");
        assert_eq!(c.rows_text(), ["  "]);
    }
}
