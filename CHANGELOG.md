# Changelog

All big changes of fterm are in this file. The newest version is at the top.
The form is from [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- **Resize the scratch shell, overlays, and companions** with Alt+Shift+arrows, like split panes. In the scratch,
  Down makes it higher and Up lower. In an overlay, Right or Down make it bigger, Left or Up smaller. In a
  companion, the arrows move the line between it and its pane. See
  [docs/KEYS.md](docs/KEYS.md#overlays-and-the-scratch-shell).
- **Overlays and a scratch shell** (ideas from agterm). Ctrl+Shift+L runs a command in a box over the active pane:
  a file manager, lazygit, htop. The layout does not change, the pane under it is dim, and the box goes when the
  program ends; Ctrl+Shift+L again hides it and shows it, and the program lives on. Ctrl+Shift+J is the window
  overlay: one for all tabs, for example a Far that you keep (`window_overlay = "far"`). A profile with
  `overlay = true` opens as an overlay too, and `overlay_size` sets the size (80% by default, 100 = the whole pane). Ctrl+` shows or hides the scratch shell: a shell that drops down over the top of
  the tab and keeps its history while it is hidden (`scratch_size`, 50%). From scripts and agents:
  `ftermctl overlay -- COMMAND`, `ftermctl scratch`, `spawn` with `place: "overlay"`, and the MCP tool `open_pane`.
  See [docs/KEYS.md](docs/KEYS.md#overlays-and-the-scratch-shell).
- **Companions:** panes that belong to a pane, for example the log of a server or a second shell. Ctrl+Shift+N runs
  a command (or a shell) as a strip at the bottom of the active pane. Ctrl+Shift+K hides it and shows it again (the
  program lives on), and Ctrl+Shift+M goes to the next one. They close with their pane, and they move and zoom
  with it. `companion_side = "right"` and `companion_size` (40% by default) set the place. From scripts and agents:
  `ftermctl companion -- COMMAND`, `ftermctl companion --toggle|--next`, `spawn` with `place: "companion"`, the API
  method `companion`, and the MCP tool `open_pane`. See [docs/KEYS.md](docs/KEYS.md#companions).
- **The Agents panel tells what each agent does.** The agents that wait for you come first, with their real
  question; a working one shows its todo ("1/3 · Writing tests"), the cost of its session, and the context used.
  Space opens the details (todos, cost, lines, context, model, the 5 h and 7 d limits, the changed files, and the
  steps), D shows the changes of its folder in an overlay, and H lists the past conversations of the folder
  (fterm's own list and the files of Claude Code) and brings one back with `claude --resume`. It comes from new
  hooks (`ftermctl agent-event --hook`) and a status line wrapper (`ftermctl statusline`, your own status line
  stays): run "Install Claude Code hooks" again. In the config: `agents = { show = ..., history = ... }`,
  `on_agent_row` (your own row), and `on_agent_event` (react to every step, for example a warning over $5).
- **Each Claude conversation comes back in its own pane.** A new `SessionStart` hook tells fterm the conversation
  of each pane, so a restored session gives each pane `claude --resume <id>`. Before, every Claude pane of a
  folder got `claude --continue` and so the same, newest conversation. Run "Install Claude Code hooks" again to
  get the hook. See [docs/CLAUDE.md](docs/CLAUDE.md).
- **An overlay can wait.** `ftermctl overlay --wait -- lazygit` shows the program to the user and ends with its
  exit code when it is closed (also `wait` in the API `spawn` and in the MCP tool `open_pane`). Through
  PowerShell an overlay now gives the real exit code of its program, not 1.
- **Agents can set up fterm for you.** The agent guide tells how to change `fterm.lua` safely (keys, commands,
  dashboard tiles), and `list_panes` gives the path of the config. Ask Claude "add a dashboard tile with my
  failed runs".
- **The dashboard** (Ctrl+Shift+D): tiles over the window, in a grid. A tile shows a pane (its state, its last lines,
  and a chart of its output), the output of any command (for example `gh pr list`, run again every minute), or
  buttons that run a command or an action (for example a publish workflow, with `confirm = true`). You choose the
  tiles in `dashboard` in `fterm.lua`; by default it shows the agents and the busy panes. Keys 1–9 go to a pane,
  a–z press buttons, Esc hides it. A command tile is also a list: Enter goes into it, the arrows choose a line,
  and your `actions` work on it: open its link, run a program (with the parts of the line as its arguments, never
  through a shell), or call a Lua function. So the pull requests that wait for your review can show, open, show
  their diff, and be approved from there. `fterm.run` and `fterm.open` are new in Lua. Scripts and agents add their own tiles with `ftermctl dashboard-tile`, the API
  method `dashboard_tile`, and the MCP tool. See [docs/KEYS.md](docs/KEYS.md#the-dashboard) and
  [docs/CONFIG.md](docs/CONFIG.md#the-dashboard).
- **Recent panes.** Alt+` goes back to the pane that you used before (in any tab or workspace), and again comes
  back. Ctrl+Shift+H shows all panes, the recently used first, with their folder and agent state; type to find
  one. A pane counts when you stay in it for a second, so a quick walk through tabs does not count.
  See [docs/KEYS.md](docs/KEYS.md#recent-panes).
- **Pictures in panes.** A picture (PNG, JPEG, BMP) takes a rectangle of cells in any pane and scrolls with
  the text. `ftermctl image cat.png` (`--height N`, 12 rows by default) shows one in the output of its pane; it
  works in WSL and over SSH too. Programs can show pictures with the iTerm2 sequence (OSC 1337). A picture pane
  (the picture fills the pane): `ftermctl show-image FILE`, the API method and the MCP tool `show_image` (so an
  agent can show you a picture), and the palette actions "Show the picture in the clipboard" and "Show a
  screenshot of this pane". `pictures = "pixels" | "braille" | "off"` in `fterm.lua`. On Windows a program must
  make room for its picture with new lines (ConPTY does not know about pictures); `ftermctl image` does it. See
  [docs/PICTURES.md](docs/PICTURES.md).
- **Mark up a picture.** In a picture pane you can draw on the picture: a pen (`P`), boxes (`R`), arrows (`A`), text
  (`T`), hidden parts with big pixels (`H`, for secrets), and a crop (`C`); `1`–`4` pick the color, Ctrl+Z / Ctrl+Y
  undo and redo. Enter saves the picture as PNG in `data_dir/shots`, puts it into the clipboard, and pastes its
  path into the pane next to it (for example the prompt of an agent). An agent can ask you to mark up a picture
  and wait for it: `show_image` with `question` and `wait` (also `ftermctl show-image --wait`).
- **Panes that live on (a mux server).** With `mux = true` in `fterm.lua` (or in a profile), the programs of new
  panes run in a small background server (`fterm --mux-server`). Close the window, and they go on running; open
  fterm again, and the panes come back in the same tabs, with their screen, their history, and the programs
  still running. Closing such a pane (or its tab) ends its program. `ftermctl mux list|stop` and the palette
  action "Stop the mux server" end them all. See docs/MUX.md.
- **Panes on other machines (SSH).** `remotes = { { name = "prod", host = "me@prod" } }` in the config adds
  "Connect: prod" to the palette (and "Connect over SSH…" is there for any host). The programs run in a mux server
  on that machine (`ftermctl mux serve`, reached with `ssh ... ftermctl mux proxy`), so they go on when the
  connection ends, and "Connect" brings them back. Only ftermctl is needed there.
- **The UI in your language.** All texts of the window (the palette, toasts, questions, the dock, the Review tab,
  the AI panel, the history lists, the window title) come from a language file. English is built in; another
  language is a JSON file `l10n/<code>.json` next to `fterm.lua`, chosen with `language = "ru"` (or
  `"system"`). A text that the file does not have stays English, and plural forms follow the rules of the
  language. The archive has `l10n/en.json` to start a translation. See docs/L10N.md.

- **Workspaces.** A workspace is a group of tabs: the tab bar shows the tabs of one workspace, and the programs in
  the others go on running. Ctrl + Alt + W (or a click on the workspace name at the left of the tab bar) opens the
  list: go to a workspace, or type a new name for a new one. The palette can move a tab to another workspace and
  rename a workspace. A dot on the name shows an agent that waits in another workspace. A profile can open its tabs
  in a workspace (`workspace = "agents"`), sessions keep the workspaces, and `list_panes` tells the workspace of
  each tab. See docs/KEYS.md.
- **Links with the keyboard, and OSC 8 links.** Ctrl + Shift + U puts a label on every link of the screen: press a
  label to open its link, or Shift + the label to copy it. Hold Ctrl and move the mouse: the link under it is
  underlined. OSC 8 links (a text with a hidden URL) work too. fterm opens only http, https, ftp, mailto, and file
  links, and asks first when a link is a file or its text does not show its URL. See docs/KEYS.md.

### Changed
- **Many tabs are faster.** When the window, the dock, or a split changes its size, only the panes on the screen
  get the new size at once. The panes of hidden tabs get it a moment later, a few at a time. Before, each step of
  a drag was slow when many tabs had a long history.
- **Less work when nothing happens.** Each tab woke up five times a second for no reason. Now it sleeps until
  there is output.
- **Braille in panes is round dots now.** The spinners of agents (opencode, and others) are Braille chars, and as
  square pixels they looked like little blocks. Panes with programs now draw Braille as round dots, like a font;
  scenes always draw it as square pixels. `braille_style = "pixels"` brings the pixels back to all panes. If your
  `fterm.lua` came from an older sample, it may have `braille_style = "pixels"`: remove that line for dots.
- **The Review tab for plans of Claude Code plan mode opens only when you want it.** fterm asks first:
  R opens the Review tab, Esc (or no answer in 2 minutes) gives the plan dialog of Claude (there Ctrl+G edits
  the plan in your editor). `plan_review = "always"`
  or `"never"` in the config changes this, and the palette action `plan_review_mode` changes it while fterm runs.
  An agent or a script that asks for a review itself (`review_plan`, `ftermctl review`) still gets the tab.

### Fixed
- On Linux and macOS the folder history (Alt+F12) was empty, and the command history did not show commands with
  a folder. fterm took every folder that starts with `/` for a WSL folder. Now all folders show.
- A new `fterm.lua` is read at once. Before, when the config folder did not exist at the start, fterm did not see
  a config that you made later, until you started it again. Now fterm makes the folder at the start.
- The box of "Connect over SSH…" (and the new command boxes of overlays and companions) now moves its cursor:
  the arrows, Home, End, and Delete edit the text, and Ctrl+V (or Shift+Insert) pastes. The box that renames a
  workspace too. Before, you could only type
  at the end and use Backspace.
- **Many tabs with agents do not slow fterm down.** The spinner of an agent changes the title of its pane about 10
  times a second, and each change in any tab drew the whole window again. Now a change that you do not see (another
  tab) is drawn with the next frame, at most once a second (`background_redraws`), and the window title is set only
  when it changes. A split pane next to the active one now shows its new output at once too.
- **No stray char when the window gets the focus back.** A key that you let go in another window (for example `/`)
  could be typed into the pane when you clicked on fterm again. fterm now skips the key presses that the window
  system sends again on focus.
- **A restored pane comes back in its folder also when its shell never told fterm the folder.** Before, such a
  pane (no shell integration, or a shell that something else started, for example a fterm started from ConEmu)
  was saved with no folder and came back in the folder of the fterm process. Now fterm keeps the folder that the
  pane started in, until the shell tells its own one. A new pane next to it starts there too.

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
