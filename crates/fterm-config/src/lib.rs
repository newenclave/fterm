//! The fterm config. The user writes a Luau script (`fterm.lua`) that returns a table.
//! This crate has no window or GPU code, so it is easy to test.

pub mod colors;
pub mod keys;
pub mod l10n;
pub mod load;
pub mod profiles;
pub mod theme;
