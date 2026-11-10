# fterm architecture

## Crates

| Crate | What it does | GPU? |
|---|---|---|
| `fterm-term` | Runs the shell in a pty. Keeps the text grid. Colors. Grid size. Links. Pictures (OSC 1337). Remote sessions and snapshots for the mux server. | No |
| `fterm-mux` | Workspaces, tabs, the tree of panes in each tab, overlays, the scratch shell, and companions. A pure model. | No |
| `fterm-config` | The Luau config: settings, themes, languages (l10n), profiles, key bindings, Lua functions. | No |
| `fterm-render` | Font, glyph atlas, tab bar, and drawing with wgpu. | Yes |
| `fterm-history` | The history of commands and folders (JSON lines files). | No |
| `fterm-scene` | Braille scenes: a canvas of dots, draw commands, and charts. | No |
| `fterm-api` | The local API: JSON-RPC over a socket, the server, the client, finding windows, the guide for agents. | No |
| `fterm-server` | The mux server (panes that live on) and its client: frames and messages on a socket. | No |
| `fterm-ai` | The AI clients (Anthropic, OpenAI-like) with streamed answers. | No |
| `fterm` | The app: window, keys, events. It connects the other crates. | Yes |
| `ftermctl` | A console program: the CLI for the API, the MCP server (`ftermctl mcp`), and the mux server on other machines (`ftermctl mux serve\|proxy`). | No |

`fterm-term` has no GPU and no window code. So we can test it with real
processes and real escape codes.

## The pty loop

`fterm-term/src/io_loop.rs` is our own pty read and write loop (a port of the alacritty loop).
- The bytes from the pty go through `osc::Scanner` first, then to the alacritty parser.
  The scanner finds the sequences that alacritty drops: OSC 7 (folder), OSC 9 / 99 / 777 (notifications),
  OSC 133 (shell integration), `OSC 777;fterm-agent;<state>;<text>` (agent state), and
  `OSC 777;fterm-tab;color;<#rrggbb|none>` (tab color). They come as `TermEvent::Osc`.
- ConPTY passes all these sequences (tested on Windows 11 with a real ConPTY).
- After the child process ends, the loop reads until the output is quiet (150 ms, at most 2 s),
  and it looks for the exit event after every wait (at most 200 ms). So a command that ends at once
  does not lose its output, and its exit is not missed.

## Threads

```
 pty thread (alacritty_terminal)            window thread (winit)
 ───────────────────────────────            ─────────────────────
 read shell output
 parse it, change the grid   ──Redraw──▶    request_redraw()
 (Term is behind a lock)                    RedrawRequested:
                                              lock Term → build quads → draw
 write to the shell          ◀──bytes──     key press → encode_key() → Session::write
 resize the pty              ◀──size───     window resize → GridSize → Session::resize
 shell ended                 ──Exit────▶    close the window
```

- Events from the pty thread go to the window thread with an `EventLoopProxy`.
- The window thread draws only when something changes (`ControlFlow::Wait`).
- `Session::with_term` holds the lock only while we build one frame.

## Tabs and panes

- `fterm-mux::Mux` keeps the tabs. Each tab has a `Layout`: a tree of panes (`Pane` or `Split`).
  A split has a direction (right or down) and a ratio. `Layout` gives the rects of the panes,
  the dividers (for drawing and for the mouse), and the neighbor of a pane (for Alt + arrows).
- Workspaces: `Mux` keeps its tabs in `Workspace`s (a name and tabs; the first is `main`). The index methods
  (`tabs`, `select`, `cycle`, ...) work on the tabs of the current workspace; the methods with a pane or a tab id
  find it in any workspace. `app/workspace_calls.rs` has the list, the keys, and the name in the tab bar.
- Some panes are not in the tree. `Tab::overlay` is a box over a pane, `Mux` also keeps the window overlay and
  the scratch shell (a drop-down shell), and `Tab::companions` are strips of a pane that close with it. A hidden one
  keeps its program. `app/overlay_calls.rs` and `app/companion_calls.rs` have their keys and their API.
- `app/recent_calls.rs` and `recent.rs` keep the recent panes (Alt+\`, Ctrl+Shift+H).
- Zoom: `Tab::zoomed` shows one pane in the whole tab area; the tree does not change.
- Tab color: `Tab::color` (RGB or none). A profile (`tab_color`), Lua (`fterm.set_tab_color`), the API
  (`set_tab_color`), and the escape sequence `777;fterm-tab;color` all go to `App::set_tab_color`.
  `tabbar.rs` draws it as a line at the top of the tab, and the session file keeps it.
- Every pane has its own grid size. After a split, a close, a drag, a zoom, or a window resize,
  `resize_all_panes` sends the new size to every session.
- The app keeps one `Session` per pane (`PaneId`). Events from a session come with its pane id
  (`UserEvent::Term(PaneId, TermEvent)`), so the app knows which tab got output or ended.
- Before a tab closes, `fterm-term::process::running_children` looks for programs under the shell,
  and fterm asks the user when it finds some.
- The tab bar is drawn with our own renderer (`tabbar.rs`), not with a UI library.
  egui comes later, for the command palette and the AI panel.

## One frame

1. `frame::build_frame` goes over the cells and makes a list of quads (`Instance`):
   backgrounds, cursor, glyphs, underlines. It has no GPU code, so the tests check it.
2. Each cell becomes a `GlyphKey`: the char, its zero-width chars (combining marks, VS16, ZWJ),
   bold/italic, and "wide" (2 cells).
3. A glyph that is not in an atlas yet is drawn:
   - box lines, blocks, and Braille (U+2500–259F, U+2800–28FF) by our own code (`builtin.rs`).
     They fill the whole cell, so cells meet with no gaps;
   - all other chars by the font (`font.rs`): the whole cluster is shaped at once, and it is
     made smaller (or, for emoji, bigger) to fit into its 1 or 2 cells.
4. There are two atlases: **mask** (R8, normal text and builtin chars) and **color** (RGBA, emoji).
   When an atlas is full, it grows 2 times and the frame is built again.
5. `renderer.rs` builds one frame from parts (`FrameParts`): the tab bar, the panes, the dock,
   the toasts, the palette, and a message box on top. Then it sends all quads to the GPU and draws them with one draw call.
6. `shader.wgsl` has three kinds of quads: solid (`0`), glyph (`1`, mask × text color), color glyph (`2`).

## Events, agents, and the dock
- `fterm-term/src/io_loop.rs` reads the pty. `osc.rs` finds our OSC sequences in the bytes
  (7, 9, 99, 133, 777) before the alacritty parser drops them, and sends them as `TermEvent::Osc`.
- `fterm/src/notify.rs` is the notification center: the history (for the Events panel), the toasts,
  and the timers. `on_notification` in Lua can change or drop each one.
- `fterm/src/agent.rs` keeps the agent state of each pane (`777;fterm-agent;<state>`): the tab dot,
  the notification for a new state, and `on_agent` in Lua.
- The dock: `fterm/src/panels.rs` is the model (open, the active panel, the keyboard, the selection, the scroll)
  and makes the rows. `fterm-render/src/dock.rs` places and draws it. The dock takes its part of the window
  first, and the tab gets the rest (`Running::tab_area`), so panes, splits, and toasts move by themselves.

## History
- `fterm-history` keeps the commands and folders in memory and in two JSON lines files. New records go to the
  end of a file with one `write`, so many windows can write at the same time. Each window reads the new lines
  of the others (`refresh`). A file with two times the limit of lines is written again (a temp file, then rename).
- The shell scripts send the command text (OSC 633;E). The pty loop runs the parser only up to the end of
  133;B and reads the cursor there (`InputStart`), so fterm knows where the typed text starts.

## The local API (Phase 7)
- `fterm-api`: JSON-RPC 2.0, one JSON object per line, over a local socket (the `interprocess` crate):
  a named pipe on Windows (only the owner and the system can open it), a unix socket in a 0700 folder elsewhere.
- The server has a listener thread and one thread per client, so a slow call (`wait_for`) does not stop
  other clients. It knows `subscribe` / `unsubscribe` itself; the app answers all other methods.
- A client thread sends each call to the event loop (`fterm/src/api.rs`, `UserEvent::Api`) and waits for the
  answer. Most methods answer at once (`app/api_calls.rs`, `api_dispatch`). The slow ones keep the reply
  and answer later: `wait_for` (`App::waits`), `screenshot` (at the next frame), `ai_ask` with `wait`
  (`App::ai_waits`, at the end of the AI answer), and `review` (when the user ends the review). A timer sends
  a timeout to old waits.
- Reading or typing into a pane of somebody else, `spawn`, and reading the AI chat need a yes from the user
  (`access.rs`). `ai_ask` also needs `ai.api_access` in the config: it costs the user's money.
- Each window writes `instances/<pid>.json` with its socket, and every pane gets `FTERM_SOCKET`.
  So `ftermctl` and `ftermctl mcp` find the right window.
- MCP clients (Claude Code, OpenCode) speak stdio, not pipes: `ftermctl mcp` is a small process between them
  and the pipe.

## Agents and Claude Code

- `agent.rs` keeps the state of the agent of each pane (the tab dots). `agent_info.rs` (pure) keeps what it does:
  its steps, its question, its todos, the changed files, and the cost.
- The data comes from Claude Code: the hooks run `ftermctl agent-event --hook` (API `agent_event`) and
  `ftermctl agent-session --hook` (API `agent_session`), and `ftermctl statusline` wraps the status line (API
  `agent_usage`). `claude_hooks.rs` installs them.
- `agent_session` gives each pane its conversation, so a restored pane gets `claude --resume <id>`.
- `agent_history.rs` reads the past conversations of a folder: our `agents.jsonl` and the files of Claude Code (read
  only). `app/agent_calls.rs` shows the list (H in the Agents panel).

## The AI panel
- `ai_chat.rs` is the model: the turns, the input, the context chips. It has no drawing and no network.
- `app/ai_calls.rs`: `ai_submit` sends a question. The keyboard (`ai_send`) and the API (`ai_ask`) both use it,
  so `on_ai_request` sees every question. The answer streams on the thread `fterm-ai` and comes back as
  `UserEvent::Ai(id, event)`. At its end, `ai_event` sends the API event `ai_answer`.
- There is one chat in each window. Only one answer runs at a time.

## Colors and themes

- `fterm-term/src/colors.rs` has the palette of a pane: 16 colors, the 256-color table, and the default fg,
  bg, and cursor. The theme in use gives them (the default is Catppuccin Mocha).
- Apps can change colors (OSC 4, 10, 11). These changes win over the palette (`palette_changes = false`
  in the config or a profile turns them off).
- `fterm-config/src/theme.rs` reads a theme: JSON (a file in a `themes` folder, a string, or a Lua table)
  or a Windows Terminal color scheme. The UI colors are roles (`accent`, `surface`, `text_dim`, …); a role
  that the theme does not give is made from the terminal colors. Built-in themes are in `assets/themes`.
- `fterm/src/themes.rs` picks the theme: a theme set while fterm runs (palette, `set_theme`) wins over the
  config; `theme = { light, dark }` follows the system. It gives the colors to every pane and to the renderer.
- `fterm-render/src/theme.rs` is the only file of the renderer with color values. All other render code takes
  the UI colors from it by role.
- Harmonize (`fterm-term/src/harmonize.rs`): colors that programs choose themselves (truecolor, the 256-color
  cube) move toward the theme in Oklab (hue and chroma to the nearest theme color, lightness kept as a
  distance from the background), then get at least `min_contrast`. The settings come from the theme, and
  `harmonize` in the config changes them. `Palette::set_harmonize` turns it on for a pane; a profile can turn
  it off (`harmonize = false`).
- The surface is sRGB, so we send linear colors to the GPU (`fterm-render/src/color.rs`).
- Text gamma: font glyph alpha goes through `text_alpha` (alpha^(1/1.45)), so light text on a dark
  background does not look thin. Builtin chars do not use it (their edges are exact).

## Languages (l10n)

- `fterm-config/src/l10n.rs`: the texts of the UI by key. English is built in (`assets/l10n/en.json`);
  another language is a JSON file with the same keys (`language` in the config). A missing key stays English.
  A text can have plural forms (`one`, `few`, `many`, `other`).
- The code takes every UI text with `tr!("key", name = value)` or `trn!("key", n)`. Tests check that each key
  in the code is in `en.json`, that each key of `en.json` is used, and that `notify(` calls have no English
  literals.

## The Review

- `fterm/src/review.rs` is the model (no window code): items from markdown or from a list, each one marked
  Ok, commented, changed, added, or removed, and the result for the agent. The app draws it into the grid of
  a pane (`Pane::review`), like a scene, and gives it the keys. The pane is an overlay over the pane of the
  agent, a split next to it, or a tab after its tab (`plan_review.place`).
- `app/review_calls.rs`: the API method `review` opens the Review at the pane of the agent and keeps the reply until the user ends the
  review. `plan_review` in the config (and a question, R or Esc) decides if a plan of Claude Code plan mode
  opens.
- The plan mode hook is `ftermctl review --hook` (a `PreToolUse` hook for `ExitPlanMode`;
  `ftermctl/src/review.rs`): approved = allow, changes = deny with the feedback, cancelled = Claude's own dialog.
  `fterm/src/claude_hooks.rs` merges the fterm hooks into the Claude Code `settings.json` (palette action).

## Fonts

- JetBrains Mono is inside the app (`assets/fonts`, license: `assets/fonts/OFL.txt`).
- System fonts are loaded too (about 90 ms). cosmic-text picks a fallback font by script:
  on Windows, for example, Segoe UI Emoji, Yu Gothic, Microsoft YaHei, Malgun Gothic, Segoe UI.
- A char that no font has is drawn as a hollow box.
- The cell size is whole pixels, so the cell backgrounds have no gaps between them.
- Tests use `Fonts::embedded_only`, so they give the same result on every machine.
  Tests that need system fonts (emoji, CJK) do nothing when the font is not there.

## Config

- `fterm-config` runs `fterm.lua` with Luau (`mlua`, built from source, sandbox on) and reads the table.
  Errors have the path of the bad field (`profiles[2].command: missing`).
- Lua functions in the config do not change the app directly. They call `fterm.spawn`, `fterm.send_text`, …,
  these calls go into a queue (`ApiCall`), and the app runs them after the function ends.
- The app watches the folder of the config file (`notify`), waits 150 ms after the last change, and loads it again.
  A broken file keeps the old config and shows the error.
- Keys: a winit key becomes a `KeyChord` by its physical key, and the `Keymap` gives the `Action`.

## Selection

- alacritty removes `term.selection` when an app erases or rewrites the selected lines.
  `fterm-term/src/select.rs` (`StickySelection`) keeps the user's selection and puts it back,
  moved by the lines that went into the history. `Session::with_term` calls it before every use.
- Change the selection only through `Session::with_term_mut` and `StickySelection::set`.
- Copy mode (`copy_mode.rs`) uses the vi mode of alacritty. After each move the view scrolls to the cursor.
  `CopyState` (in the `Session`, with the selection) keeps the start of the selection, the search, and the last jump;
  the selection is made again from its start to the cursor after each step. Its keys are a table in
  `fterm-config/src/copy_keys.rs` (chars with their case, two-key bindings); `app/copy_calls.rs` reads the keys, the
  search prompt, and Lua actions.
- Limit: when the history is full (10 000 lines) and output is very fast, a restored selection can move by a few lines.

## Pictures

See [PICTURES.md](PICTURES.md).

- `osc.rs` finds `OSC 1337;File=` and hides its body from the alacritty parser, which would keep a copy of
  megabytes. The pty loop decodes the picture (`fterm-term/src/picture.rs`) and marks its top-left cell with a
  hidden link (`fterm-picture:<id>`). So the picture moves with the text.
- `pictures.rs` keeps the pictures of a pane by that id. It drops one when its mark leaves the grid, or when the
  pictures take too much memory.
- `fterm-render/src/picture.rs` finds where each picture goes, with no GPU code. The frame has layers, so a picture
  is drawn above the quads of its layer and an overlay still covers it.
- A picture pane (`Pane::picture_pane`) has no program; the picture fills it. `markup.rs` is the pure model of
  the drawing on it, and `app/markup_calls.rs` has the mouse, the keys, and `show_image` with `wait`.

## The dashboard

`dashboard.rs` (pure) makes the tiles (panes, command output, buttons, tiles from the API) into scene commands.
`app/dashboard_calls.rs` draws them in a scene pane and gives the keys and clicks to the tiles. `runner.rs` runs the
commands of the config in the background.

## Mux panes (panes that live on)

The programs of mux panes run in the mux server (`fterm-server`), not in the window. See [MUX.md](MUX.md).

- The local server is `fterm --mux-server` (started in the background by the first mux pane). A server on
  another machine is `ftermctl mux serve` there; the window reaches it with a command (`ssh ... ftermctl mux
  proxy`), and `ftermctl mux proxy` joins its stdin and stdout to that server.
- On the socket: frames `[u32 length][u8 kind][payload]` (`frame.rs`). Control messages are JSON (`proto.rs`:
  spawn, attach, detach, close, list, stop, set/get layout); the output and the keys of a pane are raw bytes.
- The server runs each program in a normal `Session`, with `SessionOptions::tap` to get its output bytes.
  Its own `Term` answers the terminal queries of the program.
- The window keeps its own copy of the `Term`: `Session::remote` sends the keys and the sizes over a
  `RemoteLink` and parses the bytes that come into its `RemoteFeed`. On attach the server first sends a snapshot
  (`fterm-term/src/snapshot.rs`: the screen, the history, the cursor, and the modes as escape bytes), then the
  output. So selection, links, copy mode, shell integration, and agent states work as in a local pane.
- One window shows a pane at a time: a new attach takes the pane over.
- The window saves the layout of its tabs (tabs, splits, workspaces) in the server. At start,
  `reattach_mux` brings back the panes that no window shows in that layout.
- `app/mux_calls.rs` has all of this on the window side: which pane is a mux pane (`mux`, a profile's `mux`
  or `remote`), the connection on its own thread with a box to cancel it, "Connect over SSH…", and stop.

## Links

- `fterm-term/src/links.rs` finds URLs in the text (also wrapped to the next line) and OSC 8 links. Each link
  knows its cells.
- `app/link_calls.rs`: Ctrl + click, the underline and the hand cursor while Ctrl is down, the hint labels
  (`open_link_hints`), and the question before a risky link.
- `link_open.rs` (pure, with tests) decides: open at once, ask first (a file, or OSC 8 text that does not show
  its URL), or never (a scheme that is not http, https, ftp, mailto, or file).

## Known limits

- The alacritty parser does not join graphemes: ZWJ families, skin tones, and flags are drawn as
  separate chars, and `❤️` gets 1 cell. See the roadmap.
- No bidi: right-to-left text is shown in grid order.
