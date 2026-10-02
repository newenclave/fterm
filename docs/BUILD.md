# How to build fterm

## What you need
- Rust. The file `rust-toolchain.toml` picks the version. `rustup` installs it for you.
- Windows 10 or 11 (DX12), Linux (Vulkan or OpenGL), or macOS (Metal).
- On Linux, install these packages first:
  `sudo apt-get install libxkbcommon-dev libwayland-dev libx11-dev`

## Run
```sh
cargo run
```
A terminal window opens with your shell:
- Windows: `pwsh.exe` when it is installed, else `powershell.exe`.
- Linux and macOS: the shell from `$SHELL`.

Type `exit` or close the window to stop the app.

## Logs
The `RUST_LOG` variable sets the log level. The default is `info`.

```sh
RUST_LOG=debug cargo run          # bash
$env:RUST_LOG="debug"; cargo run  # PowerShell
```

The log shows the GPU and the backend, for example `backend=Dx12`.

## Choose a GPU backend
On Windows, fterm uses DX12. You can choose another backend with `WGPU_BACKEND`:

```sh
WGPU_BACKEND=vulkan cargo run
WGPU_BACKEND=gl cargo run
```

## Test scripts (debug builds only)
`FTERM_TABS=3` opens 3 tabs at start. `FTERM_SPLITS="right,down"` splits the first tab at start.
`FTERM_RUN` types a command into the shell of the first tab when fterm starts:

```powershell
$env:FTERM_RUN = "Get-Content -Encoding utf8 docs\samples\unicode-test.txt"; cargo run
```

Samples for a manual check are in `docs/samples/`:
- `unicode-test.txt`: many scripts, emoji, box lines, blocks, Braille;
- `braille-wave.ps1`: a sine wave made of Braille chars.

## Checks
Run these before you commit. CI runs them too, on Windows, Linux, and macOS.

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Release build
```sh
cargo build --release
```
The file is `target/release/fterm` (`fterm.exe` on Windows).
In a release build on Windows, there is no console window, so you do not see logs.

## A portable archive
```sh
cargo build --release -p fterm -p ftermctl
python scripts/package.py --bin-dir target/release --out dist
```
`dist/` gets `fterm-<version>-<system>-<arch>.zip` (Windows) or `.tar.gz` (Linux, macOS), and a `.sha256` file.
In the archive: `fterm`, `ftermctl`, a portable `fterm.lua` (`data_dir = "data"`), the README, `CHANGELOG.md`,
and the licenses (on Linux also `fterm.desktop` and `fterm.png`).

## Make a release
1. Set the new version in `Cargo.toml` (`[workspace.package] version`).
2. In [CHANGELOG.md](../CHANGELOG.md), make `## [Unreleased]` the new version with the date
   (`## [0.2.0] - 2026-11-01`), put a new empty `## [Unreleased]` above it, and fix the links at the end.
   `python scripts/package.py --notes v0.2.0` shows the text that the release gets. Push it.
3. Push a tag with the same version: `git tag v0.2.0 && git push origin v0.2.0`.
4. The workflow `Release` builds the archives on Windows, Linux, and macOS and makes a GitHub Release with them.
   The text of the release is the part of CHANGELOG.md for that version.
   A tag that is not the version of `Cargo.toml`, or a version with no part in CHANGELOG.md, stops it.

Every change that people see goes into `## [Unreleased]` in CHANGELOG.md, in the same branch.
