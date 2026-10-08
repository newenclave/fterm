# The fterm API

Programs can control a running fterm window: open panes, type into them, read their text, and listen to events.
Agents (Claude Code, OpenCode), scripts, and `ftermctl` use it. MCP is in [MCP.md](MCP.md).

## How to connect
- The API is **JSON-RPC 2.0**. One message is one line of JSON (it ends with `\n`).
- The transport is a local socket:
  - Windows: a named pipe `\\.\pipe\fterm-<user>-<pid>`. Only you (and the system) can open it.
  - Linux and macOS: a unix socket in a folder that only you can read.
- **Every pane has `FTERM_SOCKET`** (the socket of its window) and `FTERM_PANE_ID` (its own id).
  A program in a pane uses them to find "its" window and "itself".
- Outside of fterm, each window writes `%LOCALAPPDATA%\fterm\instances\<pid>.json` with its socket.
- Turn the API off with `api = { enabled = false }` in the config.

## Who may do what
- `hello`, `list`, `focus`, `zoom`, `set_title`, `set_tab_color`, `themes`, `set_theme`, `notify`, `panel`, `send_message`, `ai_input`, `ai_stop`,
  and the events: everybody.
- Reading or typing into **your own pane** (the pane in `hello`) or a pane that you opened with `spawn`: yes.
- Reading or typing into **another pane** (`send_text`, `get_text`, `screenshot`, `close`, `wait_for`, `read_messages`),
  `spawn`, and the AI chat (`ai_read`, `ai_ask`, `ai_clear`; the chat has text of the panes): fterm asks the
  user once for each client:

  > Allow claude (tab 2) to read and type into other panes?
  > Enter = allow, A = always allow claude, Esc = no

  "Allow" is for this connection. "Always" is for this client name until fterm closes.
  Your call waits while the question is open (up to two minutes).
- A pane with **remote control off** cannot be read or typed into by any other client.
  Turn it off and on with the action `toggle_remote_control` (in the command palette).
- `api = { ask = false }` in the config: no questions (only "remote control off" still stops clients).
- `ai_ask` uses the user's AI key (and money). It works only with `ai = { api_access = true }` in the config;
  else it is an error `-32001`.

## ftermctl
`ftermctl` is a small console program next to `fterm.exe` (fterm.exe is a window program, so a shell does not
wait for it and does not see its output). It finds the window by `FTERM_SOCKET`, by `--window <pid>`,
or it takes the newest fterm window. Without `--pane`, a command uses your own pane (inside fterm),
else the active pane.

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
A missing `pane` means: the pane of the client (from `hello`), else the active pane.

| Method | Params | Answer |
|---|---|---|
| `hello` | `name`, `pane` (your `FTERM_PANE_ID`) | `api_version`, `fterm`, `pid`, `methods`, `events`, `pane` |
| `list` | | `tabs`: each with `tab`, `title`, `active`, `zoomed`, `panes` |
| `spawn` | `place`: `"tab"` (default), `"right"`, `"down"`; `profile`; `cwd`; `pane` (split next to it) | `pane` |
| `send_text` | `pane`, `text`, `enter` (true = press Enter after the text) | |
| `get_text` | `pane`, `what`: `"screen"` (default), `"history"`, or `"last_output"`; `lines` (default 200, at most 10000); `styled` (default false) | `text`; for `last_output` also `command`, `exit`, `running`; with `styled` also `fg`, `bg`, `lines` (see below) |
| `focus` | `pane` | |
| `close` | `pane`, `force` (needed when a program runs in it) | |
| `zoom` | `pane` | |
| `set_title` | `pane`, `title` (the title of its tab; empty = the automatic title) | |
| `set_tab_color` | `pane`, `color`: `"#rrggbb"` or `"#rgb"`; `"none"` or `null` = no color | |
| `review` | `text` (a markdown plan) or `items` (a list of texts); `title`, `from` (default: the client name), `pane` (the pane that asks), `timeout_ms` (default one hour, at most a day), `plan_mode` (true = from the Claude Code plan mode hook: `plan_review` in the config says if the tab opens). It waits for the user (see [REVIEW.md](REVIEW.md)). | `decision` (`approved`, `changes`, `cancelled`, or `skipped` when a `plan_mode` review did not open), `feedback` (text for an agent), `items` (`text`, `mark`, `comment`, `edited`, `added`); `timed_out` when the time ran out |
| `themes` | | `current` (the theme in use), `themes` (all names) |
| `set_theme` | `name` (a theme name), or `theme` (a whole theme as JSON, see [THEMES.md](THEMES.md)). It lasts until fterm closes or the config changes. | `name` |
| `notify` | `title`, `body`, `level` (`info`, `success`, `warning`, `error`, `attention`) | |
| `panel` | `name`: `"events"`, `"agents"`, or nothing (close the dock) | |
| `wait_for` | `pane`, `event`: `"command_done"`, `"agent_done"`, `"agent_waiting"`, `"message"`, `"text"` (with `pattern`), or `"scene_resized"`; `timeout_ms` (default 30000, at most one hour) | the event (for example `command`, `exit`, `took_ms`) |
| `send_message` | `to` (a pane id), `text` (at most 64 KB) | `id` |
| `read_messages` | `pane` (default: yours), `unread_only` (default true), `mark_read` (default true) | `messages`: `id`, `from`, `from_name`, `text`, `time` |
| `scene_open` | `place`: `"right"` (default) or `"down"`; `pane` (split next to it) | `pane`, `cols`, `rows`, `width`, `height` (dots), `aspect` |
| `scene_draw` | `pane`, `ops`: one drawing command or a list (see [SCENE.md](SCENE.md)) | `pane`, `cols`, `rows`, `width`, `height`, `aspect` |
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
