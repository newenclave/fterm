//! The mux server: the programs of its panes live in this process, so they go on running when the window
//! closes, and a window attaches to them again. The window keeps its own copy of each Term: on attach it gets a
//! snapshot (escape bytes), then the raw output of the program, and parses it as for a local pane.

pub mod client;
pub mod frame;
pub mod proto;
pub mod server;

/// The base name of the socket of the mux server of the current user (see `fterm_api::transport::named_socket`).
pub const SOCKET: &str = "mux";

/// The socket of the mux server: `FTERM_MUX_SOCKET` (a base name, for tests and a second server) or `mux`.
pub fn socket_name() -> String {
    let base = std::env::var("FTERM_MUX_SOCKET")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| SOCKET.to_owned());
    fterm_api::transport::named_socket(&base)
}

/// Runs the server (`fterm --mux-server`) until it has had no panes and no windows for a minute.
pub fn run() -> std::io::Result<()> {
    let name = socket_name();
    tracing::info!(%name, "mux server starts");
    let server = server::Server::start(&name)?;
    server.run_until_idle(std::time::Duration::from_secs(60));
    Ok(())
}

/// Starts `exe args...` in the background: no console and no window, and it does not end with this process (or
/// with the SSH session that started it). For the server (`fterm --mux-server`, `ftermctl mux serve`).
pub fn spawn_detached(exe: &std::path::Path, args: &[&str]) -> std::io::Result<()> {
    let mut command = std::process::Command::new(exe);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NO_WINDOW: no console, so it does not end with a console. Not
        // CREATE_NEW_PROCESS_GROUP: that turns Ctrl+C off for the server and every program in its panes.
        command.creation_flags(0x0000_0008 | 0x0800_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: the end of the terminal or the SSH session that started it does not end it.
        command.process_group(0);
    }
    command.spawn().map(|_| ())
}

/// `ftermctl mux proxy`: stdin → the mux server of this machine → stdout (the server is started when it is not
/// there, with `start`). It ends when stdin or the server ends; the server and its panes stay.
pub fn proxy(start: impl Fn() -> std::io::Result<()>) -> std::io::Result<()> {
    use interprocess::local_socket::prelude::*;
    use std::io::{Read, Write};

    let name = socket_name();
    let stream = match fterm_api::transport::connect(&name) {
        Ok(stream) => stream,
        Err(_) => {
            start()?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                match fterm_api::transport::connect(&name) {
                    Ok(stream) => break stream,
                    Err(_) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    };
    let (mut recv, mut send) = stream.split();
    // stdin → the server.
    std::thread::Builder::new()
        .name("mux proxy in".into())
        .spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if send
                            .write_all(&buf[..n])
                            .and_then(|()| send.flush())
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            // The window went (SSH ended): the proxy ends; the server keeps the panes.
            std::process::exit(0);
        })?;
    // The server → stdout.
    let mut stdout = std::io::stdout().lock();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match recv.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                stdout.write_all(&buf[..n])?;
                stdout.flush()?;
            }
            Err(err) => return Err(err),
        }
    }
}
