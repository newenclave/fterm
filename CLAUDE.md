# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

fterm is a GPU terminal (Rust, wgpu + winit) for Windows, Linux, and macOS, with a local JSON-RPC API,
an MCP server for agents, Braille "scenes", and an AI panel. Note: `docs/CLAUDE.md` is user documentation
about the Claude Code tab dots and hooks, not instructions for you.

## Commands

```sh
cargo run                                   # debug build, opens a window with your shell
cargo fmt --all --check                     # CI runs these three on Windows, Linux, macOS
cargo clippy --all-targets -- -D warnings
cargo test
cargo test -p fterm-term                    # one crate
cargo test -p fterm-term --test session     # one integration test file (crates/<crate>/tests/*.rs)
cargo test -p fterm-mux layout::            # tests whose path matches a filter
```

- Logs: `RUST_LOG=debug cargo run` (PowerShell: `$env:RUST_LOG="debug"; cargo run`).
- GPU backend: `WGPU_BACKEND=vulkan|gl cargo run` (Windows default is DX12).
- Debug-only start helpers: `FTERM_TABS=3`, `FTERM_SPLITS="right,down"`, `FTERM_RUN="<command>"`
  (typed into the first tab). Manual-check samples are in `docs/samples/`.
- Release archive: `cargo build --release -p fterm -p ftermctl` then
  `python scripts/package.py --bin-dir target/release --out dist`.
- Linux build deps: `libxkbcommon-dev libwayland-dev libx11-dev`.

## Rules of the repo

- Every user-visible change goes into `## [Unreleased]` in `CHANGELOG.md` in the same branch. Release notes
  are cut from it (`scripts/package.py --notes vX.Y.Z`); see `docs/BUILD.md` for the release steps.
- `unsafe_code = "deny"` for the workspace. The only exceptions are the ConPTY code
  (`fterm-term/src/conpty`) and one spot in `io_loop.rs`; keep unsafe there.
- Commits use conventional-commit prefixes with a scope and short, plain wording,
  e.g. `fix(conpty): stop ConPTY from drawing over the reflow on a resize`.
- Docs (`docs/*.md`, `README.md`, `CHANGELOG.md`, `assets/agents/GUIDE.md`) are written in very simple English:
  short sentences, common words. Match that style when you edit them.
- Tests must pass on all three CI systems: no hard-coded Windows paths, and tests that need system fonts
  (emoji, CJK) skip themselves when the font is missing. Font tests use `Fonts::embedded_only`.

## Architecture

Read `docs/ARCHITECTURE.md` for detail. The big picture:

| Crate | Role |
|---|---|
| `fterm-term` | pty session (own ConPTY on Windows), our pty read/write loop (`io_loop.rs`), OSC scanner, sticky selection, copy mode. No GPU, so it is tested with real processes. |
| `fterm-mux` | Pure model: tabs, pane tree (`Layout` of `Pane`/`Split`), rects, neighbors, zoom. |
| `fterm-config` | Runs `fterm.lua` in sandboxed Luau (`mlua`); settings, colors, profiles, keymap. `sample.lua` is the default config. |
| `fterm-render` | wgpu renderer: glyph atlases (mask + color), builtin box/Braille glyphs, tab bar, dock, toasts, overlays. One draw call per frame. |
| `fterm-history` | Command/folder history in JSON-lines files shared by many windows (append-only writes). |
| `fterm-scene` | Braille canvas: draw ops and plots for scene panes. |
| `fterm-api` | JSON-RPC 2.0 over a local socket (named pipe / unix socket); server, client, discovery (`instances/<pid>.json`), the agent guide. |
| `fterm-ai` | Streaming clients for the AI panel (Anthropic, OpenAI-like, Ollama), SSE parsing. |
| `fterm` | The app: winit event loop, wires everything. `app.rs` is the core; `app/*_calls.rs` handle API/AI/session calls. |
| `ftermctl` | Console CLI and the stdio MCP server (`ftermctl mcp`) that talk to a running window over the API socket. |

Key flows that span files:

- **Threads:** the pty thread parses output into the alacritty `Term` (behind a lock) and sends `UserEvent`s
  through an `EventLoopProxy`; the window thread draws only on change. Events carry a `PaneId`
  (`UserEvent::Term(PaneId, TermEvent)`). Use `Session::with_term` / `with_term_mut`; change the selection
  only through `StickySelection::set`.
- **OSC sequences** alacritty drops (7, 9/99/777, 133, 633;E, `777;fterm-agent;<state>;<text>`, and
  `777;fterm-tab;color;<color>`) are pulled out by `fterm-term/src/osc.rs` before the parser and arrive as
  `TermEvent::Osc`; the app handles them in `app.rs`, `agent.rs`, and `notify.rs`.
- **Lua config functions** never change the app directly: `fterm.spawn`, `fterm.send_text`, … queue
  `ApiCall`s that the app runs after the function returns. The config hot-reloads; a broken file keeps the old one.
- **Adding an API method** touches several places: the method list in `fterm/src/api.rs` (`METHODS`),
  the handler in `fterm/src/app/api_calls.rs`, the CLI in `ftermctl/src/cli.rs` (+ output in `main.rs`),
  the MCP tool in `ftermctl/src/mcp.rs`, and the docs `docs/API.md`, `docs/MCP.md`, and
  `assets/agents/GUIDE.md` (the guide shipped to agents and installed as a Claude Code skill).
- **Frame building** (`fterm-render/src/frame.rs`) turns cells into quads with no GPU code, so it is unit-tested;
  `renderer.rs` composes `FrameParts` (tab bar, panes, dock, toasts, palette, message box).
