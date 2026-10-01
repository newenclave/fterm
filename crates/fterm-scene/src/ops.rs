//! Drawing commands as JSON, for the API and MCP: `[{"op":"line","x0":0,"y0":0,"x1":9,"y1":5}, …]`.

use serde::Deserialize;

use crate::canvas::{Canvas, Rgb};

/// One drawing command. Coordinates are dots, except `text` (cells).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
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

/// Draws the commands. A bad color stops before anything is drawn.
pub fn apply(canvas: &mut Canvas, ops: &[Op]) -> Result<(), String> {
    // The colors first, so a bad one draws nothing.
    let mut colors = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        if let Op::Color { color: Some(text) } = op {
            match Rgb::parse(text) {
                Some(rgb) => colors.push(rgb),
                None => return Err(format!("[{i}]: `{text}` is not a color (use #rrggbb)")),
            }
        }
    }
    let mut colors = colors.into_iter();
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
        }
    }
    Ok(())
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
