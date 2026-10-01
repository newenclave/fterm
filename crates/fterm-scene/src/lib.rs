//! A Braille canvas (Phase 11): every cell is a 2×4 grid of dots, so a 80×24 pane is a 160×96 pixel screen.
//! The idea comes from tank_rs (`braille_canvas.rs`). Dots are `x` (0 = left) and `y` (0 = top) in pixels.

pub mod canvas;
pub mod ops;

pub use canvas::{Canvas, Cell, Rgb};
pub use ops::{Op, apply, parse_ops};
