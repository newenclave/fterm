//! A real server in this process, real programs, and a client: spawn, output, detach, attach, close, the end.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fterm_server::client::{Handlers, MuxClient};
use fterm_server::proto::Spawn;
use fterm_server::server::Server;
use fterm_term::alacritty_terminal::event::VoidListener;
use fterm_term::alacritty_terminal::grid::Dimensions;
use fterm_term::alacritty_terminal::index::{Column, Line};
use fterm_term::alacritty_terminal::term::{Config, Term};
use fterm_term::alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use fterm_term::size::GridSize;

fn timeout() -> Duration {
    if std::env::var_os("CI").is_some() {
        Duration::from_secs(90)
    } else {
        Duration::from_secs(40)
    }
}

/// A server with its own socket name, and a client.
fn start(test: &str) -> (Server, Arc<MuxClient>) {
    let name =
        fterm_api::transport::named_socket(&format!("mux-test-{}-{test}", std::process::id()));
    let server = Server::start(&name).expect("server");
    let client = MuxClient::connect(&name, || {}).expect("client");
    (server, client)
}

fn shell() -> Spawn {
    let (program, args) = if cfg!(windows) {
        ("cmd.exe", vec!["/q".to_owned()])
    } else {
        ("sh", Vec::new())
    };
    Spawn {
        program: Some(program.to_owned()),
        args,
        scrollback: 100,
        ..Spawn::default()
    }
}

/// Collects the output of a pane.
fn collect(client: &MuxClient, pane: u64) -> (Arc<Mutex<Vec<u8>>>, Arc<Mutex<bool>>) {
    let out = Arc::new(Mutex::new(Vec::new()));
    let ended = Arc::new(Mutex::new(false));
    let (o, e) = (out.clone(), ended.clone());
    client.set_handlers(
        pane,
        Handlers {
            output: Box::new(move |b| o.lock().unwrap().extend_from_slice(b)),
            exit: Box::new(move || *e.lock().unwrap() = true),
        },
    );
    (out, ended)
}

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout();
    while !ok() {
        assert!(Instant::now() < deadline, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn text(term: &Term<VoidListener>) -> String {
    let grid = term.grid();
    let mut s = String::new();
    let mut line = term.topmost_line();
    while line <= term.bottommost_line() {
        for c in 0..term.columns() {
            s.push(grid[line][Column(c)].c);
        }
        s.push('\n');
        line += Line(1);
    }
    s
}

#[test]
fn a_pane_lives_on_without_a_window_and_comes_back() {
    let (server, client) = start("lives");
    let size = GridSize::new(60, 10);
    let (pane, program) = client.spawn(shell(), size).expect("spawn");
    assert!(!program.is_empty());
    let (out, _) = collect(&client, pane);
    client.write(pane, b"echo mux-one\r");
    wait("the first output", || {
        String::from_utf8_lossy(&out.lock().unwrap()).contains("mux-one")
    });

    // The window goes; the program goes on and writes more.
    client.detach(pane);
    client.write(pane, b"echo mux-two\r");
    std::thread::sleep(Duration::from_millis(800));
    let list = client.list().expect("list");
    assert_eq!(list.len(), 1);
    assert!(!list[0].attached);

    // A window takes it again: the snapshot has all of it.
    let (snapshot, _meta, _, server_size) = client.attach(pane).expect("attach");
    assert_eq!(server_size, size);
    let mut term = Term::new(
        Config {
            scrolling_history: 100,
            ..Config::default()
        },
        &server_size,
        VoidListener,
    );
    Processor::<StdSyncHandler>::new().advance(&mut term, &snapshot);
    let shown = text(&term);
    assert!(
        shown.contains("mux-one") && shown.contains("mux-two"),
        "{shown}"
    );
    assert!(client.list().unwrap()[0].attached);

    // Closing the pane ends its program.
    client.close(pane);
    wait("the pane to go", || server.pane_count() == 0);
}

#[test]
fn the_end_of_a_program_comes_to_the_window() {
    let (server, client) = start("end");
    let spawn = Spawn {
        program: Some(if cfg!(windows) { "cmd.exe" } else { "sh" }.to_owned()),
        args: if cfg!(windows) {
            vec!["/c".into(), "echo bye".into()]
        } else {
            vec!["-c".into(), "echo bye".into()]
        },
        scrollback: 10,
        ..Spawn::default()
    };
    let (pane, _) = client.spawn(spawn, GridSize::new(40, 5)).expect("spawn");
    let (out, ended) = collect(&client, pane);
    wait("the end", || *ended.lock().unwrap());
    assert!(String::from_utf8_lossy(&out.lock().unwrap()).contains("bye"));
    wait("the pane to go", || server.pane_count() == 0);
}

#[test]
fn the_layout_is_kept_for_the_next_window() {
    let (_server, client) = start("layout");
    assert_eq!(client.layout().unwrap(), None);
    client.set_layout(serde_json::json!({"tabs": [1, 2]}));
    assert_eq!(
        client.layout().unwrap(),
        Some(serde_json::json!({"tabs": [1, 2]}))
    );
}

#[test]
fn a_pane_with_no_folder_starts_in_the_home_folder() {
    let (server, client) = start("home");
    let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).expect("home");
    let (pane, program) = client.spawn(shell(), GridSize::new(200, 5)).expect("spawn");
    assert!(
        program.contains("cmd") || program.contains("sh"),
        "the real program: {program}"
    );
    let (out, _) = collect(&client, pane);
    client.write(pane, if cfg!(windows) { b"cd\r" } else { b"pwd\r" });
    wait("the folder", || {
        String::from_utf8_lossy(&out.lock().unwrap()).contains(&home)
    });
    client.close(pane);
    wait("the pane to go", || server.pane_count() == 0);
}

#[test]
fn stop_ends_all_panes_and_the_server() {
    let (server, client) = start("stop");
    for _ in 0..2 {
        client.spawn(shell(), GridSize::new(40, 5)).expect("spawn");
    }
    assert_eq!(server.pane_count(), 2);
    assert_eq!(client.stop(), Ok(2));
    assert_eq!(server.pane_count(), 0);
    // `run_until_idle` returns at once now, also for a server that never ends by itself.
    let started = Instant::now();
    server.run_until_idle(None);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_server_with_nothing_to_do_ends() {
    let name = fterm_api::transport::named_socket(&format!("mux-test-{}-idle", std::process::id()));
    let server = Server::start(&name).expect("server");
    let started = Instant::now();
    server.run_until_idle(Some(Duration::from_millis(300)));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_request_for_a_pane_that_is_not_there_fails() {
    let (_server, client) = start("missing");
    assert!(client.attach(12345).is_err());
}
