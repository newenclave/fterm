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
