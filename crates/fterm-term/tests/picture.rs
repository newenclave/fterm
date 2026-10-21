//! Pictures in the output (OSC 1337): the feeder decodes them, marks their top-left cell, and the mark moves with
//! its cell (a full history, a clear, a reflow).

use fterm_term::alacritty_terminal::event::VoidListener;
use fterm_term::alacritty_terminal::grid::Dimensions;
use fterm_term::alacritty_terminal::term::{Config, Term};
use fterm_term::io_loop::{CellPx, Feeder};
use fterm_term::osc::OscEvent;
use fterm_term::picture::marks;
use fterm_term::size::GridSize;

fn term(columns: usize, rows: usize, scrollback: usize) -> Term<VoidListener> {
    Term::new(
        Config {
            scrolling_history: scrollback,
            ..Config::default()
        },
        &GridSize::new(columns, rows),
        VoidListener,
    )
}

fn feeder(make_room: bool) -> Feeder {
    let mut feeder = Feeder::default();
    feeder.cell = CellPx::new((10, 20));
    feeder.make_room = make_room;
    feeder
}

/// A `w`×`h` PNG as an OSC 1337 sequence.
fn sequence(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let mut out = b"\x1b]1337;File=name=eC5wbmc=;inline=1:".to_vec();
    out.extend(b64(&png.into_inner()).as_bytes());
    out.push(0x07);
    out
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |a, &b| (a << 8) | u32::from(b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            s.push(if i <= chunk.len() {
                T[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    s
}

fn feed(feeder: &mut Feeder, term: &mut Term<VoidListener>, bytes: &[u8]) -> Vec<OscEvent> {
    let mut osc = Vec::new();
    feeder.feed(term, bytes, &mut osc);
    osc
}

fn all_marks(term: &Term<VoidListener>) -> Vec<(u32, i32, usize)> {
    marks(term, i32::MIN..i32::MAX)
}

fn screen_text(term: &Term<VoidListener>) -> String {
    use fterm_term::alacritty_terminal::index::{Column, Line};
    let grid = term.grid();
    (0..grid.screen_lines() as i32)
        .map(|l| {
            (0..grid.columns())
                .map(|c| grid[Line(l)][Column(c)].c)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_picture_is_decoded_and_its_cell_is_marked() {
    let mut term = term(40, 10, 100);
    let mut feeder = feeder(true);
    let mut bytes = b"ab".to_vec();
    bytes.extend(sequence(50, 40));
    bytes.extend(b"cd");
    let osc = feed(&mut feeder, &mut term, &bytes);
    let [OscEvent::Picture(mark)] = &osc[..] else {
        panic!("{osc:?}");
    };
    // 50×40 px with 10×20 px cells: 5 columns, 2 rows.
    assert_eq!((mark.columns, mark.rows), (5, 2));
    assert_eq!((mark.picture.width, mark.picture.height), (50, 40));
    assert_eq!(all_marks(&term), [(mark.id, 0, 2)], "the cell after `ab`");
    // The parser did not get the body: no base64 on the screen. The text goes on right of the picture.
    assert_eq!(screen_text(&term).trim(), "ab\n       cd");
}

#[test]
fn without_room_the_cursor_stays_and_with_room_it_goes_below() {
    // ConPTY (Windows): the cursor stays, so it stays as ConPTY's one.
    let mut term_a = term(40, 10, 100);
    feed(&mut feeder(false), &mut term_a, &sequence(50, 60));
    let cursor = term_a.grid().cursor.point;
    assert_eq!((cursor.line.0, cursor.column.0), (0, 0));
    // No ConPTY: down to the last row of the picture (3 rows), right of it (5 columns).
    let mut term_b = term(40, 10, 100);
    feed(&mut feeder(true), &mut term_b, &sequence(50, 60));
    let cursor = term_b.grid().cursor.point;
    assert_eq!((cursor.line.0, cursor.column.0), (2, 5));
}

#[test]
fn the_mark_moves_with_its_line() {
    let mut term = term(40, 5, 100);
    let mut feeder = feeder(false);
    feed(&mut feeder, &mut term, b"top\r\n");
    feed(&mut feeder, &mut term, &sequence(10, 20));
    // The room that the program makes (as `ftermctl image` does): only a new line, so the cell stays.
    feed(&mut feeder, &mut term, b"\n");
    let id = all_marks(&term)[0].0;
    assert_eq!(all_marks(&term), [(id, 1, 0)]);
    // 10 more lines from row 2 of 5: the screen goes up by 8, and the line of the mark goes into the history.
    feed(&mut feeder, &mut term, &b"x\r\n".repeat(10));
    assert_eq!(all_marks(&term), [(id, 1 - 8, 0)]);
}

#[test]
fn the_mark_stays_on_its_line_when_the_history_is_full() {
    let mut term = term(40, 5, 10);
    let mut feeder = feeder(false);
    // Fill the history first.
    feed(&mut feeder, &mut term, &b"old\r\n".repeat(30));
    assert_eq!(term.grid().history_size(), 10);
    feed(&mut feeder, &mut term, b"here ");
    feed(&mut feeder, &mut term, &sequence(10, 20));
    let (id, line, _) = all_marks(&term)[0];
    // 3 more lines: the history size stays 10, but the line of the mark goes up by 3.
    feed(&mut feeder, &mut term, b"\r\na\r\nb\r\nc");
    assert_eq!(term.grid().history_size(), 10);
    assert_eq!(all_marks(&term), [(id, line - 3, 5)]);
    // Many more: its line leaves the history, and the mark goes with it.
    feed(&mut feeder, &mut term, &b"z\r\n".repeat(30));
    assert_eq!(all_marks(&term), []);
}

#[test]
fn a_clear_removes_the_mark() {
    let mut term = term(40, 5, 100);
    let mut feeder = feeder(false);
    feed(&mut feeder, &mut term, &sequence(10, 20));
    feed(&mut feeder, &mut term, b"\n");
    feed(&mut feeder, &mut term, &b"x\r\n".repeat(10));
    assert_eq!(all_marks(&term).len(), 1, "in the history now");
    // `cls` / `clear`: ESC[2J ESC[3J.
    feed(&mut feeder, &mut term, b"\x1b[H\x1b[2J\x1b[3J");
    assert_eq!(all_marks(&term), []);
}

#[test]
fn text_over_the_cell_removes_the_mark() {
    // With no room (a program that does not know about ConPTY), its next text goes on the cell of the picture.
    let mut term = term(40, 5, 100);
    let mut feeder = feeder(false);
    feed(&mut feeder, &mut term, &sequence(10, 20));
    assert_eq!(all_marks(&term).len(), 1);
    // A program writes on the cell of the picture: the picture is gone.
    feed(&mut feeder, &mut term, b"\rX");
    assert_eq!(all_marks(&term), []);
}

#[test]
fn the_mark_survives_a_reflow() {
    let mut term = term(40, 5, 100);
    let mut feeder = feeder(false);
    feed(
        &mut feeder,
        &mut term,
        b"0123456789012345678901234567890123456789",
    );
    feed(&mut feeder, &mut term, b"\r\nab");
    feed(&mut feeder, &mut term, &sequence(10, 20));
    feed(&mut feeder, &mut term, b"\r\nend");
    let id = all_marks(&term)[0].0;
    // Half the width: the long line takes two lines, so the mark goes one line down.
    term.resize(GridSize::new(20, 5));
    let after = all_marks(&term);
    assert_eq!(after.len(), 1);
    assert_eq!((after[0].0, after[0].2), (id, 2));
}

#[test]
fn a_bad_picture_gives_no_event_and_no_mark() {
    let mut term = term(40, 5, 100);
    let mut feeder = feeder(true);
    // Good base64, but not a picture.
    let osc = feed(
        &mut feeder,
        &mut term,
        b"\x1b]1337;File=inline=1:aGVsbG8gd29ybGQ=\x07ok",
    );
    assert_eq!(osc, []);
    assert_eq!(all_marks(&term), []);
    assert_eq!(screen_text(&term).trim(), "ok");
}

#[test]
fn two_pictures_get_two_ids() {
    let mut term = term(40, 10, 100);
    let mut feeder = feeder(true);
    feed(&mut feeder, &mut term, &sequence(10, 20));
    feed(&mut feeder, &mut term, b"\r\n");
    feed(&mut feeder, &mut term, &sequence(10, 20));
    let marks = all_marks(&term);
    assert_eq!(marks.len(), 2);
    assert_ne!(marks[0].0, marks[1].0);
}
