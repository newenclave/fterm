# fterm for agents

You run in **fterm**, a terminal with tabs and split panes. fterm lets you see and use its panes:
run commands in other panes, read their output, wait for events, talk to agents in other panes,
send notifications to the user, and draw charts in a "scene" pane.

## Are you in fterm?
- `FTERM_PANE_ID` is set: it is the id of your own pane.
- `FTERM_SOCKET` is set: `ftermctl` (and `ftermctl mcp`) finds this window with it.
- `TERM_PROGRAM` is `fterm`.

There are two ways in. Both do the same things:
- **MCP tools** (when `ftermctl mcp` is an MCP server of your client): `list_panes`, `run_command`, …
- **The `ftermctl` command** (any agent with a shell): `ftermctl list`, `ftermctl run --wait …`.
  On Windows and in WSL it is `ftermctl.exe`. `ftermctl --json …` gives JSON.

## Panes
Every pane has an id. `list_panes` (`ftermctl list`) shows all tabs and panes: the program, the folder,
whether a command runs, the last command and its exit code, and the agent state.
**No pane id = your own pane.** Most tools take `pane` for another one.

## Recipes

**Run tests or a build in another pane, and read the result**
1. `open_pane` with `place: "right"` (`ftermctl spawn --right`) gives a new pane id.
2. `run_command` with `pane` and `command` (`ftermctl run --pane N --wait "cargo test"`) waits for the end
   and gives the exit code and the output.

The user sees the command run. This is better than a hidden command when the user wants to watch it.

**A long command (a server, a watcher)**
- Start it with `send_text` (`enter: true`): it does not wait.
- Then `wait_for` with `event: "text"` and a `pattern` (for example `"Listening on"`), or with
  `event: "command_done"`. Do not ask again and again in a loop: `wait_for` waits for you.
- `read_pane` with `what: "last_output"` gives the output of the last command; `"screen"` gives what the
  user sees now; `lines` keeps it short.

**Talk to the agent in another pane**
- `send_message` with `to` (its pane id) and `text`. Nothing is typed into that pane: the message goes to
  its inbox, and the user sees a toast.
- The other agent reads it with `read_messages`, or waits for it with `wait_for` and `event: "message"`.
- To wait for its answer, use `wait_for` with `event: "message"` on your own pane.
- `wait_for` with `event: "agent_done"` or `"agent_waiting"` waits for the agent in a pane to finish or to
  need the user.

**Tell the user something**
- `notify` with `title`, `body`, and `level` (`info`, `success`, `warning`, `error`, `attention`).
  Use `attention` only when the user must act.

**Show a chart or a picture**
1. `open_scene` (`ftermctl scene`) opens a scene pane. It gives the pane id, the size in dots, and the
   `aspect` (dot height / dot width). Each cell is 2×4 dots.
2. `plot` with `pane` and `values` draws a chart (a line, or `bars: true`), scaled to the scene, with
   `color`, `title`, `min`, and `max`. Call it again with new values for a live chart.
3. `draw_scene` draws anything: `clear`, `color`, `dot`, `line`, `rect`, `circle`, `text`, and `plot`.
   `x`, `y` are dots from the top left; `text` uses cells (`col`, `row`). All commands of one call are drawn
   at once.

The scene draws itself again when the user zooms or resizes it. To use the new size (for example more
points in a chart), `wait_for` with `event: "scene_resized"` and draw again.

**Other tools**
- `focus_pane` shows a pane to the user (its tab, and the keyboard goes there). Use it when the user
  should look at something now, not for your own work.
- `set_title` sets the title of the tab of a pane, for example "tests" or "server".
- `close_pane` closes a pane (see the rules).

## Rules
- **Ask the user before you close a pane** where something runs (`close_pane` with `force`), or a pane
  that you did not open.
- Typing into a pane that you did not open, or reading it, can make fterm ask the user first. Use panes
  that you opened (`open_pane`, `open_scene`) when you can.
- Use `wait_for`, not a loop of `read_pane` calls.
- Keep the output small: `lines` in `read_pane`, and `last_output` instead of the whole history.
- Do not type secrets into panes, and do not send them in messages.
- Commands run in the shell of that pane (PowerShell, bash, …): write them for that shell.
  `list_panes` shows the program of each pane.

## ftermctl, short
| What | ftermctl |
|---|---|
| All panes | `ftermctl list` |
| A new pane | `ftermctl spawn [--right\|--down] [--profile P] [--cwd DIR]` |
| Run and wait | `ftermctl run --pane N --wait "COMMAND"` |
| Type text | `ftermctl send-text --pane N [--enter] TEXT` |
| Read a pane | `ftermctl get-text --pane N [--last-output] [--lines 50]` |
| Wait | `ftermctl wait-for --pane N EVENT [--pattern TEXT] [--timeout S]` |
| A message | `ftermctl send-message N TEXT`, `ftermctl read-messages` |
| A notification | `ftermctl notify TITLE --body TEXT --level info` |
| A scene | `ftermctl scene`, `ftermctl draw --pane N JSON` (or `-` for stdin) |

`ftermctl help` lists everything. The docs for people are in the fterm repository: `docs/MCP.md`,
`docs/API.md`, and `docs/SCENE.md`.
