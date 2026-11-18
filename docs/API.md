# The fterm API

Programs can control a running fterm window: open panes, type into them, read their text, and listen to events.
Agents (Claude Code, OpenCode), scripts, and `ftermctl` use it. MCP is in [MCP.md](MCP.md).

## How to connect
- The API is **JSON-RPC 2.0**. One message is one line of JSON (it ends with `\n`).
- The transport is a local socket:
  - Windows: a named pipe `\\.\pipe\fterm-<user>-<pid>`. Only you (and the system) can open it.
  - Linux and macOS: a unix socket in a folder that only you can read.
- **Every pane has `FTERM_SOCKET`** (the socket of its window) and `FTERM_PANE_ID` (its own id).
  A program in a pane uses them to find "its" window. fterm does not trust `FTERM_PANE_ID`: it finds the pane of
  a program itself (see the next part).
- Outside of fterm, each window writes `%LOCALAPPDATA%\fterm\instances\<pid>.json` with its socket.
- Turn the API off with `api = { enabled = false }` in the config.

## Who may do what
**Your pane.** Any program can set `FTERM_PANE_ID`, so fterm checks the pane itself. It looks at the process of
the client and at its parent processes. The nearest one that is the shell of a pane of this window gives the pane.
`hello` answers this pane (`pane`) and `"proof": "process"`. When fterm finds no pane, the answer has
`"pane": null`: then fterm can not tell who you are, and the `pane` that you said is only the default pane of
your calls. **A program in WSL has no pane:** a program in WSL can choose which `wsl.exe` starts a Windows program
(`WSL_INTEROP`), also the `wsl.exe` of another pane, so fterm does not trust that way.

- `hello`, `list`, `focus`, `zoom`, `set_title`, `set_tab_color`, `themes`, `set_theme`, `notify`, `panel`, `send_message`, `ai_input`, `ai_stop`,
  and the events: everybody.
- Reading or typing into **your own pane** (the pane that fterm found) or a pane that you opened with `spawn`: yes.
- Reading or typing into **another pane** (`send_text`, `get_text`, `screenshot`, `close`, `wait_for`, `read_messages`),
  `spawn`, and the AI chat (`ai_read`, `ai_ask`, `ai_clear`; the chat has text of the panes): fterm asks the
  user (see "The question" below). A client with no pane asks for every pane.
- A **dialog** (`review`, `form`) or a **picture** (`show_image`) at **another pane** (`pane` is not your own
  pane) needs the same yes. Over another program, a dialog could look like that program asks for something
  (a password). A pane that you opened with `scene_open` or `show_image` is not your own here. With no `pane`,
  it opens at your own pane: no question. A client with no pane gets a new tab in the back.
- The hooks of the agents (`agent_event`, `agent_session`, `agent_usage`) never ask. fterm drops them, with no
  word, when they come from a client with no pane.
- A plan of plan mode (`review` with `plan_mode`) from a client with no pane is `skipped`: the agent shows its
  own dialog. One pane has one plan question at a time.
- A pane with **remote control off** cannot be read or typed into by any other client.
  Turn it off and on with the action `toggle_remote_control` (in the command palette).
- `ai_ask` uses the user's AI key (and money). It works only with `ai = { api_access = true }` in the config;
  else it is an error `-32001`.

### The question
fterm asks in a box at the top of the panes:

> A program that calls itself "claude" wants to read and type into other panes.
> fterm checked: it runs in tab 2, under claude.exe.
> It can see their text and run commands there.
> **[Allow]** **[Always for this pane]** **[No]**

- The first line has the name from `hello`. A program can give any name. The second line, in another color, is
  what fterm found itself: the tab (and its workspace) and the program, or "it is not in a pane of this window".
  The program is the first program below the shell of the pane that is not a shell itself.
- **The box does not take your typing.** Your keys go to your pane as before. To answer, press
  **Ctrl+Shift+Enter**, wait a moment, then press a key: **Y** or **1** = allow, **A** or **2** and then
  **Enter** = always, **N**, **3**, or **Esc** = no. A key in the first 0.3 seconds, another key, or 5 seconds
  with no key ends this with no answer (so typing on after Ctrl+Shift+Enter, a new line in some programs, answers
  nothing). Shift with a digit is no answer. (In the AI panel, Ctrl+Shift+Enter is a key of the panel: click a
  button there.)
- Or click a button. A click counts only one second after the box comes, changes, or moves (the dock opens, the
  window or the font changes size), and after the window comes back to the front.
- **Allow** is for this connection.
- **Always for this pane** is for this program in this pane, until the program or the pane ends, or fterm closes.
  Only a client with a pane gets it.
- **No**: for one minute, the next questions of the same pane get an error at once, and so do the questions of
  all clients with no pane (a program could start a helper that fterm can not find in a pane). An agent should
  ask the user before it tries again.
- Your call waits while the question is open (up to two minutes).

### Event hooks of the config
`on_agent` and `on_agent_event` in `fterm.lua` run when a program does something, not when you press a key. When
they type into a pane, open a pane, run a program, or run an action, fterm asks first ("The on_agent hook in your
config wants to type into a pane or start a program"). Before your yes, these calls do not run. The yes counts
for the next events, until the config changes; a No counts for one minute. Toasts, tab colors, and the clipboard
need no yes.

### Turn the questions off
`api = { ask = false }` in the config: no questions (only "remote control off" still stops clients). A program
that can write `fterm.lua` could do this too. So when the config turns a guard off (`api.ask = false`, or
`ai.api_access = true`), fterm keeps the old value and shows a warning in yellow:
- Only a click on **Turn the guard off** turns it off.
- **Keep the guard** (or N, 3, or Esc after Ctrl+Shift+Enter) keeps it. The same value does not ask again; change
  the value to ask again.
- fterm keeps your answers in `guards.json`, in `%LOCALAPPDATA%\fterm` on Windows and in `~/.local/state/fterm`
  (or `$XDG_STATE_HOME/fterm`) on Linux and macOS. It is never in `data_dir`: the config sets that folder. With no
  file, the guards are on.
- While the API asks no questions, the tab bar shows **no API questions**.

### What the guard does not do
It is a guard against mistakes, and against agents and scripts that use ftermctl, MCP, or the API as they are.
It does not stop a program that works hard to get around it: a program that reads other processes, makes a fake
parent process or a fake process id, ends a process and waits until a new one of its own gets the same id, sends
keys, writes `guards.json`, or uses the mux socket. `fterm.lua` is code that you trust, like your `.bashrc`: the
functions of your keys and commands never ask, and language files next to it change the texts of the boxes.

Known gaps:
- `subscribe` gives the events of all panes. `list` shows the commands of all panes. `focus` and `zoom` need no yes.
- The mux socket takes input for any mux pane (see [MUX.md](MUX.md)).
- Another terminal (VS Code, WezTerm) that you start from a pane counts as that pane. So does a program that a tool
  in that pane starts (a watcher, a build script).
- On Linux and macOS with no `XDG_RUNTIME_DIR`, the instance files are in the temp folder.
- A program in tmux or screen, or one started with `nohup` or `setsid`, often has no pane: it asks, and its agent
  hooks do nothing. The same is true for every program in WSL (see "Your pane").

## ftermctl
`ftermctl` is a small console program next to `fterm.exe` (fterm.exe is a window program, so a shell does not
wait for it and does not see its output). It finds the window by `FTERM_SOCKET`, by `--window <pid>`,
or it takes the newest fterm window. Without `--pane`, a command uses your own pane (the pane that fterm
found), else the active pane.

```
ftermctl list                                  # all tabs and panes
ftermctl spawn --right                         # a split on the right; prints the new pane id
ftermctl run --pane 3 --wait cargo test        # type it, wait for the end, print the output,
                                               # and exit with the exit code of cargo
ftermctl get-text --pane 3 --last-output       # the output of the last command
ftermctl get-text --pane 3 --styled            # the screen with colors and styles, as JSON
ftermctl send-text --pane 3 --enter git status
ftermctl theme "Catppuccin Latte"              # all colors of the window ("ftermctl theme" lists them)
ftermctl tab-color --pane 3 "#f38ba8"         # a red line at the top of its tab ("none" = no color)
ftermctl screenshot --pane 3 shot.png          # a PNG of the pane; prints the full path
ftermctl wait-for --pane 3 text --pattern "ready" --timeout 60
ftermctl notify "Deploy is done" --level success
ftermctl send-message 3 "please review the diff"
ftermctl read-messages                         # the messages of your own pane
ftermctl ai read --last 2                      # the last question and answer of the AI panel
ftermctl ai ask --pane 3 --wait "why did it fail?"   # with the last command of pane 3; prints the answer
ftermctl subscribe command_done agent_state    # events as JSON lines
ftermctl call list                             # any method with JSON params
ftermctl --json list                           # JSON for scripts
```

`ftermctl help` shows all commands. Exit codes: 0 = good, 1 = an error from fterm, 2 = bad arguments;
`run --wait` gives the exit code of the command; `ai ask --wait` gives 1 when the answer failed.

## Methods
A missing `pane` means: the pane of the client (the one that fterm found, else the one in `hello`), else the
active pane.

| Method | Params | Answer |
|---|---|---|
| `hello` | `name` (shown in the question; one line, at most 32 cells), `pane` (your `FTERM_PANE_ID`: only a hint) | `api_version`, `fterm`, `pid`, `methods`, `events`, `pane` (the pane that fterm found by your processes, or `null`), `proof` (`"process"`, or `null`). The first `hello` of a connection counts: a later one can only add a pane that fterm did not know before. |
| `list` | | `config` (the path of the config file); each pane's `agent` has `state`, `message`, `question`, `todos` (`done`, `total`, `current`), `cost`, and `context`; `workspace` (the one in use); `tabs` (of all workspaces): each with `tab` (its number in its workspace), `workspace`, `title`, `active`, `zoomed`, `panes` (each pane has `workspace` too) |
| `spawn` | `place`: `"tab"` (default), `"right"`, `"down"`, `"overlay"`, or `"companion"`; `profile`; `cwd`; `pane` (split next to it; default: your own pane, else the active one; with `"overlay"`: the pane to cover); with `"overlay"`: `command` (the shell runs it; the box goes when it ends), `size` (percent), and `wait: true` (the answer comes when the program ends, with `exit`, its exit code; `timeout_ms`, 24 hours at most); `"companion"`: a strip of `pane` (default: your own pane, else the active one) that runs `command` (no command = a shell) | `pane`. A new tab or split does not take the keys of the user: they stay in their tab and pane (`focus` shows a pane to them). |
| `scratch` | `show`: `true` / `false` (no `show` = the other state) | `pane`, `shown` |
| `agent_event` | `kind` (`prompt`, `tool`, `question`, `done`); `text`, `tool`, `file`, `command`, `todos` (`content`, `status`, `active`), `session_id`, `cwd`; `pane` (default: your own) | `pane`. A step of the agent in the pane: the Agents panel shows it, and the state of the agent follows it. The hooks call it (`ftermctl agent-event --hook`). |
| `agent_usage` | `cost`, `lines_added`, `lines_removed`, `context` (percent), `model`, `five_hour`, `seven_day`; `pane` (default: your own) | `pane`. The numbers of the status line of Claude Code (`ftermctl statusline`). |
| `agent_session` | `session_id` (letters, digits, `-`, `_`); `pane` (default: your own) | `pane`. The conversation of the agent in the pane: a restore brings it back (`claude --resume`). The SessionStart hook calls it (`ftermctl agent-session --hook`). |
| `dashboard` | `show`: `true` / `false` (no `show` = the other state) | `shown` |
| `dashboard_tile` | `id`; `title`, `status`, `state` (`working`, `waiting`, `done`, `error`, `idle`), `lines` (texts), `values` (numbers for a chart); `remove: true` takes it away. The same `id` changes the tile. Text and numbers only (at most 32 tiles, 50 lines, 500 values) | `tiles` |
| `companion` | `action`: `"toggle"` (hide or show) or `"next"`; `pane` (default: the active pane; its companions) | `pane` (the shown companion or `null`), `shown` |
| `send_text` | `pane`, `text`, `enter` (true = press Enter after the text) | |
| `get_text` | `pane`, `what`: `"screen"` (default), `"history"`, or `"last_output"`; `lines` (default 200, at most 10000); `styled` (default false) | `text`; for `last_output` also `command`, `exit`, `running`; with `styled` also `fg`, `bg`, `lines` (see below) |
| `focus` | `pane` | |
| `close` | `pane`, `force` (needed when a program runs in it) | |
| `zoom` | `pane` | |
| `set_title` | `pane`, `title` (the title of its tab; empty = the automatic title) | |
| `set_tab_color` | `pane`, `color`: `"#rrggbb"` or `"#rgb"`; `"none"` or `null` = no color | |
| `review` | `text` (a markdown plan) or `items` (a list of texts); `title`, `from` (default: the client name; another name is shown with the client name, `claude (ftermctl)`), `pane` (the pane that asks; another pane than yours needs a yes), `timeout_ms` (default one hour, at most a day; it counts from the call, also while fterm asks the user for access), `plan_mode` (true = from the Claude Code plan mode hook: `plan_review` in the config says if the tab opens). It waits for the user (see [REVIEW.md](REVIEW.md)). | `decision` (`approved`, `changes`, `cancelled`, or `skipped` when a `plan_mode` review did not open), `feedback` (text for an agent), `items` (`text`, `mark`, `comment`, `edited`, `added`); `timed_out` when the time ran out |
| `form` | the form: `title`, `text`, `fields` (see [FORMS.md](FORMS.md)); `from` (default: the client name; another name is shown with the client name), `pane` (the pane that asks; another pane than yours needs a yes), `timeout_ms` (default 30 minutes, at most a day; it counts from the call). It waits for the user. | `status` (`saved` or `cancelled`), `values` (only when saved: every id and its value; a field that the user did not change comes back exactly as it came), `changed` (the ids whose value changed); `timed_out` when the time ran out |
| `themes` | | `current` (the theme in use), `themes` (all names) |
| `set_theme` | `name` (a theme name), or `theme` (a whole theme as JSON, see [THEMES.md](THEMES.md)). It lasts until fterm closes or the config changes. | `name` |
| `notify` | `title`, `body`, `level` (`info`, `success`, `warning`, `error`, `attention`) | |
| `panel` | `name`: `"events"`, `"agents"`, or nothing (close the dock) | |
| `wait_for` | `pane`, `event`: `"command_done"`, `"agent_done"`, `"agent_waiting"`, `"message"`, `"text"` (with `pattern`), or `"scene_resized"`; `timeout_ms` (default 30000, at most one hour) | the event (for example `command`, `exit`, `took_ms`) |
| `send_message` | `to` (a pane id), `text` (at most 64 KB) | `id` |
| `read_messages` | `pane` (default: yours), `unread_only` (default true), `mark_read` (default true) | `messages`: `id`, `from`, `from_name`, `text`, `time` |
| `scene_open` | `place`: `"right"` (default) or `"down"`; `pane` (split next to it; default: your own pane, else the active one) | `pane`, `cols`, `rows`, `width`, `height` (dots), `aspect` |
| `scene_draw` | `pane`, `ops`: one drawing command or a list (see [SCENE.md](SCENE.md)) | `pane`, `cols`, `rows`, `width`, `height`, `aspect` |
| `show_image` | `data` (a PNG, JPEG, or BMP file in base64) or `file` (its path on the fterm machine); `place`: `"right"` (default) or `"down"`; `pane` (split next to it); `question` (a text on the tool bar); `wait: true` (answer when the user has marked it up; `timeout_ms`, default one hour) | `pane`, `width`, `height` (pixels). With `wait`: `done: true` and `path`, `width`, `height` of the marked-up PNG; or `done: false` (closed; `timed_out: true` when the time is over). See [PICTURES.md](PICTURES.md). |
| `screenshot` | `pane`; `path`: a `.png` file (default: a new file in the temp folder) | `pane`, `path`, `width`, `height` (pixels) |
| `ai_read` | `last`: only the last turns (a question and its answer are 2 turns) | `turns`: each with `role` (`"user"` or `"ai"`), `text`, `streaming`, `error`; `input`, `context`, `running`, `provider`, `model` |
| `ai_ask` | `text`; `pane` (send the last command of this pane and its output too); `wait` (default false); `timeout_ms` (default 300000, at most one hour) | `id`; with `wait`, when the answer has ended: `id`, `text`, `error` |
| `ai_input` | `text`: put it into the input of the AI panel (it is not sent) | |
| `ai_stop` | | `stopped` (false = no answer was running) |
| `ai_clear` | | |
| `subscribe` | `events`: a list of names, or `["*"]` for all | `events` |
| `unsubscribe` | `events` | `events` |

A pane in `list` has: `id`, `tab`, `program`, `title`, `cwd`, `active`, `running` (a command runs),
`at_prompt`, `agent` (`state`, `message`), `last_command` (`command`, `exit`, `took_ms`), `size`,
`remote` (remote control on), `messages` (unread messages).

`get_text` with `styled: true` also gives the colors and styles, as the user sees them. `fg` and `bg`
are the default colors of the pane. `lines` has one list of runs for each line (a wrapped line is one
line). A run is text with one style: `text`, and only what is not the default: `fg`, `bg` (`"#rrggbb"`),
`bold`, `italic`, `underline`, `strike`, `dim`. The colors are final (bold, dim, and inverse are in them).

```json
{ "pane": 1, "text": "ok error", "fg": "#cdd6f4", "bg": "#1e1e2e",
  "lines": [[{ "text": "ok " }, { "text": "error", "fg": "#f38ba8", "bold": true }]] }
```

`screenshot` takes the pane as it is on the screen, with its colors and its border, at the next frame. The
pane must be in the active tab (or zoomed); else it is an error. fterm writes the file itself, so `path`
should be a full path (ftermctl makes a short path full). It works when the window is covered by
other windows too, but not when it is minimized.

The AI methods use the chat of the AI panel (one chat for each window; see [AI.md](AI.md)). The panel does
not open for them. The chat shows a question of `ai_ask` with `(from <client name>)`. `on_ai_request` in the
config sees these questions too. Only one answer runs at a time: `ai_ask` while an answer runs is an error.
Without `wait`, listen to the `ai_answer` event. `ai_clear` stops a running answer, so its `ai_answer` comes
with the error `stopped`.

`wait_for` waits for the **next** event (a text that is on the screen already answers at once).
`last_output` needs shell integration (PowerShell has it by itself; see [CONFIG.md](CONFIG.md)).

## Messages between agents
Every pane has an inbox. `send_message` puts a message there; nothing is typed into the other pane.
The user sees a toast, and the Agents panel shows the count (`claude · ✉ 2`). The agent in that pane
reads them with `read_messages`, or waits for one with `wait_for { event = "message" }`.

## Events
After `subscribe`, the server sends notifications (JSON-RPC messages without an `id`):

| Event | Params |
|---|---|
| `pane_opened` | `pane` |
| `pane_closed` | `pane` |
| `command_done` | `pane`, `command`, `exit`, `took_ms`, `cwd` |
| `agent_state` | `pane`, `state`, `message` |
| `cwd` | `pane`, `cwd` |
| `title` | `pane`, `title` |
| `notification` | `pane`, `title`, `body`, `level`, `source` |
| `message` | `id`, `to`, `from`, `from_name`, `text` |
| `ai_answer` | `id`, `text`, `error` (only when it failed or was stopped) |

## Errors
Errors are JSON-RPC errors: `-32700` bad JSON, `-32600` bad request, `-32601` no such method,
`-32602` bad params, `-32001` not allowed, `-32002` no such pane or tab, `-32003` timeout.

## The future
New methods and new optional params do not break old clients. `hello` says which methods this fterm has.
