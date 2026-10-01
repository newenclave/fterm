//! The model of tabs and panes. It has no GPU, window, or pty code, so it is easy to test.

pub mod layout;
pub mod mux;

pub use layout::{Direction, Divider, Edge, Layout, Rect};
pub use mux::{Closed, Mux, PaneId, TabId};
