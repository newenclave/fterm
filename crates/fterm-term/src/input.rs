//! The text that the user typed at the prompt: from the input start (shell integration, 133;B)
//! to the end of the input. Used by the command history (6b) and the hints.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags;

/// What is typed now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub text: String,
    /// The cursor is at the end of the text (nothing typed after it).
    pub cursor_at_end: bool,
}

/// Reads the typed text. `start` is (line from the top of the history, column), from `ShellState::input_start`.
/// `None` when the place is not on the grid any more.
pub fn typed_input<T>(term: &Term<T>, start: (usize, usize)) -> Option<Input> {
    let grid = term.grid();
    let history = grid.history_size() as i32;
    let rows = grid.screen_lines() as i32;
    let columns = grid.columns();
    let mut line = start.0 as i32 - history;
    let mut column = start.1;
    if column >= columns {
        // The prompt filled the line: the input starts on the next one.
        line += 1;
        column = 0;
    }
    if line < -history || line >= rows {
        return None;
    }
    let cursor = grid.cursor.point;
    // The input goes to the cursor line, and on to the lines that the screen wrapped.
    let mut end = cursor.line.0.max(line);
    while end < rows - 1
        && grid[Line(end)][Column(columns - 1)]
            .flags
            .contains(Flags::WRAPLINE)
    {
        end += 1;
    }

    let mut text = String::new();
    let mut chars = 0;
    let mut before_cursor = 0;
    for l in line..=end {
        let row = &grid[Line(l)];
        let first = if l == line { column } else { 0 };
        for c in first..columns {
            if Line(l) == cursor.line && c == cursor.column.0 {
                before_cursor = chars;
            }
            let cell = &row[Column(c)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            text.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra);
            }
            chars += 1;
        }
    }
    if cursor.line.0 > end {
        before_cursor = chars;
    }
    let text = text.trim_end().to_owned();
    let length = text.chars().count();
    Some(Input {
        cursor_at_end: before_cursor >= length,
        text,
    })
}

/// All lines of the history and the screen (line 0 = the oldest line of the history).
pub fn total_lines<T>(term: &Term<T>) -> usize {
    term.grid().history_size() + term.grid().screen_lines()
}

/// The text of the lines `from..to` (lines from the top of the history). Lines that the screen wrapped
/// are one line again; spaces at the ends are gone.
pub fn lines_text<T>(term: &Term<T>, from: usize, to: usize) -> String {
    let grid = term.grid();
    let history = grid.history_size() as i32;
    let columns = grid.columns();
    let to = to.min(total_lines(term));
    let mut lines = Vec::new();
    let mut line_text = String::new();
    for abs in from..to {
        let row = &grid[Line(abs as i32 - history)];
        for c in 0..columns {
            let cell = &row[Column(c)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            line_text.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                line_text.extend(extra);
            }
        }
        // A line that the screen wrapped goes on in the next row.
        let wrapped = row[Column(columns - 1)].flags.contains(Flags::WRAPLINE);
        if !wrapped || abs + 1 == to {
            lines.push(line_text.trim_end().to_owned());
            line_text.clear();
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::size::GridSize;

    fn term(columns: usize, rows: usize, bytes: &str) -> Term<VoidListener> {
        let mut term = Term::new(
            Config::default(),
            &GridSize::new(columns, rows),
            VoidListener,
        );
        Processor::<StdSyncHandler>::new().advance(&mut term, bytes.as_bytes());
        term
    }

    fn input(text: &str, cursor_at_end: bool) -> Option<Input> {
        Some(Input {
            text: text.to_owned(),
            cursor_at_end,
        })
    }

    #[test]
    fn lines_of_the_history_and_the_screen() {
        let t = term(10, 3, "one\r\ntwo\r\nthree\r\nfour\r\nabcdefghijkl");
        // 6 lines: one two three four abcdefghij kl -> 3 in the history.
        assert_eq!(total_lines(&t), 6);
        assert_eq!(lines_text(&t, 0, 2), "one\ntwo");
        assert_eq!(lines_text(&t, 2, 4), "three\nfour");
        assert_eq!(
            lines_text(&t, 4, 6),
            "abcdefghijkl",
            "a wrapped line is one line"
        );
        assert_eq!(
            lines_text(&t, 3, 100),
            "four\nabcdefghijkl",
            "the end is cut"
        );
        assert_eq!(lines_text(&t, 5, 2), "", "an empty range");
    }

    #[test]
    fn the_text_after_the_prompt() {
        let t = term(20, 4, "PS> git sta");
        assert_eq!(typed_input(&t, (0, 4)), input("git sta", true));
    }

    #[test]
    fn nothing_typed() {
        let t = term(20, 4, "PS> ");
        assert_eq!(typed_input(&t, (0, 4)), input("", true));
    }

    #[test]
    fn the_cursor_in_the_middle() {
        // Type "git status", then move the cursor 3 cells to the left.
        let t = term(20, 4, "PS> git status\x1b[3D");
        assert_eq!(typed_input(&t, (0, 4)), input("git status", false));
    }

    #[test]
    fn a_long_line_wraps() {
        let t = term(10, 4, "PS> abcdefghijkl");
        assert_eq!(typed_input(&t, (0, 4)), input("abcdefghijkl", true));
    }

    #[test]
    fn the_place_counts_the_history() {
        // 6 lines on a 3-line screen: 3 lines went to the history. The prompt is on history line 5.
        let t = term(20, 3, "a\r\nb\r\nc\r\nd\r\ne\r\nPS> ls");
        assert_eq!(t.grid().history_size(), 3);
        assert_eq!(typed_input(&t, (5, 4)), input("ls", true));
        // A place that is far below the screen is not valid.
        assert_eq!(typed_input(&t, (50, 0)), None);
    }

    #[test]
    fn wide_chars_are_one_char() {
        let t = term(20, 4, "PS> 日本");
        assert_eq!(typed_input(&t, (0, 4)), input("日本", true));
    }
}
