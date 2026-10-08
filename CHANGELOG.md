# Changelog

All big changes of fterm are in this file. The newest version is at the top.
The form is from [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- **The Review tab for plans of Claude Code plan mode opens only when you want it.** fterm asks first:
  R opens the Review tab, Esc (or no answer in 2 minutes) gives the plan dialog of Claude. `plan_review = "always"`
  or `"never"` in the config changes this, and the palette action `plan_review_mode` changes it while fterm runs.
  An agent or a script that asks for a review itself (`review_plan`, `ftermctl review`) still gets the tab.

## [0.2.0] - 2026-10-08

More for agents (plan reviews, screenshots, colored text, the AI panel), themes for the whole window,
and colors of programs that fit the theme.

### Added
- **The AI panel for agents.** The API methods `ai_read`, `ai_ask`, `ai_input`, `ai_stop`, `ai_clear` and the
  event `ai_answer`, `ftermctl ai read|ask|input|stop|clear`, and the MCP tools `ai_read` and `ai_ask`.
  An agent can read the chat and ask questions there. `ai_ask` uses your key, so it works only with
  `ai = { api_access = true }` in the config.
- **Screenshots of panes.** The API method `screenshot`, `ftermctl screenshot [--pane N] [FILE.png]`,
  and the MCP tool `screenshot_pane`. An agent gets the PNG as a picture, so it can see what it drew in a
  scene, or how a program looks. It works when the window is under other windows too.
- **Agents with no hooks.** When `claude`, `opencode`, `codex`, `aider`, or `gemini` runs in a pane, it is
  in the Agents panel and has a dot on its tab. Its state comes from the window title (Claude Code shows a
  spinner while it works). Hooks, when they are set up, are more exact and win.
- **The colors of programs can fit the theme.** `harmonize` in a theme (or in the config) moves the colors that
  programs choose themselves (truecolor and the 256-color table) toward the theme: the theme's red for a red, gray
  in a gray theme, dark text on a light theme. Text always keeps a minimum contrast. A profile can keep exact
  colors (`harmonize = false`), and `toggle_original_colors` shows them as they are. Ctrl + Shift + ] and
  Ctrl + Shift + [ try a stronger or weaker value live. Palette colors that a program changed (Far Manager sets
  the old console colors) fit the theme too, and `palette_changes = false` (in the config or a profile) keeps the
  theme's palette. See docs/THEMES.md.
- **Review a plan item by item.** An agent can show its plan in a Review tab: you mark each item Ok, comment on
  it, change its text, add or remove items, and send the review back. Claude Code plan mode uses it through a hook
  (all Ok accepts the plan, any change goes back to Claude), other agents through the MCP tool `review_plan`, and
  scripts through `ftermctl review plan.md`. See docs/REVIEW.md.
- **Install the Claude Code hooks from the palette.** "Install Claude Code hooks (agent states)" adds the fterm
  hooks to the Claude Code settings after a question. Your other settings and hooks stay, and the old file is
  kept as `settings.json.bak-fterm`.
- **Read a whole event.** In the Events panel, Space (or Right) opens the full text of an event, for
  example a long message from an agent. Enter goes to its pane, Ctrl + C copies it, Esc goes back.
- **Themes.** One theme gives all colors: the terminal and the whole UI (the tab bar, the dock, toasts,
  the palette, the frames of panes). A theme is JSON with color roles; missing UI colors are made from the
  terminal colors. Windows Terminal color schemes work as themes. Catppuccin Mocha (the default) and Latte
  are built in; other themes are files in a `themes` folder next to `fterm.lua`. Choose one in the config
  (`theme = "Nord"`, or `{ light = ..., dark = ... }` to follow the system), in the palette ("Theme…"),
  with `ftermctl theme`, or with the API and MCP (`set_theme`). See docs/THEMES.md.
- **Tab colors.** A tab can have a color line at its top, so you see it from other tabs. A profile
  (`tab_color`), Lua (`fterm.set_tab_color`), agents and scripts (`set_tab_color`, `ftermctl tab-color`, the
  MCP tool `set_tab_color`), and any program (`ESC ] 777 ; fterm-tab ; color ; #rrggbb BEL`) can set it.
  It is saved with the session.
- **Full screen** with no window frame: Alt + Enter (the action `toggle_fullscreen`), like WezTerm.
- **Text with colors and styles.** `get_text` with `styled: true`, `ftermctl get-text --styled`, and
  `read_pane` with `styled` give the colors (`#rrggbb`) and the styles (bold, italic, underline, strike,
  dim) of the text, as the user sees it. So an agent can find red errors, or what is selected in a menu.

### Fixed
- A message box with a very long line (for example a long path) went out of the window.
- On Windows, after you made the window taller, PowerShell wrote what you typed one row (or more) above
  the prompt, and the old text stayed there. The rows now stay where ConPTY has them.

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

[Unreleased]: https://github.com/newenclave/fterm/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/newenclave/fterm/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/newenclave/fterm/releases/tag/v0.1.0
