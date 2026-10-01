# fterm Roadmap

## What is fterm?
fterm is a terminal app. It is like WezTerm. It uses the GPU to draw text.
First we make it for Windows. Later it works on Linux and macOS too.

## Main goals
- Full Unicode: Chinese, Japanese, emoji, and other hard text.
- Tabs.
- Claude Code and other AI tools work well inside it.
- AI features:
  - Profiles: start `claude`, `opencode`, `ollama run ...` and others in a new tab.
  - AI panel: chat with a model inside the terminal.
  - Text to command: you write what you want, AI gives a shell command.
  - API and MCP: other AI agents can control the terminal.
  - Events: agents and systems can send events (for example, "task is done").

## Tech stack
We use **Rust**.

| Part | Library | Why |
|---|---|---|
| Window and input | `winit` | Works on all systems. Has IME and DPI support. |
| Drawing | `wgpu` | Fast GPU drawing (DX12, Vulkan, Metal). |
| Fonts | `cosmic-text` | Text shaping, fallback fonts, emoji. |
| Terminal logic | `alacritty_terminal` | Ready parser, grid, scrollback, selection. |
| PTY | tty from `alacritty_terminal` | ConPTY on Windows, pty on Linux/macOS. We can ship a new `conpty.dll` + `OpenConsole.exe`, like WezTerm, because the Windows one has bugs. |
| UI (tabs, panels, dialogs) | `egui` + `egui-wgpu` | We do not need to write our own UI toolkit. |
| Config | TOML (`serde`, `toml`, `notify`) | Easy to read. Reload on change. |
| Async and HTTP | `tokio`, `reqwest` | For AI providers and the API. |
| Secrets | `keyring` | Keep API keys safe in the system store. |

## Project layout
We add crates step by step, only when we need them.

```
fterm/
  Cargo.toml            # workspace
  crates/
    fterm/              # main app: window, event loop
    fterm-render/       # GPU drawing, glyph atlas
    fterm-term/         # terminal logic + pty
    fterm-mux/          # windows, tabs, panes, event bus
    fterm-config/       # config, profiles, key bindings
    fterm-ipc/          # local API + `fterm cli`
    fterm-mcp/          # MCP server
    fterm-ai/           # AI providers: Anthropic, OpenAI-like, Ollama
  docs/
    ROADMAP.md
```

## Phases
After each phase the app must run and work.
Before each phase we write a detailed plan for it.

### Phase 0 — Simple window ✅ (done)
- Make the Cargo workspace and the `fterm` crate.
- Open a window with `winit`. Draw a background color with `wgpu`.
- The window can change size and close.
- Add logs (`tracing`), `rustfmt`, `clippy`.
- Add GitHub Actions: build on Windows, Linux, macOS.
- **Check:** `cargo run` opens a window. CI is green.

### Phase 1 — One working terminal ✅ (done)
- Start a shell (PowerShell by default) with ConPTY.
- Draw the text grid: letters, background, cursor.
- Colors: 16, 256, and true color. Bold, italic, underline.
- Keys go to the shell. Window size changes the PTY size.
- **Check:** `dir`, `git log`, `vim` work. Colors look right.

### Phase 2 — Good Unicode and text ✅ (done)
- Fallback fonts for emoji and Chinese/Japanese.
- Wide letters, combined letters, color emoji.
- Draw box lines ourselves, so there are no gaps.
- Draw Braille chars (U+2800–U+28FF) ourselves too. Each cell is a 2×4 grid of dots.
  The dots fill the whole cell, with no gaps between cells, so Braille graphs look like real pixels.
- Shape a char together with its combining marks (é, emoji with skin tone, ZWJ emoji).
- Text gamma, so light text on a dark background does not look thin.
- Maybe ligatures.
- No bidi (right-to-left order) for now: Arabic and Hebrew get the right glyphs, in grid order.
- Known limit (from the alacritty parser): it does not join graphemes. So ZWJ families (👨‍👩‍👧),
  skin tones (👍🏽), and flags (🇫🇮) are drawn as separate chars, and `❤️` (with VS16) gets only 1 cell.
  Fix later with grapheme support (for example, mode 2027) in our own parser layer.
- **Check:** test files with Unicode and emoji look right. Claude Code looks right.

### Phase 3 — Easy to use, Claude works well ✅ (done)
- Scroll back with the mouse wheel.
- Paste (bracketed paste). Mouse support for apps, focus events, IME, click on links.
- Shift+Enter and Alt keys. The kitty keyboard protocol comes later (after Phase 5).
- Selection and copy: see the next section. This is a main goal, not a small thing.
- **Check:** `claude` works fully: many lines of input, big paste, Esc, Ctrl+C, scroll.

#### Selection and copy (done right) — first part ✅ (sticky selection, auto-scroll, copy mode)
In many terminals, copying is painful, most of all when the text is longer than the screen.
Ideas (we pick the order in the Phase 3 plan):
- **Select and scroll together.**
  - Drag the mouse above or below the window: the view scrolls, faster when the mouse is farther away.
  - The mouse wheel works while you select, and the selection stays.
  - Click at the start, scroll (wheel, PgUp/PgDn), then Shift+click at the end.
  - New output does not move the view and does not remove the selection while you are scrolled up.
- **Select without the mouse:** a copy mode with the keyboard (arrows or vi keys),
  search (`/`) inside the scrollback, `v` to select, `y` to copy.
- **Smart selection:**
  - double click = word, but paths, URLs, git hashes, and numbers are one word;
  - triple click = the whole line, also when it wraps;
  - Alt + drag = a rectangle (block).
- **Copy whole blocks with one key** (with shell integration, Phase 6):
  - "copy the output of the last command" (OSC 133);
  - click next to a command = select all its output.
- **Clean copy:**
  - no spaces at the end of lines;
  - lines that were only wrapped by the screen are joined again;
  - option: remove frame chars (`│ ╭ ╰`) and their indent, for example when you copy code from Claude Code;
  - copy as plain text, with colors (ANSI), as HTML, or as a Markdown code block.
- **Quick select (hints):** press a key, short labels show on URLs, paths, and hashes;
  press a label to copy it (like kitty hints or WezTerm quick select).
- **Big output:** open the scrollback (or the output of one command) in an editor or a pager, and search or copy there.
- Small toast: "copied 42 lines". Optional: copy on select, right click = paste.
- **Check:** copy 300 lines of Claude output that go past the screen, with no extra spaces,
  no broken lines, and no frame chars.

### Phase 3b — Images in the terminal
- First a test: which image protocols get through ConPTY (Kitty, Sixel, iTerm2)?
  A new `conpty.dll` + `OpenConsole.exe` from Windows Terminal may be needed.
- Our own pty read loop, so we can catch image sequences before the parser.
  It also reads the pty until the end of the stream after the process ends. ✅ (done in Phase 6.0: `io_loop.rs`)
- iTerm2 (OSC 1337) and Sixel first, then the Kitty graphics protocol.
- Images are quads that stay on their cells and scroll with the text.
- `fterm cli image file.png` and an MCP tool: they send images over the local API (Phase 7), not over ConPTY.
- **Check:** `yazi` shows image previews, `chafa`/`imgcat` work, an agent shows a picture.

### Phase 4 — Tabs ✅ (done)
- Model: window → tabs → panes. A tab has a tree of panes (from the start, so splits fit in later).
- Tab bar drawn with our own renderer (egui comes later, for real widgets). Hot keys.
  Tab title from the app (OSC 0/2). Rename tabs.
- Later: drag tabs with the mouse; new tabs open in the folder of the current tab (needs OSC 7, Phase 6).
- Ask before closing a tab with a running app.
- **Check:** open 5+ tabs with different shells. Switch and close them. No processes stay alive.

### Phase 4b — Split panes ✅ (done: split, focus, resize, zoom, close)
Still to do later: move and swap panes, move a pane to another tab, broadcast input, layouts in the config, API control.
- Split a pane **right** or **down**, many times: each tab is a tree of panes (like tmux and WezTerm).
  Each pane has its own shell or app, its own scroll history, selection, and copy mode.
- **Focus:** click a pane, or Alt + arrows. The pane with focus has a colored border;
  the others are a bit darker (optional).
- **Resize:** drag the line between panes with the mouse, or Alt + Shift + arrows.
  Each pane sends its own new size to its pty.
- **Zoom:** one key makes the current pane full size in the tab, the same key brings the layout back.
- **Move and swap** panes; move a pane to a new tab, or join a tab into a pane.
- **Close** a pane: the space goes to its neighbor. The last pane closes the tab.
- Drawing: one GPU frame for the whole window. Each pane is a rect with its own grid quads
  (a scissor rect per pane), so splits do not cost more draw calls than needed.
- **Broadcast input** (optional): type into many panes at once.
- Later (with Phases 5–7): save layouts in the config (`fterm.lua`), open a layout from a profile
  (for example: Claude on the left, the shell and `cargo watch` on the right),
  and control panes from the API and MCP (`split`, `focus`, `resize`, `send-text` to a pane).
- **Check:** split 2×2, run `claude`, `vim`, `htop`-like output, and a shell. Resize with the mouse,
  zoom one pane, close panes in any order. Every pane gets the right size, and no processes stay alive.

### Phase 5 — Config and profiles ✅ (done: Luau config, profiles, keys, live reload, command palette)
- The config is a **script**, not only a list of values: it has functions and tables,
  so you can use `if`, loops, and your own helpers (like WezTerm with Lua).
  - Plan: **Luau** (a fast Lua with types and a sandbox, from Roblox) through the `mlua` crate.
    It is built from source (`vendored`), so it works the same on Windows, Linux, and macOS.
    Other choices to compare in the Phase 5 plan: LuaJIT (the fastest, also through `mlua`),
    plain Lua 5.4, Rhai (pure Rust, but slower).
  - `fterm.lua` returns a table: font, colors, keys, profiles.
  - Functions for events: `on_key`, `on_output`, `on_title`, `on_agent_event` (Phase 6),
    and your own commands for the command palette.
  - A small API: `fterm.spawn(...)`, `fterm.send_text(...)`, `fterm.notify(...)`, `fterm.copy(...)`.
  - The script runs only on load and on events, never for every frame, so it does not make drawing slow.
  - Errors in the config: fterm starts with the last good config and shows the error.
- It reloads when you save the file.
- Font, color themes, key bindings.
- Profiles: PowerShell, cmd, WSL, Git Bash, **claude, opencode, ollama run ..., openclaude**.
  Each profile has: command, arguments, folder, env vars, icon.
- Command palette (Ctrl+Shift+P): "new tab with profile".
- Braille style: `braille_style = "pixels"` (default, no gaps) or `"dots"` (round dots).
- **Check:** change the config and see the change at once. Start an AI tool from the palette.

### Phase 6 — Events and shell integration ✅ (done: OSC loop, shell integration, notifications, agent dots, Claude hooks, the dock with Events and Agents panels)
- An event bus inside the app.
- Support OSC 7 (current folder), OSC 133 (command start and end, exit code), OSC 9 and OSC 777 (notifications), bell.
- Tab status: "agent is working", "agent waits for you", "done", "error". Show a badge and a system notification.
- Every pane gets the env var `FTERM_PANE_ID`.
- Claude Code hooks (`UserPromptSubmit`, `Notification`, `Stop`) send `OSC 777;fterm-agent;<state>` to the terminal.
  No `fterm cli` is needed. A ready example for `settings.json` is in [CLAUDE.md](CLAUDE.md).
- A notification center: toasts that do not get in the way, a history, a Lua filter, and optional OS notifications (off by default).
- A dock with service panels: an Events panel and an Agents panel.
- **Check:** Claude finishes in a background tab → the tab shows a badge and a notification. `cd` changes the tab folder.

### Phase 6b — Folder and command history (like Far Manager) ✅ (done: the command text, the history files, Alt+F8 / Alt+F12, and hints)
An idea from the user. Shell integration (Phase 6) already tells us the folder (OSC 7) and where each command starts and ends (OSC 133).
- **Folder history** (like Alt+F12 in Far): fterm saves every folder where a pane was.
  - A list window, like the command palette: newest at the top, type to filter, Enter = `cd` to it in the active pane
    (or open a new tab or split there).
  - Folders you use often go higher (count + time). You can pin a folder or delete it from the list.
- **Command history** (like Alt+F8 in Far): fterm saves every command with its folder, exit code, time, and how long it took.
  - The command text comes from the shell script (a new mark with the command line, like VS Code `OSC 633;E`),
    so it is exact, not read from the screen.
  - A list window: filter by text, by "only this folder", by "only good (exit 0)". Enter = put the command
    in the prompt (not run it), Shift+Enter = run it, Ctrl+C = copy.
  - The history is one file for all tabs and sessions (`%APPDATA%\fterm\history.db` or a JSON lines file).
    Commands with secrets can be skipped: a space at the start, or a Lua filter `on_history`.
- **Hints while you type** (like Far): a grey suggestion after the cursor from the history of this folder.
  - To think about: PowerShell (PSReadLine), fish, and zsh already have their own hints. So it is off by default
    and fterm does not show it when the shell has its own. Right arrow = take it.
- Keys (can be changed): `Alt+F12` folders, `Alt+F8` commands. Also in the palette and in Lua (`fterm.history`).
- **Check:** cd into 5 folders in 2 tabs, Alt+F12 shows all 5, Enter goes there. Run commands, close fterm,
  open it again: Alt+F8 shows them with exit codes.

### Phase 6c — Close the window safely (an idea from the user) ✅ (done)
Now the window × (and Alt+F4) closes fterm at once, with all tabs and programs. It must ask first.
- The × of the window asks: "Close fterm? 3 tabs, 5 panes. claude works in tab 2, cargo runs in tab 3."
  Enter = close, Esc = stay. It uses the same box as "close the tab".
- Config: `confirm_close = "running"` (the default: ask only when a program or an agent runs in some pane,
  like for tabs), `"always"`, or `"never"`.
- **Lua hook** `on_close_window = function(info, fterm) … end`. `info` has `tabs`, `panes`, and lists of
  `running` programs and `agents` (with their state). It returns:
  - `true` = close now, do not ask (for example: only idle shells are open);
  - `false` = do not close (for example: an agent is `working`) — fterm shows a toast "Closing was stopped by your config";
  - `nil` = the normal rule from `confirm_close`.
- The same rule for the last tab and for `fterm cli quit` (Phase 7).
- **Check:** × with only shells closes at once (with `"running"`); with `claude` running it asks; a Lua hook
  that returns `false` keeps the window open and shows the toast.

### Phase 6d — WSL (an idea from the user)
- **Profiles:** fterm finds the WSL distros (`wsl.exe -l -q`) and makes a profile for each one
  (`wsl.exe -d Ubuntu --cd ~`), like Windows Terminal.
- **Shell integration in WSL:** fterm starts bash or zsh with its script (it still loads your `~/.bashrc`),
  so OSC 7 and OSC 133 work there with no setup.
- **Folders:** a WSL pane tells Linux paths (`/home/me`). A new tab or split from it opens the same distro
  in the same folder (not a Windows shell). A Windows pane can open `\\wsl$\Ubuntu\home\me` as a WSL pane.
- **History:** each folder and command knows its "world" (Windows, or WSL with the distro name),
  so Alt+F12 does not offer `/home/...` to PowerShell or `C:\...` to bash, and `cd` uses the right form.
- **API and MCP from WSL:** Linux cannot open the Windows named pipe, but WSL can run `.exe` files.
  fterm sets `WSLENV=FTERM_SOCKET/u:FTERM_PANE_ID/u`, so these vars go into WSL, and `ftermctl.exe`
  (and `ftermctl.exe mcp` for Claude Code that runs in WSL) works there.
- Claude Code hooks in WSL work as they are (the escape sequences go through the terminal).
- **Check:** open Ubuntu from the profile list; `cd /tmp`, then split: the new pane is Ubuntu in `/tmp`;
  Alt+F12 in Ubuntu shows only Linux folders; Claude Code in WSL uses the fterm MCP tools.

### Phase 7 — Local API, CLI, and MCP ✅ (done: the API, `ftermctl`, `ftermctl mcp`, access questions, messages; see [API.md](API.md) and [MCP.md](MCP.md))
The CLI is a separate console program, `ftermctl` (not `fterm cli`): fterm.exe is a window program,
so a shell would not wait for it and would not show its output. Later: an HTTP transport for MCP clients
that want a URL, and saved "always allow" answers.
- Local API: JSON-RPC over a named pipe (Windows) or a unix socket (Linux/macOS).
- Commands: `list`, `spawn`, `send-text`, `get-text` (screen, scrollback, output of the last command), `set-title`, `notify`, `subscribe` (stream of events).
- `fterm cli ...` — the same app, but in CLI mode.
- `fterm mcp` — an MCP server (stdio). Claude Code and OpenCode can open tabs, run commands, read output, and listen to events.
- **Control the terminal:** tabs and panes (`split`, `focus`, `resize`, `close`, `zoom`), titles, the dock and panels.
- **Sessions talk to each other** (an idea from the user):
  - every pane has an id (`FTERM_PANE_ID`), so an agent knows "me" and can find "the others";
  - `list` shows all panes: the program, the folder, the agent state, the last command, and its exit code;
  - an agent in one pane can read the screen or the last command output of another pane,
    and send text to it (for example, Claude in the left pane runs tests in the right pane and reads the result);
  - **messages between agents:** `send-message <pane> <text>` puts a message into the inbox of another pane.
    The other agent reads it with an MCP tool (`read-messages`) or gets it as an event.
    So two Claude sessions can work together: one writes code, one reviews it;
  - `wait-for <pane> <event>`: wait until a command ends, or an agent is `done` or `waiting`.
- MCP tools are the same as the API commands. MCP resources: the screen and history of each pane.
- Safety: only the current user can use the pipe. Optional "Are you sure?" for `send-text`.
  A pane can be marked "no remote control" (other agents cannot type into it or read it).
- **Check:** `claude mcp add fterm -- fterm mcp`. Claude opens a tab, runs `cargo test`, and reads the result.
  Two Claude sessions in two panes send messages to each other.

### Phase 8 — AI panel ✅ (done: Anthropic and OpenAI-like providers with streaming, the AI panel, context chips, "Explain the last error", commands from answers; see [AI.md](AI.md))
- `fterm-ai`: one interface for many providers:
  Anthropic API, OpenAI-like APIs (OpenAI, OpenRouter, LM Studio, Ollama `/v1`), and Ollama.
- Answers come as a stream. API keys live in `keyring`.
- Side panel with chat. You can add context: selected text, last command output, current folder, error.
- **Check:** ask about a failed command with local Ollama and with the Claude API.

### Phase 9 — Text to command ✅ (done: the task in the prompt + Ctrl+Shift+G; see [AI.md](AI.md#text-to-command))
- A small window (for example Ctrl+Shift+Space).
- You write a task in normal words. AI gives a command for your shell and OS.
- You see the command first. Then it goes into the terminal without Enter (or runs after you say yes).
- **Check:** "find the 10 biggest files in this folder" gives a good command for PowerShell and for bash.

### Phase 9b — Restore the session (an idea from the user) — in progress
Done: 9b.0 (save the window, the "Restore the last session?" box, the palette command) and
9b.1 (the list of sessions, Ctrl+Shift+S: named sessions and the last 20 closed windows),
9b.2 (a program that ran comes back into the prompt; Claude Code as `claude --continue`;
`restore_programs` and `restore_agents` = "prompt" | "run" | "never"),
9b.3 (the old text of a pane in grey above the new prompt, `restore_history = 200`).
Still to do: `on_restore`. Later: ConPTY clears the screen when a shell starts, so the old text goes to
the history and fterm scrolls it into view. With our own ConPTY code (the `PSEUDOCONSOLE_INHERIT_CURSOR` flag)
it could stay on the screen.
Like a browser: when you open fterm again, you can get back what you had before you closed it.
- **What is saved:** the window size and place; all tabs (their titles, the active tab); the split layout of each tab
  (the tree and the sizes); for each pane its profile, its folder (OSC 7), and its title; the dock (open, the panel, the size);
  and the AI chat.
- **When:** when fterm closes, and every 30 seconds (so a crash or a reboot does not lose it).
  The files are in `%LOCALAPPDATA%\fterm\sessions\` (one live file per window).
- **Restore:** `restore = "ask"` (the default: at start, a small box "Restore the last session? 4 tabs, 7 panes"),
  `"always"`, or `"never"`. Also the command palette: **Restore the last session**.
- **Programs cannot come back**, only shells: each pane starts its profile in its old folder. Panes where a program ran
  (for example `cargo watch`, `npm run dev`) show a line "This pane ran: npm run dev" and a key to run it again.
- **Agents:** a pane where Claude Code ran can start `claude --continue` in its folder (it brings back the conversation),
  with a question first (`restore_agents = "ask" | "always" | "never"`).
- **The old text (optional):** the last screen of each pane (and N lines of history) is drawn above the new prompt in grey,
  so you see where you were. `restore_history = 200` lines; `0` = off. Secrets: it uses the `on_history` filter.
- Lua: `on_restore = function(session) … end` can change or drop tabs before they open.
- **Check:** open 3 tabs with splits in different folders, close fterm, open it again: the box asks, Enter brings back
  the same tabs, layouts, and folders; a crash (kill the process) still restores the state from the last 30 seconds.

### Phase 10 — All systems and release
- Finish Linux and macOS support (pty, fonts, IME, macOS menu).
- Installers: MSI (`cargo-wix`) and winget, dmg, AppImage and deb.
- Code signing, auto update, many windows, save and restore sessions.

### Phase 11 — Braille scene
- Braille gives 2×4 "pixels" in every cell. So an 80×24 terminal is a 160×96 pixel screen.
- A Braille canvas: `draw_dot`, `clear_dot`, lines, rects, circles, and text on top.
  Idea and code from [tank_rs](https://github.com/newenclave/tank_rs/blob/master/src/braille_canvas.rs)
  (dot bits: `0x01 0x02 0x04 0x40` for the left column, `0x08 0x10 0x20 0x80` for the right, char = `U+2800 + bits`).
- A "scene" pane: agents and apps can draw charts, graphs, and simple games there,
  through `fterm cli draw ...` and MCP tools (Phase 7).
- Colors per cell, so one scene can have many colors.
- **Check:** an agent draws a live chart (for example, CPU or token use) in a Braille pane.

## Next step
Choose the next phase: 9b (restore the session), 6d (WSL), or 3b (images).
Write a detailed plan for it. Then build it.
