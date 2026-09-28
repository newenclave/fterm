//! Terminal session for fterm: it runs a shell in a pty and keeps the text grid.
//! This crate has no GPU or window code, so it is easy to test.

pub mod colors;
pub mod session;
pub mod size;

pub use alacritty_terminal;
