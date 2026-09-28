//! GPU renderer for the fterm text grid.

pub mod atlas;
pub mod builtin;
pub mod color;
pub mod font;
pub mod frame;
mod renderer;

pub use renderer::Renderer;
