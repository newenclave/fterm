//! These tests start real processes in a real pty (ConPTY on Windows).

use std::sync::mpsc;
use std::time::Duration;

use fterm_term::alacritty_terminal::grid::Dimensions;
use fterm_term::session::{Session, SessionOptions, TermEvent};
use fterm_term::size::GridSize;

const TIMEOUT: Duration = Duration::from_secs(20);

fn spawn(options: SessionOptions) -> (Session, mpsc::Receiver<TermEvent>) {
    let (tx, rx) = mpsc::channel();
    let session = Session::spawn(options, GridSize::new(80, 24), (8, 16), move |event| {
        let _ = tx.send(event);
    })
    .expect("cannot start the session");
    (session, rx)
}

fn wait_for_exit(rx: &mpsc::Receiver<TermEvent>) {
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(TermEvent::Exit) => return,
            Ok(_) => {}
            Err(err) => panic!("no Exit event: {err}"),
        }
    }
}

fn echo_command() -> SessionOptions {
    if cfg!(windows) {
        SessionOptions::command("cmd.exe", ["/c", "echo fterm-ok"])
    } else {
        SessionOptions::command("sh", ["-c", "echo fterm-ok"])
    }
}

#[test]
fn output_of_a_command_is_in_the_grid() {
    let (session, rx) = spawn(echo_command());
    wait_for_exit(&rx);
    let text = session.screen_text();
    assert!(text.contains("fterm-ok"), "screen text was:\n{text}");
}

#[test]
fn shell_ends_after_exit_command() {
    let (session, rx) = spawn(SessionOptions::default());
    session.write(b"exit\r".to_vec());
    wait_for_exit(&rx);
}

#[test]
fn resize_changes_the_grid_size() {
    let (session, _rx) = spawn(SessionOptions::default());
    session.resize(GridSize::new(100, 30), (8, 16));
    assert_eq!(session.grid_size(), GridSize::new(100, 30));
    let (columns, rows) = session.with_term(|term| (term.columns(), term.screen_lines()));
    assert_eq!((columns, rows), (100, 30));
    session.write(b"exit\r".to_vec());
}
