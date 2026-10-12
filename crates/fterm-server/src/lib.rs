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
