//! `ftermctl mux proxy` as the connect command of a remote: the window side speaks to the server through the
//! stdin and stdout of the proxy (over SSH on a real remote; here a local process).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fterm_server::client::{Handlers, MuxClient};
use fterm_server::proto::Spawn;
use fterm_server::server::Server;
use fterm_term::size::GridSize;

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !ok() {
        assert!(Instant::now() < deadline, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_pane_through_the_proxy() {
    // The server of "the other machine" (this process), with its own socket name.
    let base = format!("mux-proxy-test-{}", std::process::id());
    let server = Server::start(&fterm_api::transport::named_socket(&base)).expect("server");

    let command = vec![
        env!("CARGO_BIN_EXE_ftermctl").to_owned(),
        "mux".to_owned(),
        "proxy".to_owned(),
    ];
    let env = vec![("FTERM_MUX_SOCKET".to_owned(), base)];
    let client = MuxClient::connect_command(&command, &env, || {}).expect("proxy");
    assert_eq!(
        client.hello(Duration::from_secs(10)),
        Ok(fterm_server::proto::VERSION)
    );

    let (program, args) = if cfg!(windows) {
        ("cmd.exe", vec!["/q".to_owned()])
    } else {
        ("sh", Vec::new())
    };
    let spawn = Spawn {
        program: Some(program.to_owned()),
        args,
        scrollback: 50,
        ..Spawn::default()
    };
    let (pane, _) = client.spawn(spawn, GridSize::new(50, 8)).expect("spawn");
    let out = Arc::new(Mutex::new(Vec::new()));
    let into = out.clone();
    client.set_handlers(
        pane,
        Handlers {
            output: Box::new(move |b| into.lock().unwrap().extend_from_slice(b)),
            exit: Box::new(|| {}),
        },
    );
    client.write(pane, b"echo through-proxy\r");
    wait("the output", || {
        String::from_utf8_lossy(&out.lock().unwrap()).contains("through-proxy")
    });
    client.close(pane);
    wait("the pane to go", || server.pane_count() == 0);
}

#[test]
fn a_command_that_is_not_a_proxy_gives_its_error() {
    // The other side prints an error and ends (like ssh with a bad key, or no ftermctl there).
    let command: Vec<String> = if cfg!(windows) {
        ["cmd.exe", "/c", "echo ftermctl: not found 1>&2"]
            .map(str::to_owned)
            .to_vec()
    } else {
        ["sh", "-c", "echo 'ftermctl: not found' >&2"]
            .map(str::to_owned)
            .to_vec()
    };
    let client = MuxClient::connect_command(&command, &[], || {}).expect("it starts");
    let err = client.hello(Duration::from_secs(10)).unwrap_err();
    assert!(err.contains("ftermctl: not found"), "{err}");
}

#[test]
fn a_connection_that_hangs_can_be_cancelled() {
    // Like ssh to a wrong host: the command says nothing for a long time.
    let command: Vec<String> = if cfg!(windows) {
        ["powershell", "-NoProfile", "-Command", "Start-Sleep 60"]
            .map(str::to_owned)
            .to_vec()
    } else {
        ["sleep", "60"].map(str::to_owned).to_vec()
    };
    let client = MuxClient::connect_command(&command, &[], || {}).expect("it starts");
    let waiting = std::sync::Arc::clone(&client);
    let started = Instant::now();
    let hello = std::thread::spawn(move || waiting.hello(Duration::from_secs(30)));
    std::thread::sleep(Duration::from_millis(500));
    client.kill();
    assert!(hello.join().unwrap().is_err());
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the cancel ends the wait at once: {:?}",
        started.elapsed()
    );
}
