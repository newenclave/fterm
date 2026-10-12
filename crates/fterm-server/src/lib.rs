//! The mux server: the programs of its panes live in this process, so they go on running when the window
//! closes, and a window attaches to them again. The window keeps its own copy of each Term: on attach it gets a
//! snapshot (escape bytes), then the raw output of the program, and parses it as for a local pane.

pub mod client;
pub mod frame;
pub mod proto;
pub mod server;

/// The base name of the socket of the mux server of the current user (see `fterm_api::transport::named_socket`).
pub const SOCKET: &str = "mux";
