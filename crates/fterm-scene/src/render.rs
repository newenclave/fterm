//! The canvas as terminal bytes, for a scene pane: the pane is a terminal grid that fterm draws into.

use crate::canvas::{Canvas, Rgb};

/// The bytes that draw the whole canvas from the top left cell: no cursor, each row in its place,
/// and a color only where it changes.
pub fn render(canvas: &Canvas) -> Vec<u8> {
    use std::fmt::Write;
    let mut out = String::from("\x1b[?25l\x1b[0m");
    let mut color: Option<Rgb> = None;
    for row in 0..canvas.rows() {
        let _ = write!(out, "\x1b[{};1H", row + 1);
        for col in 0..canvas.cols() {
            let cell = canvas.cell(col, row);
            if cell.color != color {
                match cell.color {
                    Some(c) => {
                        let _ = write!(out, "\x1b[38;2;{};{};{}m", c.r, c.g, c.b);
                    }
                    None => out.push_str("\x1b[39m"),
                }
                color = cell.color;
            }
            out.push(cell.ch);
        }
    }
    out.push_str("\x1b[0m");
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap().replace('\x1b', "^")
    }

    #[test]
    fn rows_go_to_their_places() {
        let mut c = Canvas::new(2, 2);
        c.set(0, 0);
        c.text(1, 1, "x");
        assert_eq!(text(render(&c)), "^[?25l^[0m^[1;1H⠁ ^[2;1H x^[0m");
    }

    #[test]
    fn colors_change_only_where_they_change() {
        let red = Rgb::new(255, 0, 0);
        let mut c = Canvas::new(4, 1);
        c.set_pen(Some(red));
        c.text(1, 0, "ab");
        c.set_pen(None);
        c.text(3, 0, "c");
        assert_eq!(
            text(render(&c)),
            "^[?25l^[0m^[1;1H ^[38;2;255;0;0mab^[39mc^[0m"
        );
    }
}
