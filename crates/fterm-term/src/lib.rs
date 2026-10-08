//! Terminal session for fterm: it runs a shell in a pty and keeps the text grid.
//! This crate has no GPU or window code, so it is easy to test.

pub mod colors;
#[cfg(windows)]
pub mod conpty;
pub mod copy_mode;
pub mod harmonize;
pub mod input;
pub mod io_loop;
pub mod links;
pub mod osc;
pub mod process;
pub mod record;
pub mod select;
pub mod session;
pub mod shell;
pub mod size;
pub mod styled;

pub use alacritty_terminal;
