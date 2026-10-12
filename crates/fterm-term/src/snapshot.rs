//! A snapshot of a Term as escape bytes: fed into a new Term of the same size, they make the same screen, history,
//! cursor, and modes. The mux server sends it when a window attaches to a pane; after it the raw output follows.
//!
//! Not in the snapshot (alacritty keeps them private): the scroll region and the tab stops. A full-screen program
//! sets them again when it draws for the new size after an attach.

use std::fmt::Write as _;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Cursor, Dimensions};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};

/// The rows of one screen buffer: `rows[i]` = cells, and if the row goes on to the next one (`WRAPLINE`).
struct Buffer {
    rows: Vec<Vec<Cell>>,
    cursor: Cursor<Cell>,
    saved_cursor: Cursor<Cell>,
    /// Lines above the screen (the first `history` rows).
    history: usize,
}

fn read_buffer<T: EventListener>(term: &Term<T>) -> Buffer {
    let grid = term.grid();
    let columns = term.columns();
    let mut rows = Vec::new();
    let mut line = term.topmost_line();
    while line <= term.bottommost_line() {
        rows.push(
            (0..columns)
                .map(|c| grid[line][Column(c)].clone())
                .collect(),
        );
        line += 1;
    }
    Buffer {
        rows,
        cursor: grid.cursor.clone(),
        saved_cursor: grid.saved_cursor.clone(),
        history: grid.history_size(),
    }
}

/// The escape bytes that make `term` again in a new Term of the same size. `term` is the same after it (the
/// alternate screen is read by swapping it out and back).
pub fn snapshot<T: EventListener>(term: &mut Term<T>) -> Vec<u8> {
    let alt = term.mode().contains(TermMode::ALT_SCREEN);
    let (primary, alternate) = if alt {
        let alternate = read_buffer(term);
        // The alternate screen is active: swap to the primary one to read it (no reset this way)...
        term.swap_alt();
        let primary = read_buffer(term);
        // ...and back. Going into the alternate screen clears it, so its cells and cursor are put back.
        term.swap_alt();
        let grid = term.grid_mut();
        for (i, row) in alternate.rows.iter().enumerate() {
            let line = Line(i as i32);
            for (c, cell) in row.iter().enumerate() {
                grid[line][Column(c)] = cell.clone();
            }
        }
        grid.cursor = alternate.cursor.clone();
        grid.saved_cursor = alternate.saved_cursor.clone();
        (primary, Some(alternate))
    } else {
        (read_buffer(term), None)
    };

    let mut out = String::from("\x1b[0m\x1b[H\x1b[2J\x1b[3J");
    write_buffer(&mut out, &primary);
    if let Some(alternate) = &alternate {
        out.push_str("\x1b[?1049h\x1b[H");
        write_buffer(&mut out, alternate);
    }
    write_colors(&mut out, term);
    write_modes(&mut out, *term.mode());
    let style = term.cursor_style();
    let shape = match style.shape {
        CursorShape::Block | CursorShape::HollowBlock | CursorShape::Hidden => 1,
        CursorShape::Underline => 3,
        CursorShape::Beam => 5,
    };
    let _ = write!(out, "\x1b[{} q", shape + usize::from(!style.blinking));
    out.into_bytes()
}

/// The rows, then the saved cursor and the cursor.
fn write_buffer(out: &mut String, buffer: &Buffer) {
    let mut pen = Pen::default();
    for (i, row) in buffer.rows.iter().enumerate() {
        let wraps = row
            .last()
            .is_some_and(|c| c.flags.contains(Flags::WRAPLINE));
        // A row that is not wrapped ends at its last cell that is not empty.
        let end = if wraps {
            row.len()
        } else {
            row.iter().rposition(|c| !is_blank(c)).map_or(0, |i| i + 1)
        };
        for cell in &row[..end] {
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            pen.set(out, cell);
            out.push(if cell.c == '\0' { ' ' } else { cell.c });
            if let Some(extra) = cell.zerowidth() {
                out.extend(extra.iter());
            }
        }
        if !wraps && i + 1 < buffer.rows.len() {
            // A plain row end; the colors of the empty cells after it are the default ones.
            pen.reset(out);
            out.push_str("\r\n");
        }
    }
    pen.reset(out);
    pen.link(out, None);
    // The rows above the screen are the history; the screen is the last rows.
    let saved = &buffer.saved_cursor;
    let _ = write!(
        out,
        "\x1b[{};{}H",
        saved.point.line.0 + 1,
        saved.point.column.0 + 1
    );
    pen.set(out, &saved.template);
    out.push_str("\x1b7");
    pen.reset(out);
    let cursor = &buffer.cursor;
    let line = cursor.point.line.0;
    if cursor.input_needs_wrap {
        // The cursor waits after the last cell of its row: write that cell again to wait there too.
        let col = cursor.point.column.0;
        let _ = write!(out, "\x1b[{};{}H", line + 1, col + 1);
        let row = &buffer.rows[buffer.history + line.max(0) as usize];
        let cell = &row[col];
        pen.set(out, cell);
        out.push(if cell.c == '\0' { ' ' } else { cell.c });
    } else {
        let _ = write!(out, "\x1b[{};{}H", line + 1, cursor.point.column.0 + 1);
    }
    pen.set(out, &cursor.template);
}

fn is_blank(cell: &Cell) -> bool {
    (cell.c == ' ' || cell.c == '\0')
        && cell.bg == Color::Named(NamedColor::Background)
        && cell.extra.is_none()
        && !cell.flags.intersects(
            Flags::INVERSE | Flags::ALL_UNDERLINES | Flags::STRIKEOUT | Flags::WIDE_CHAR,
        )
}

/// The style that was written last, so only changes are written.
#[derive(Default)]
struct Pen {
    style: Option<(Color, Color, Flags, Option<Color>)>,
    link: Option<(String, String)>,
}

impl Pen {
    fn reset(&mut self, out: &mut String) {
        if self.style.is_some() {
            out.push_str("\x1b[0m");
            self.style = None;
        }
    }

    fn link(&mut self, out: &mut String, link: Option<&Hyperlink>) {
        let next = link.map(|l| (l.id().to_owned(), l.uri().to_owned()));
        if next == self.link {
            return;
        }
        match &next {
            Some((id, uri)) => {
                let _ = write!(out, "\x1b]8;id={id};{uri}\x1b\\");
            }
            None => out.push_str("\x1b]8;;\x1b\\"),
        }
        self.link = next;
    }

    fn set(&mut self, out: &mut String, cell: &Cell) {
        self.link(out, cell.hyperlink().as_ref());
        let style = (
            cell.fg,
            cell.bg,
            cell.flags & STYLE_FLAGS,
            cell.underline_color(),
        );
        if self.style == Some(style) {
            return;
        }
        let mut sgr = vec!["0".to_owned()];
        let f = style.2;
        for (flag, code) in [
            (Flags::BOLD, "1"),
            (Flags::DIM, "2"),
            (Flags::ITALIC, "3"),
            (Flags::INVERSE, "7"),
            (Flags::HIDDEN, "8"),
            (Flags::STRIKEOUT, "9"),
        ] {
            if f.contains(flag) {
                sgr.push(code.to_owned());
            }
        }
        for (flag, code) in [
            (Flags::UNDERLINE, "4"),
            (Flags::DOUBLE_UNDERLINE, "4:2"),
            (Flags::UNDERCURL, "4:3"),
            (Flags::DOTTED_UNDERLINE, "4:4"),
            (Flags::DASHED_UNDERLINE, "4:5"),
        ] {
            if f.contains(flag) {
                sgr.push(code.to_owned());
            }
        }
        if let Some(c) = color_sgr(style.0, 38) {
            sgr.push(c);
        }
        if let Some(c) = color_sgr(style.1, 48) {
            sgr.push(c);
        }
        if let Some(c) = style.3.and_then(|c| color_sgr(c, 58)) {
            sgr.push(c);
        }
        let _ = write!(out, "\x1b[{}m", sgr.join(";"));
        self.style = Some(style);
    }
}

/// The flags that are the style of a cell (the others are about its place).
const STYLE_FLAGS: Flags = Flags::BOLD
    .union(Flags::DIM)
    .union(Flags::ITALIC)
    .union(Flags::INVERSE)
    .union(Flags::HIDDEN)
    .union(Flags::STRIKEOUT)
    .union(Flags::ALL_UNDERLINES);

/// The SGR of a color: `base` 38 (text), 48 (background), or 58 (underline). `None` = the default color.
fn color_sgr(color: Color, base: u8) -> Option<String> {
    match color {
        Color::Spec(rgb) => Some(format!("{base}:2::{}:{}:{}", rgb.r, rgb.g, rgb.b)),
        Color::Indexed(i) => Some(format!("{base}:5:{i}")),
        Color::Named(named) => {
            let i = named as usize;
            match named {
                NamedColor::Foreground | NamedColor::Background => None,
                _ if i < 8 && base != 58 => Some(format!("{}", base - 8 + i as u8)),
                _ if i < 16 && base != 58 => Some(format!("{}", base + 52 + (i as u8 - 8))),
                _ if i < 256 => Some(format!("{base}:5:{i}")),
                // The dim and bright foreground colors come from the flags; the plain color is fine.
                _ => None,
            }
        }
    }
}

/// The palette colors that a program changed (OSC 4, 10, 11, 12).
fn write_colors<T: EventListener>(out: &mut String, term: &Term<T>) {
    let colors = term.colors();
    for i in 0..256 {
        if let Some(c) = colors[i] {
            let _ = write!(
                out,
                "\x1b]4;{i};rgb:{:02x}/{:02x}/{:02x}\x1b\\",
                c.r, c.g, c.b
            );
        }
    }
    for (index, osc) in [
        (NamedColor::Foreground, 10),
        (NamedColor::Background, 11),
        (NamedColor::Cursor, 12),
    ] {
        if let Some(c) = colors[index] {
            let _ = write!(
                out,
                "\x1b]{osc};rgb:{:02x}/{:02x}/{:02x}\x1b\\",
                c.r, c.g, c.b
            );
        }
    }
}

/// Every mode, set or reset (a new Term has its own defaults, so both ways are written).
fn write_modes(out: &mut String, mode: TermMode) {
    let private = [
        (TermMode::APP_CURSOR, 1),
        (TermMode::ORIGIN, 6),
        (TermMode::LINE_WRAP, 7),
        (TermMode::SHOW_CURSOR, 25),
        (TermMode::MOUSE_REPORT_CLICK, 1000),
        (TermMode::MOUSE_DRAG, 1002),
        (TermMode::MOUSE_MOTION, 1003),
        (TermMode::FOCUS_IN_OUT, 1004),
        (TermMode::UTF8_MOUSE, 1005),
        (TermMode::SGR_MOUSE, 1006),
        (TermMode::ALTERNATE_SCROLL, 1007),
        (TermMode::URGENCY_HINTS, 1042),
        (TermMode::BRACKETED_PASTE, 2004),
    ];
    for (flag, code) in private {
        let _ = write!(
            out,
            "\x1b[?{code}{}",
            if mode.contains(flag) { 'h' } else { 'l' }
        );
    }
    for (flag, code) in [(TermMode::INSERT, 4), (TermMode::LINE_FEED_NEW_LINE, 20)] {
        let _ = write!(
            out,
            "\x1b[{code}{}",
            if mode.contains(flag) { 'h' } else { 'l' }
        );
    }
    out.push_str(if mode.contains(TermMode::APP_KEYPAD) {
        "\x1b="
    } else {
        "\x1b>"
    });
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::size::GridSize;

    fn term(columns: usize, rows: usize) -> Term<VoidListener> {
        let config = Config {
            scrolling_history: 100,
            ..Config::default()
        };
        Term::new(config, &GridSize::new(columns, rows), VoidListener)
    }

    fn feed(term: &mut Term<VoidListener>, bytes: &[u8]) {
        Processor::<StdSyncHandler>::new().advance(term, bytes);
    }

    /// Everything that a window shows or uses.
    fn state(term: &Term<VoidListener>) -> String {
        let grid = term.grid();
        let mut s = format!(
            "history {} cursor {:?} wrap {} mode {:?} style {:?}\n",
            grid.history_size(),
            grid.cursor.point,
            grid.cursor.input_needs_wrap,
            *term.mode() & !TermMode::empty(),
            term.cursor_style(),
        );
        let mut line = term.topmost_line();
        while line <= term.bottommost_line() {
            for c in 0..term.columns() {
                let cell = &grid[line][Column(c)];
                let _ = write!(
                    s,
                    "[{:?} {:?} {:?} {:?} {:?} {:?} {:?}]",
                    cell.c,
                    cell.fg,
                    cell.bg,
                    cell.flags,
                    cell.zerowidth(),
                    cell.underline_color(),
                    cell.hyperlink()
                        .map(|h| (h.id().to_owned(), h.uri().to_owned())),
                );
            }
            s.push('\n');
            line += 1;
        }
        let _ = write!(s, "template {:?}", grid.cursor.template);
        s
    }

    /// Feeds `bytes` into a Term, makes its snapshot, and feeds that into a new Term: both must be the same.
    fn round_trip(
        columns: usize,
        rows: usize,
        bytes: &[u8],
    ) -> (Term<VoidListener>, Term<VoidListener>) {
        let mut a = term(columns, rows);
        feed(&mut a, bytes);
        let before = state(&a);
        let snap = snapshot(&mut a);
        assert_eq!(state(&a), before, "the snapshot changed the Term");
        let mut b = term(columns, rows);
        feed(&mut b, &snap);
        assert_eq!(
            state(&b),
            state(&a),
            "snapshot: {:?}",
            String::from_utf8_lossy(&snap)
        );
        (a, b)
    }

    #[test]
    fn plain_text_and_the_cursor() {
        round_trip(20, 5, b"hello\r\nworld");
        round_trip(20, 5, b"");
        round_trip(20, 5, b"a\r\n\r\n\r\n   b   \r\n");
    }

    #[test]
    fn colors_and_styles() {
        round_trip(
            30,
            4,
            b"\x1b[1;31mred\x1b[0m \x1b[38;5;200mpink\x1b[48;2;1;2;3m bg \x1b[0m\x1b[4:3;58;5;9mcurl\x1b[0m \x1b[7;9;3mx",
        );
        // The pen at the end goes on: a style that is set but not used yet.
        round_trip(20, 3, b"ab\x1b[32;44m");
        round_trip(20, 3, b"\x1b[97;100mbright\x1b[2mdim");
    }

    #[test]
    fn history_and_wrapped_lines() {
        let mut text = Vec::new();
        for i in 0..40 {
            text.extend_from_slice(format!("line {i}\r\n").as_bytes());
        }
        text.extend_from_slice(b"a long line that goes on to the next rows of the screen");
        let (a, _) = round_trip(12, 5, &text);
        assert!(a.grid().history_size() > 30);
        // A line that fills the row exactly waits to wrap.
        round_trip(10, 3, b"0123456789");
        round_trip(10, 3, b"0123456789x");
    }

    #[test]
    fn a_full_history_stays_full() {
        // More lines than the history keeps (100): the snapshot has the kept ones only, and so does the copy.
        let mut text = Vec::new();
        for i in 0..300 {
            text.extend_from_slice(format!("\x1b[3{}mrow {i}\r\n", i % 8).as_bytes());
        }
        let (a, _) = round_trip(15, 6, &text);
        assert_eq!(a.grid().history_size(), 100);
    }

    #[test]
    fn wide_chars_and_marks() {
        round_trip(10, 3, "日本語のテキスト".as_bytes());
        round_trip(10, 3, "e\u{301}x 👍🏽 ok".as_bytes());
        // A wide char that does not fit at the end of a row goes to the next one.
        round_trip(5, 3, "abcd日x".as_bytes());
    }

    #[test]
    fn modes_and_the_cursor_style() {
        round_trip(
            20,
            4,
            b"\x1b[?1h\x1b=\x1b[?2004h\x1b[?1000h\x1b[?1006h\x1b[?25l\x1b[5 q\x1b[?1004hx",
        );
        round_trip(20, 4, b"\x1b[?7lno wrap here at all, really");
    }

    #[test]
    fn the_alternate_screen_and_what_is_under_it() {
        let (mut a, mut b) = round_trip(
            20,
            5,
            b"shell $ vim\r\n\x1b[?1049h\x1b[H\x1b[2J\x1b[44m~ vim screen\x1b[0m\x1b[3;5Hx",
        );
        // After the program ends, both show the same primary screen again.
        feed(&mut a, b"\x1b[?1049l");
        feed(&mut b, b"\x1b[?1049l");
        assert_eq!(state(&a), state(&b));
        assert!(state(&a).contains("'v'"));
    }

    #[test]
    fn osc8_links_and_changed_palette_colors() {
        let (a, b) = round_trip(
            30,
            3,
            b"see \x1b]8;id=pr;https://x.org/pr/42\x1b\\PR #42\x1b]8;;\x1b\\ \x1b]4;1;rgb:12/34/56\x1b\\",
        );
        assert_eq!(a.colors()[1], b.colors()[1]);
        assert!(b.colors()[1].is_some());
    }

    #[test]
    fn the_saved_cursor() {
        let (mut a, mut b) = round_trip(20, 5, b"abc\x1b[2;4H\x1b[31m\x1b7\x1b[0m\x1b[4;1Hz");
        feed(&mut a, b"\x1b8q");
        feed(&mut b, b"\x1b8q");
        assert_eq!(state(&a), state(&b));
    }
}
