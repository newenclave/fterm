# fterm and MCP

With MCP, an agent (Claude Code, OpenCode, and other MCP clients) gets tools to work with fterm:
open panes, run commands there and read the result, wait for events, and send messages to other agents.

## How it works
MCP clients start an MCP server as a program and talk to it on stdin and stdout.
For fterm this program is `ftermctl mcp`. It talks to the fterm window over the local API (see [API.md](API.md)).

```
Claude Code ──stdio (MCP)──> ftermctl mcp ──named pipe (JSON-RPC)──> the fterm window
```

`ftermctl mcp` runs inside the pane of the agent, so it knows its window (`FTERM_SOCKET`) and its own pane
(`FTERM_PANE_ID`). Tools without a `pane` use the agent's own pane.

## Set it up
Claude Code (once, for all projects):

```
claude mcp add --scope user fterm -- ftermctl mcp
```

Inside fterm this just works: fterm puts its own folder (with `ftermctl.exe`) at the end of `PATH` in every pane.
To use it in Claude Code that runs outside of fterm, put the folder of `fterm.exe` into your `PATH`, or give the full path:

```
claude mcp add --scope user fterm -- C:\path\to\ftermctl.exe mcp
```

**Claude Code in WSL** (in a WSL pane of fterm): Linux cannot open the Windows pipe, but WSL can start
`.exe` files, and fterm gives WSL panes `FTERM_SOCKET` and `FTERM_PANE_ID` (with `WSLENV`). So use the `.exe` name:

```
claude mcp add --scope user fterm -- ftermctl.exe mcp
```

Other MCP clients: the command is `ftermctl` with the argument `mcp`, transport stdio.

## The tools

| Tool | What it does |
|---|---|
| `list_panes` | All tabs and panes: ids, programs, folders, agent states, the last command and its exit code. |
| `open_pane` | A new tab, or a split on the right or below (`place`, `profile`, `cwd`, `next_to`). Gives the new pane id. |
| `run_command` | Types a command into a pane, waits for its end, and gives the exit code and the output. |
| `send_text` | Types text into a pane (for example an answer to a program). |
| `read_pane` | The screen, the history, or the output of the last command of a pane. |
| `wait_for` | Waits for an event: `command_done`, `agent_done`, `agent_waiting`, `message`, a `text` on the screen, or `scene_resized` (a scene got a new size). |
| `notify` | A notification for you. |
| `send_message` | A message to the agent in another pane (it goes to the inbox of that pane). |
| `read_messages` | The messages that other agents sent to this pane. |
| `focus_pane` | Shows a pane to you. |
| `close_pane` | Closes a pane (`force` when a program runs in it). |
| `set_title` | Sets the title of the tab. |
| `open_scene` | Opens a Braille scene (a pane to draw into) on the right or below. Gives its id, its size in dots, and the aspect of the dots. |
| `draw_scene` | Draws commands into a scene: dots, lines, rects, circles, text, colors, charts (see [SCENE.md](SCENE.md)). |
| `plot` | A chart of numbers in a scene, as a line or bars, with a title and a color. Call it again with new values for a live chart. |

The server also gives the agent short instructions: the main recipes (run tests in a pane, wait for
an event, talk to another agent, draw a chart) and the rules (for example: ask the user before you close
a pane). The full guide for agents is `ftermctl guide` (the same text is in
[assets/agents/GUIDE.md](../assets/agents/GUIDE.md)); agents with no MCP can read it too. For Claude Code there is also a skill with the same guide
(see [CLAUDE.md](CLAUDE.md#the-fterm-skill)).

Ask Claude for example: "open a scene and plot the time of each test run", or "draw the module graph of this
project in a scene".

## Safety
The first time an agent wants to read or type into a pane that is **not its own** (or opens a new pane),
fterm asks you: Allow, Always (for this agent name until fterm closes), or No.
A pane with remote control off (`toggle_remote_control` in the command palette) is never used by agents.
See [API.md](API.md#who-may-do-what).

## Examples
Ask Claude in fterm:

- "Open a pane on the right and run the tests there. Tell me which tests failed."
  Claude uses `open_pane` and `run_command`, and reads the output.
- "Start the dev server in a new tab and wait until it says `ready`."
  `open_pane`, `send_text`, then `wait_for` with `text` and the pattern `ready`.

**Two Claude sessions that work together.** Start Claude in two panes (for example pane 1 and pane 2).
Tell the first one: "When you finish a change, send a message to pane 2 with what to review."
Tell the second one: "Wait for messages and review the changes." It uses `wait_for` with `message`,
then `read_messages`. You see each message as a toast, and the Agents panel shows the count.
