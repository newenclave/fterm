# Changelog

All big changes of fterm are in this file. The newest version is at the top.
The form is from [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- **Screenshots of panes.** The API method `screenshot`, `ftermctl screenshot [--pane N] [FILE.png]`,
  and the MCP tool `screenshot_pane`. An agent gets the PNG as a picture, so it can see what it drew in a
  scene, or how a program looks. It works when the window is under other windows too.
- **Text with colors and styles.** `get_text` with `styled: true`, `ftermctl get-text --styled`, and
  `read_pane` with `styled` give the colors (`#rrggbb`) and the styles (bold, italic, underline, strike,
  dim) of the text, as the user sees it. So an agent can find red errors, or what is selected in a menu.

### Fixed
- A test of the data folder failed on Linux and macOS (it used a Windows path).

## [0.1.0] - 2026-10-02

The first version. Portable archives for Windows, Linux, and macOS.

### Added
- **The terminal:** a GPU-drawn grid (wgpu: DX12, Vulkan, Metal, GL), fallback fonts, color emoji, and
  our own box and Braille glyphs. The GPU backend and power are in the config.
- **Tabs and split panes:** focus, resize, zoom, and close. fterm asks before it closes while something
  runs.
- **Mouse and keys:** sticky selection, auto-scroll, copy mode, paste, links, mouse for programs, IME,
  and Shift+Enter.
- **Config in Luau:** profiles, key bindings, live reload, and a command palette with actions and user
  commands.
- **Shell integration** (PowerShell, bash, zsh, and WSL): the folder and the command of each pane, its
  exit code, and "a long command ended".
- **Notifications:** toasts, a Lua filter, and OS notifications (off by default). A dock with the
  Events and Agents panels.
- **Agents:** agent states on tabs, Claude Code hooks, and the title of the window shows waiting agents.
- **History:** commands and folders are saved; Far-like popups and grey hints while you type.
- **Sessions:** the window, the tabs, and the panes come back at start, with the old text and the
  programs. Named sessions, and a Lua hook before a session opens.
- **WSL:** a profile for each distro; new panes open in the same distro and folder.
- **The API:** JSON-RPC over a local socket. `ftermctl` for shells and scripts, and an MCP server
  (`ftermctl mcp`) for agents. fterm asks the user before a client uses other panes. `wait_for` and
  messages between agents.
- **Braille scenes:** a pane to draw into (dots, lines, shapes, text, colors, and charts), from the API,
  ftermctl, and MCP. A scene draws itself again for a new size.
- **A guide for agents** (`ftermctl guide`), and a Claude Code skill with it.
- **AI:** an AI panel with a streaming chat (several providers), context chips, "Explain the last error",
  and a task in the prompt turned into a command (Ctrl+Shift+G).
- **Portable mode:** fterm runs from one folder, with `--config PATH` and `data_dir`.
- **Debug:** record the output of panes with `FTERM_RECORD` (asciinema files).
- An icon: a bold f made of Braille dots.

### Fixed
- ConPTY drew the screen again after a resize, so lines came twice in the history.
- The text of the prompt lost its first letter when the task ended with a space.
- Less memory at start (from about 337 MB down to about 161 MB on DX12).

[Unreleased]: https://github.com/newenclave/fterm/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/newenclave/fterm/releases/tag/v0.1.0
