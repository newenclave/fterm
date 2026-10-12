//! A remote session: the program runs somewhere else (the mux server), its output comes in as bytes, and keys and
//! sizes go back over a link.

use std::sync::{Arc, Mutex};

use fterm_term::osc::{OscEvent, PromptMark};
use fterm_term::session::{RemoteLink, Session, SessionOptions, TermEvent};
use fterm_term::size::GridSize;

#[derive(Default)]
struct FakeLink {
    writes: Mutex<Vec<Vec<u8>>>,
    sizes: Mutex<Vec<GridSize>>,
}

impl RemoteLink for FakeLink {
    fn write(&self, bytes: Vec<u8>) {
        self.writes.lock().unwrap().push(bytes);
    }
    fn resize(&self, size: GridSize) {
        self.sizes.lock().unwrap().push(size);
    }
}

fn remote() -> (
    Session,
    fterm_term::session::RemoteFeed,
    Arc<FakeLink>,
    Arc<Mutex<Vec<TermEvent>>>,
) {
    let link = Arc::new(FakeLink::default());
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let options = SessionOptions {
        program: Some("pwsh".into()),
        scrollback: 50,
        ..SessionOptions::default()
    };
    let (session, feed) = Session::remote(&options, GridSize::new(40, 5), link.clone(), move |e| {
        seen.lock().unwrap().push(e)
    });
    (session, feed, link, events)
}

#[test]
fn output_from_the_server_is_drawn_and_its_osc_events_come() {
    let (session, feed, _, events) = remote();
    assert_eq!(session.program(), "pwsh");
    assert_eq!(session.pid(), None);
    feed.output(b"\x1b]0;my title\x07\x1b]133;A\x07PS> \x1b]133;B\x07");
    feed.output(b"dir\r\n");
    assert!(session.screen_text().starts_with("PS> dir\n"));
    let events = events.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TermEvent::Title(t) if t == "my title"))
    );
    assert!(events.iter().any(|e| matches!(
        e,
        TermEvent::Osc(OscEvent::Prompt(PromptMark::CommandStart))
    )));
    // The cursor at 133;B, as for a local pane.
    assert!(events.iter().any(|e| matches!(
        e,
        TermEvent::Osc(OscEvent::InputStart { line: 0, column: 4 })
    )));
    assert!(events.iter().any(|e| matches!(e, TermEvent::Redraw)));
}

#[test]
fn keys_and_sizes_go_to_the_server() {
    let (session, _feed, link, _) = remote();
    session.write(b"ls\r".to_vec());
    session.resize(GridSize::new(60, 10), (8, 16));
    assert_eq!(*link.writes.lock().unwrap(), [b"ls\r".to_vec()]);
    assert_eq!(*link.sizes.lock().unwrap(), [GridSize::new(60, 10)]);
    assert_eq!(session.grid_size(), GridSize::new(60, 10));
    session.with_term(|term| {
        use fterm_term::alacritty_terminal::grid::Dimensions;
        assert_eq!((term.columns(), term.screen_lines()), (60, 10));
    });
}

#[test]
fn the_window_copy_does_not_answer_terminal_queries() {
    // The server's Term answers (it has the pty); an answer from the window too would come twice.
    let (_session, feed, link, _) = remote();
    feed.output(b"\x1b[c\x1b[6n");
    assert!(link.writes.lock().unwrap().is_empty());
}

#[test]
fn the_end_of_the_program_comes_as_an_exit() {
    let (_session, feed, _, events) = remote();
    feed.exit();
    assert!(matches!(
        events.lock().unwrap().last(),
        Some(TermEvent::Exit)
    ));
}
