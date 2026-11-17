//! The text that the user types: one line or many, with a cursor.

use unicode_width::UnicodeWidthChar;

use crate::text::cut;

/// The text that the user types (many lines). The cursor is a char index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputBox {
    pub text: String,
    cursor: usize,
}

impl InputBox {
    /// The cursor as a char index.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Puts the cursor at a char index (kept in the text).
    pub fn set_cursor(&mut self, at: usize) {
        self.cursor = at.min(self.text.chars().count());
    }

    pub fn insert(&mut self, text: &str) {
        self.clamp();
        let at = self.byte(self.cursor);
        self.text.insert_str(at, text);
        self.cursor += text.chars().count();
    }

    pub fn backspace(&mut self) {
        self.clamp();
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let at = self.byte(self.cursor);
        self.text.remove(at);
    }

    pub fn delete(&mut self) {
        self.clamp();
        if self.cursor < self.text.chars().count() {
            let at = self.byte(self.cursor);
            self.text.remove(at);
        }
    }

    pub fn left(&mut self) {
        self.clamp();
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.clamp();
        self.cursor = (self.cursor + 1).min(self.text.chars().count());
    }

    /// The start / the end of the current line.
    pub fn home(&mut self) {
        self.clamp();
        let before: Vec<char> = self.text.chars().take(self.cursor).collect();
        self.cursor = before.iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
    }

    pub fn end(&mut self) {
        self.clamp();
        let after = self.text.chars().skip(self.cursor).position(|c| c == '\n');
        self.cursor = match after {
            Some(n) => self.cursor + n,
            None => self.text.chars().count(),
        };
    }

    /// One screen row up, at the same cell (as far as the row goes), when the text is `width` cells wide.
    /// `false` = the cursor is on the first row (a form then goes to the field above).
    pub fn up(&mut self, width: usize) -> bool {
        self.go_row(width, false)
    }

    /// One screen row down; `false` = the cursor is on the last row.
    pub fn down(&mut self, width: usize) -> bool {
        self.go_row(width, true)
    }

    fn go_row(&mut self, width: usize, down: bool) -> bool {
        self.clamp();
        let places = self.places(width);
        let (row, cell) = places[self.cursor];
        let target = match (down, row) {
            (false, 0) => return false,
            (false, r) => r - 1,
            (true, r) => r + 1,
        };
        // The place on the target row that is nearest to the cell, not past it.
        let best = places
            .iter()
            .enumerate()
            .filter(|(_, p)| p.0 == target && p.1 <= cell)
            .max_by_key(|(_, p)| p.1)
            .or_else(|| places.iter().enumerate().find(|(_, p)| p.0 == target));
        match best {
            Some((i, _)) => {
                self.cursor = i;
                true
            }
            None => false,
        }
    }

    /// The screen place (row, cell) of each cursor place (char index 0..=len) when lines are cut at `width`
    /// cells, by the same rule as `cut`. A line that fills its last row gets one more (empty) row for the cursor
    /// at its end, so the height of the box does not change when the cursor moves (`layout` draws that row too).
    fn places(&self, width: usize) -> Vec<(usize, usize)> {
        let width = width.max(1);
        let mut out = Vec::with_capacity(self.text.len() + 1);
        let mut row = 0;
        for (n, logical) in self.text.split('\n').enumerate() {
            if n > 0 {
                row += 1;
            }
            let mut used = 0;
            let mut last = (row, 0);
            let mut any = false;
            for c in logical.chars() {
                let w = c.width().unwrap_or(0);
                if w == 0 && used >= width && any {
                    // A mark on the last char of a full row: the cursor before it is on that char.
                    out.push(last);
                    continue;
                }
                if used + w > width && any {
                    row += 1;
                    used = 0;
                }
                last = (row, used);
                out.push(last);
                used += w;
                any = true;
            }
            // The end of the line (the place of its '\n', or the end of the text).
            if any && used >= width {
                row += 1;
                used = 0;
            }
            out.push((row, used));
        }
        out
    }

    /// Takes the text out (to send it) and empties the box.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_owned();
        self.cursor = self.text.chars().count();
    }

    /// The text on one line with `mark` at the place of the cursor (for a small box with one line).
    pub fn line_with_cursor(&self, mark: char) -> String {
        let mut line = self.text.clone();
        line.insert(self.byte(self.cursor), mark);
        line
    }

    /// The byte place of a char index.
    fn byte(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(i, _)| i)
    }

    /// The lines to draw (each line cut at `width` cells; a line that fills its last row is followed by an empty
    /// row) and the place of the cursor (line, cell).
    pub fn layout(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(1);
        let mut lines = Vec::new();
        for logical in self.text.split('\n') {
            let pieces = cut(logical, width);
            let full = pieces
                .last()
                .is_some_and(|last| !last.is_empty() && crate::text::cells(last) >= width);
            lines.extend(pieces);
            if full {
                lines.push(String::new());
            }
        }
        let places = self.places(width);
        let cursor = places[self.cursor.min(places.len() - 1)];
        (lines, cursor)
    }

    /// Keeps the cursor in the text (`text` is public: a caller may set a shorter text).
    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.text.chars().count());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_moving() {
        let mut b = InputBox::default();
        b.insert("helo");
        b.left();
        b.insert("l");
        assert_eq!((b.text.as_str(), b.cursor()), ("hello", 4));
        b.end();
        b.insert("\nwörld");
        b.home();
        assert_eq!(b.cursor(), 6, "the start of the second line");
        b.delete();
        assert_eq!(b.text, "hello\nörld");
        b.right();
        b.backspace();
        assert_eq!(b.text, "hello\nrld");
        b.home();
        b.backspace();
        assert_eq!(
            b.text, "hellorld",
            "backspace at a line start joins the lines"
        );
        assert_eq!(b.take(), "hellorld");
        assert_eq!((b.text.as_str(), b.cursor()), ("", 0));
        b.backspace();
        b.left();
        b.delete();
        assert_eq!(b.text, "", "nothing breaks at the ends");
    }

    #[test]
    fn one_line_with_the_cursor_in_its_place() {
        let mut b = InputBox::default();
        assert_eq!(b.line_with_cursor('|'), "|");
        b.insert("ping 1");
        assert_eq!(b.line_with_cursor('|'), "ping 1|");
        b.home();
        assert_eq!(b.line_with_cursor('|'), "|ping 1");
        b.right();
        b.right();
        assert_eq!(b.line_with_cursor('|'), "pi|ng 1");
        let mut b = InputBox::default();
        b.insert("dür");
        b.left();
        assert_eq!(
            b.line_with_cursor('|'),
            "dü|r",
            "the cursor counts chars, not bytes"
        );
    }

    #[test]
    fn the_input_layout() {
        let mut b = InputBox::default();
        b.insert("abcdefgh\nxy");
        let (lines, cursor) = b.layout(5);
        assert_eq!(lines, ["abcde", "fgh", "xy"]);
        assert_eq!(cursor, (2, 2));
        b.set("");
        assert_eq!(b.layout(5), (vec![String::new()], (0, 0)));
    }

    #[test]
    fn a_full_line_gives_the_cursor_a_row_of_its_own() {
        // Before: the cursor went to row 1, but there was no row 1 to draw, so it was not seen.
        let mut b = InputBox::default();
        b.set("abcde");
        assert_eq!(b.layout(5), (vec!["abcde".into(), String::new()], (1, 0)));
        b.left();
        assert_eq!(
            b.layout(5),
            (vec!["abcde".into(), String::new()], (0, 4)),
            "the row stays when the cursor goes back, so the box keeps its height"
        );
    }

    #[test]
    fn up_and_down_see_the_row_after_a_full_line() {
        let mut b = InputBox::default();
        b.set("abcdefghij");
        // Rows at width 5: "abcde", "fghij", "" (the cursor at the end is on the empty row).
        assert_eq!(b.layout(5).1, (2, 0));
        assert!(b.up(5));
        assert_eq!(
            b.layout(5).1,
            (1, 0),
            "up goes to the row above, not over it"
        );
        assert!(b.down(5));
        assert_eq!(b.layout(5).1, (2, 0), "and down comes back to the end");
        b.set("abcde");
        assert!(b.up(5), "a row is above the cursor");
        assert_eq!(b.layout(5).1, (0, 0));
    }

    #[test]
    fn a_mark_after_a_full_row_adds_no_row() {
        // "e" + a combining accent (as text from macOS often is), right at the end of a full row.
        let mut b = InputBox::default();
        b.set("abcde\u{301}fg");
        b.left();
        b.left();
        b.left();
        let (lines, cursor) = b.layout(5);
        assert_eq!(lines, ["abcde\u{301}", "fg"], "no empty row in the middle");
        assert_eq!(cursor, (0, 4), "between e and its accent: on the e");
    }

    #[test]
    fn text_set_from_outside_does_not_break_the_cursor() {
        let mut b = InputBox::default();
        b.set("a long text");
        b.text = "ab".into();
        assert_eq!(
            b.layout(5),
            (vec!["ab".into()], (0, 2)),
            "the cursor is kept in the text"
        );
        assert!(!b.up(5));
        assert!(!b.down(5));
    }

    #[test]
    fn the_cursor_rows_agree_with_the_drawn_rows() {
        for text in [
            "",
            "abcde",
            "abcdefghij\nxy",
            "abcd中ef",
            "中中中",
            "abcde\u{301}fg",
            "a\n\nb",
            "x\u{200d}y",
        ] {
            for width in [1, 2, 3, 5, 8] {
                let mut b = InputBox::default();
                b.set(text);
                let rows = b.layout(width).0.len();
                for _ in 0..=text.chars().count() {
                    let (lines, (row, cell)) = b.layout(width);
                    assert_eq!(lines.len(), rows, "{text:?} at {width}: the height stays");
                    assert!(
                        row < rows,
                        "{text:?} at {width}: the cursor row {row} is drawn"
                    );
                    assert!(cell <= width, "{text:?} at {width}");
                    b.left();
                }
            }
        }
    }

    #[test]
    fn the_cursor_after_a_wide_char_that_went_to_the_next_row() {
        // "abcd" + "中" at width 5: the wide char does not fit and goes to row 1 (cells 0-1).
        let mut b = InputBox::default();
        b.set("abcd中");
        assert_eq!(b.layout(5), (vec!["abcd".into(), "中".into()], (1, 2)));
        b.left();
        assert_eq!(
            b.layout(5).1,
            (1, 0),
            "before the wide char: at the start of its row"
        );
    }

    #[test]
    fn up_and_down_go_by_screen_rows() {
        let mut b = InputBox::default();
        b.set("abcdefgh\nxy");
        // Rows at width 5: "abcde", "fgh", "xy". The cursor is at the end of "xy" (row 2, cell 2).
        assert!(b.up(5));
        assert_eq!(b.layout(5).1, (1, 2), "the same cell one row up");
        assert!(b.up(5));
        assert_eq!(b.layout(5).1, (0, 2));
        assert!(!b.up(5), "the first row: nothing above");
        assert_eq!(b.layout(5).1, (0, 2));
        b.right();
        b.right();
        assert_eq!(b.layout(5).1, (0, 4));
        assert!(b.down(5));
        assert_eq!(b.layout(5).1, (1, 3), "a short row: the end of it");
        assert!(b.down(5));
        assert_eq!(b.layout(5).1, (2, 2));
        assert!(!b.down(5), "the last row: nothing below");
    }
}
