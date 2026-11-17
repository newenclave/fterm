//! UI parts that fterm draws as text into a pane with no program (the Review, forms): an input box, the looks of
//! text and their ANSI drawing, the keys, and text helpers. Pure: no window, no GPU, no UI words (the app gives
//! the words, so they follow the UI language).

pub mod input;
pub mod key;
pub mod look;
pub mod text;
