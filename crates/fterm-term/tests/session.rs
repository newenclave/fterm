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

fn wait_for_exit(session: &Session, rx: &mpsc::Receiver<TermEvent>) {
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(TermEvent::Exit) => return,
            Ok(_) => {}
            Err(err) => panic!(
                "no Exit event: {err}
screen:
{}",
                session.screen_text()
            ),
        }
    }
}

fn echo_command() -> SessionOptions {
    if cfg!(windows) {
        // The short wait (ping) is needed: when a process ends at once, alacritty reads the pty
        // only one more time, and ConPTY can send the output later. Then the output is lost.
        // Our own pty loop (roadmap, Phase 3b) will read until the end of the stream.
        SessionOptions::command(
            "cmd.exe",
            ["/c", "echo fterm-ok & ping -n 2 127.0.0.1 >nul"],
        )
    } else {
        SessionOptions::command("sh", ["-c", "echo fterm-ok"])
    }
}

#[test]
fn output_of_a_command_is_in_the_grid() {
    let (session, rx) = spawn(echo_command());
    wait_for_exit(&session, &rx);
    let text = session.screen_text();
    assert!(text.contains("fterm-ok"), "screen text was:\n{text}");
}

#[test]
fn shell_ends_after_exit_command() {
    let (session, rx) = spawn(SessionOptions::default());
    session.write(b"exit\r".to_vec());
    wait_for_exit(&session, &rx);
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

#[test]
fn session_knows_its_shell_pid_and_program() {
    let options = if cfg!(windows) {
        SessionOptions::command("cmd.exe", ["/k"])
    } else {
        SessionOptions::command("sh", Vec::<String>::new())
    };
    let (session, _rx) = spawn(options);
    let pid = session.pid().expect("no pid");
    assert!(pid > 0);
    let expected = if cfg!(windows) { "cmd" } else { "sh" };
    assert_eq!(session.program(), expected);

    // Run a program in the shell: it shows up as a child of the shell.
    let (command, name) = if cfg!(windows) {
        ("ping -n 5 127.0.0.1\r", "ping")
    } else {
        ("sleep 5\r", "sleep")
    };
    session.write(command.as_bytes().to_vec());
    std::thread::sleep(Duration::from_millis(1500));
    let children = fterm_term::process::running_children(pid);
    assert!(
        children.iter().any(|c| c.to_lowercase().starts_with(name)),
        "{children:?}"
    );
    session.write(b"\x03exit\r".to_vec());
}
