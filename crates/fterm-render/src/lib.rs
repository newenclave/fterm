//! GPU renderer for the fterm text grid.

pub mod atlas;
pub mod builtin;
pub mod color;
pub mod font;
pub mod frame;
pub mod overlay;
pub mod panes;
mod renderer;
pub mod tabbar;

pub use renderer::{FrameParts, Renderer};
