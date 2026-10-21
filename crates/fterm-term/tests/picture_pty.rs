//! A picture through a real pty (ConPTY on Windows): the mark of the picture stays on its cell when the program
//! makes room, writes more lines, and the pane is resized.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use fterm_term::osc::OscEvent;
use fterm_term::picture::marks;
use fterm_term::session::{Session, SessionOptions, TermEvent};
use fterm_term::size::GridSize;

/// A 1×1 PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";

fn timeout() -> Duration {
    if std::env::var_os("CI").is_some() {
        Duration::from_secs(90)
    } else {
        Duration::from_secs(40)
    }
}

#[test]
fn the_mark_of_a_picture_survives_the_pty() {
    // The picture takes 10×3 cells, as `ftermctl image` writes it: its mark is an OSC 8 link on a space (ConPTY
    // keeps such links when it paints the lines again after a resize), then three new lines (room), then more text.
    let args = "width=10;height=3;preserveAspectRatio=0;inline=1;ftermRoom=1;ftermMark=2147483999";
    let link = "8;id=2147483999;fterm-picture:2147483999";
    let options = if cfg!(windows) {
        let script = format!(
            "$e=[char]27; $b=[char]7; Write-Host 'before'; \
             Write-Host -NoNewline \"$e]1337;File={args}:{PNG}$b$e]{link}$b $e]8;;$b\"; \
             Write-Host \"`n`n\"; Write-Host 'after'; Start-Sleep -Seconds 3; Write-Host 'end'"
        );
        SessionOptions::command("powershell.exe", ["-NoProfile", "-Command", &script])
    } else {
        let script = format!(
            "echo before; printf '\\033]1337;File={args}:{PNG}\\007\\033]{link}\\007 \\033]8;;\\007'; \
             echo; echo after; sleep 3; echo end"
        );
        SessionOptions::command("sh", ["-c", &script])
    };
    let (tx, rx) = mpsc::channel();
    let session = Session::spawn(options, GridSize::new(80, 24), (8, 16), move |e| {
        let _ = tx.send(e);
    })
    .expect("session");
    let deadline = Instant::now() + timeout();
    let mut picture = None;
    let mut resized = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left.min(Duration::from_millis(200))) {
            Ok(TermEvent::Osc(OscEvent::Picture(mark))) => picture = Some(mark),
            Ok(TermEvent::Exit) => break,
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(err) => panic!("{err}"),
        }
        // While it sleeps: a narrower and then a wider pane.
        if !resized && session.screen_text().contains("after") {
            resized = true;
            session.resize(GridSize::new(50, 20), (8, 16));
            std::thread::sleep(Duration::from_millis(500));
            session.resize(GridSize::new(90, 24), (8, 16));
        }
        assert!(
            Instant::now() < deadline,
            "too slow:\n{}",
            session.screen_text()
        );
    }
    let mark = picture.expect("a Picture event");
    assert_eq!((mark.columns, mark.rows), (10, 3));
    let found = session.with_term(|t| marks(t, i32::MIN..i32::MAX));
    let screen = session.screen_text();
    assert_eq!(found.len(), 1, "the mark is still there:\n{screen}");
    assert_eq!(found[0].0, mark.id);
    assert!(
        screen.contains("after") && screen.contains("end"),
        "{screen}"
    );
}
