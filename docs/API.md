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
- `hello`, `list`, `focus`, `zoom`, `set_title`, `notify`, `panel`, `send_message`, and the events: everybody.
- Reading or typing into **your own pane** (the pane in `hello`) or a pane that you opened with `spawn`: yes.
- Reading or typing into **another pane** (`send_text`, `get_text`, `close`, `wait_for`, `read_messages`)
  and `spawn`: fterm asks the user once for each client:

  > Allow claude (tab 2) to read and type into other panes?
  > Enter = allow, A = always allow claude, Esc = no

  "Allow" is for this connection. "Always" is for this client name until fterm closes.
  Your call waits while the question is open (up to two minutes).
- A pane with **remote control off** cannot be read or typed into by any other client.
  Turn it off and on with the action `toggle_remote_control` (in the command palette).
- `api = { ask = false }` in the config: no questions (only "remote control off" still stops clients).

## ftermctl
`ftermctl` is a small console program next to `fterm.exe`. It finds the window by `FTERM_SOCKET`,
by `--window <pid>`, or it takes the newest fterm window.

```
ftermctl call list
ftermctl call spawn '{"place": "right"}'
ftermctl call send_text '{"pane": 3, "text": "cargo test", "enter": true}'
ftermctl call get_text '{"pane": 3, "what": "history", "lines": 50}'
```

More commands (without JSON) come in the next step.

## Methods
A missing `pane` means: the pane of the client (from `hello`), else the active pane.

| Method | Params | Answer |
|---|---|---|
| `hello` | `name`, `pane` (your `FTERM_PANE_ID`) | `api_version`, `fterm`, `pid`, `methods`, `events`, `pane` |
| `list` | | `tabs`: each with `tab`, `title`, `active`, `zoomed`, `panes` |
| `spawn` | `place`: `"tab"` (default), `"right"`, `"down"`; `profile`; `cwd`; `pane` (split next to it) | `pane` |
| `send_text` | `pane`, `text`, `enter` (true = press Enter after the text) | |
| `get_text` | `pane`, `what`: `"screen"` (default), `"history"`, or `"last_output"`; `lines` (default 200, at most 10000) | `text`; for `last_output` also `command`, `exit`, `running` |
| `focus` | `pane` | |
| `close` | `pane`, `force` (needed when a program runs in it) | |
| `zoom` | `pane` | |
| `set_title` | `pane`, `title` (the title of its tab; empty = the automatic title) | |
| `notify` | `title`, `body`, `level` (`info`, `success`, `warning`, `error`, `attention`) | |
| `panel` | `name`: `"events"`, `"agents"`, or nothing (close the dock) | |
| `wait_for` | `pane`, `event`: `"command_done"`, `"agent_done"`, `"agent_waiting"`, `"message"`, or `"text"` (with `pattern`); `timeout_ms` (default 30000, at most one hour) | the event (for example `command`, `exit`, `took_ms`) |
| `send_message` | `to` (a pane id), `text` (at most 64 KB) | `id` |
| `read_messages` | `pane` (default: yours), `unread_only` (default true), `mark_read` (default true) | `messages`: `id`, `from`, `from_name`, `text`, `time` |
| `subscribe` | `events`: a list of names, or `["*"]` for all | `events` |
| `unsubscribe` | `events` | `events` |

A pane in `list` has: `id`, `tab`, `program`, `title`, `cwd`, `active`, `running` (a command runs),
`at_prompt`, `agent` (`state`, `message`), `last_command` (`command`, `exit`, `took_ms`), `size`,
`remote` (remote control on), `messages` (unread messages).

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

## Errors
Errors are JSON-RPC errors: `-32700` bad JSON, `-32600` bad request, `-32601` no such method,
`-32602` bad params, `-32001` not allowed, `-32002` no such pane or tab, `-32003` timeout.

## The future
New methods and new optional params do not break old clients. `hello` says which methods this fterm has.
