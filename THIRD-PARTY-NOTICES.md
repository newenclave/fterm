# Third-party notices

fterm (MIT, see LICENSE) has parts of these projects in it:

## JetBrains Mono
The fonts in `assets/fonts` are JetBrains Mono, Copyright 2020 The JetBrains Mono Project Authors,
under the SIL Open Font License 1.1. The full license is in `assets/fonts/OFL.txt` (in a release
archive: `OFL.txt`).

## alacritty_terminal
fterm uses the `alacritty_terminal` crate (Apache-2.0), Copyright The Alacritty Project
Contributors (https://github.com/alacritty/alacritty).

Two parts of it are copied into fterm and changed, under the Apache License 2.0
(https://www.apache.org/licenses/LICENSE-2.0):
- `crates/fterm-term/src/io_loop.rs`: the pty read loop (changed to read OSC sequences first).
- `crates/fterm-term/src/conpty/`: the Windows pseudo console (changed to set the ConPTY flags).

## Rust crates
fterm is built from many crates of crates.io, each under its own license (mostly MIT or Apache-2.0).
`cargo tree` lists them, and `cargo about` or `cargo license` can list their licenses.
