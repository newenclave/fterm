//! The local API of fterm: JSON-RPC 2.0 (one JSON object per line) over a local socket.
//!
//! - Windows: a named pipe that only the current user can open.
//! - Linux and macOS: a unix socket in a folder that only the current user can read.
//!
//! The app runs a [`server::Server`]; `fterm cli`, `fterm mcp`, and scripts use a [`client::Client`].
//! Each fterm window writes a small file in [`discovery::instances_dir`], and every pane gets
//! `FTERM_SOCKET`, so a client finds its window.

pub mod client;
pub mod discovery;
pub mod protocol;
pub mod server;
pub mod transport;

pub use protocol::{API_VERSION, Notification, Request, Response, RpcError};
