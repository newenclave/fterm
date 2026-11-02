# Keys and mouse

These are the default keys. You can change them in the config (see [CONFIG.md](CONFIG.md)).
**Ctrl + Shift + P** opens the command palette. **Ctrl + Shift + ,** opens the config file. **Ctrl + Shift + F5** reads the config again (fterm also does it by itself when you save the file).

## Command palette
Ctrl + Shift + P shows all actions, all profiles ("New tab: Claude", "Split right: Claude", …),
and the `commands` from your config. Type a few letters to filter (for example `spl cl`).
Up / Down / PageUp / PageDown or the mouse wheel select a line, Enter runs it, Esc closes the palette.
The key of each action is on the right.

## Tabs
| Key or mouse | What it does |
|---|---|
| Ctrl + Shift + T, click `+` | New tab (after the active tab). |
| Click `×`, middle click on a tab | Close the tab (all its panes). If a program runs in it, fterm asks first. |
| Ctrl + Tab, Ctrl + PageDown | Next tab. |
| Ctrl + Shift + Tab, Ctrl + PageUp | Previous tab. |
| Ctrl + Shift + 1 … 8 | Go to tab 1 … 8. |
| Ctrl + Shift + 9 | Go to the last tab. |
| Ctrl + Shift + PageUp / PageDown | Move the tab left / right. |
| Ctrl + Shift + R, double click on a tab | Rename the tab. Enter = save, Esc = cancel. An empty name = the automatic title again. |
| Mouse wheel on the tab bar | Next / previous tab. |

The tab title is your name for the tab, or the title from the program, or the program name.

## Workspaces
A workspace is a group of tabs, for example "fterm", "work", and "agents". The tab bar shows only the tabs of the
workspace in use. The programs in the other workspaces go on running. The first workspace is `main`.

| Key or mouse | What it does |
|---|---|
| Ctrl + Alt + W, click the name at the left of the tab bar | The list of workspaces. Enter goes to the chosen one. Type a new name and press Enter: a new workspace with a new tab. |
| Palette → Move tab to workspace… | The same list: Enter moves the active tab there (or to a new workspace). |
| Palette → Next workspace / Previous workspace | Go to the next / the previous workspace. |
| Palette → Rename workspace | A new name for the workspace in use. |

With more than one workspace, the name of the workspace in use is at the left end of the tab bar. A dot on it
means that an agent in another workspace waits for you (yellow) or failed (red). The tab keys (next, previous,
Ctrl + Shift + 1 … 8) work inside the workspace. When you go to a pane from a toast or the Agents panel, fterm goes
to its workspace. When the last tab of a workspace closes, the workspace goes away. The session keeps the
workspaces. To open the tabs of a profile in a workspace, give it `workspace = "agents"` (see CONFIG.md).

## The window
| Key or mouse | What it does |
|---|---|
| The window ×, Alt + F4 | Close fterm. When a program or an agent runs, fterm asks first (Enter = close, Esc = stay). See `confirm_close` in [CONFIG.md](CONFIG.md). |
| Ctrl + Shift + ] / [ | The colors of programs closer to the theme, or closer to their own (`harmonize`, steps of 0.1; see [THEMES.md](THEMES.md#the-colors-of-programs-harmonize)). |
| Alt + Enter | Full screen on or off: the window takes the whole monitor, with no frame and no title bar (like WezTerm). Programs do not get Alt + Enter then; to give it back to them, turn the key off: `{ key = "alt+enter", action = "none" }` (see [CONFIG.md](CONFIG.md)). |
| Ctrl + Shift + S | The list of sessions: named ones (★) and the last closed windows. Enter = restore, Delete = forget. "Save session as…" is in the palette. See [CONFIG.md](CONFIG.md#sessions). |

## Split panes
| Key or mouse | What it does |
|---|---|
| Alt + Shift + `=` | Split the active pane: a new pane on the right. |
| Alt + Shift + `-` | Split the active pane: a new pane below. |
| Alt + arrows, click in a pane | Go to the pane on the left / right / above / below. |
| Alt + Shift + arrows | Move the nearest line of the active pane (one cell per press). |
| Drag the line between panes | Make panes bigger or smaller. The mouse cursor shows arrows over the line. |
| Ctrl + Shift + Z | Zoom: the active pane takes the whole tab. Press again to see all panes. |
| Ctrl + Shift + W | Close the active pane. The last pane closes the tab. If a program runs, fterm asks first. |

Each pane has its own shell, history, selection, and copy mode. The active pane has a colored frame.
When a shell ends (for example, you type `exit`), only its tab closes. The window closes after the last tab.

## Recent panes
| Key | What it does |
|---|---|
| Alt + ` | Go back to the pane that you used before, in any tab or workspace. Press again to come back. |
| Ctrl + Shift + H | The list of all panes, the recently used first. Type to find one, Enter goes there. |

A pane counts as used when you stay in it for a second. So a quick walk through tabs with Ctrl + Tab does not
change where Alt + ` goes. Overlays and the scratch shell are not in the list: they have their own keys.

## The dashboard
Ctrl + Shift + D shows the **dashboard** over any tab: a grid of tiles. A tile shows a pane (its state, its last
lines, and a chart of its output in the last 5 minutes), the output of a command, buttons, or a tile from a script
or an agent. The frame of a tile has the color of its state: blue works, yellow waits for you, green is done, red
failed. Which tiles there are is up to you: see `dashboard` in [CONFIG.md](CONFIG.md#the-dashboard). With no
`dashboard` in the config, it shows the panes with agents and the panes where a command runs or failed.

| Key | What it does |
|---|---|
| Ctrl + Shift + D | Show or hide the dashboard. |
| 1 … 9 | Go to the pane of that tile (the dashboard hides). On a command tile: run it again. |
| a … z | Press that button (the key is in front of the button: `[a] Staging`). |
| Arrows, Enter | Choose a tile; Enter does the same as its number key. |
| F5 | Run all command tiles again now. |
| Esc | Hide the dashboard. |

Enter on a command tile goes into its list (see [Lists with actions](CONFIG.md#lists-with-actions)):

| Key | What it does |
|---|---|
| Up, Down, PageUp, PageDown, Home, End | Choose an item. |
| Enter, a letter | The action of that key on the item (the keys are at the bottom of the tile). With no action on Enter, it opens the first link of the line. |
| F5 | Run the command again now. |
| Esc | Back to the tiles. |

A click on a tile chooses it, a second click opens it, and a click on a button presses it. A button with
`confirm = true` wants a second press.

## Overlays and the scratch shell
An **overlay** is a program in a box over the active pane: a file manager (yazi, Far), lazygit, htop. The layout
does not change. The pane under the box is dim. A tab has one overlay: hide it, and its program lives on until you
show it again. When the program ends, the box goes.

The **window overlay** is the same, but one for the whole window: it comes over any tab, so you can keep one Far
there for all your tabs.

A **scratch shell** is a shell that drops down over the top of the tab, like a console in a game. Hide it, and
its shell lives on: it comes back with its history, also in another tab. There is one per window.

| Key | What it does |
|---|---|
| Ctrl + Shift + L | The overlay of the tab: with none, type a command and Enter runs it in a box over the active pane; with one, show or hide it. |
| Ctrl + Shift + J | The window overlay: show or hide it. The first time it runs `window_overlay` of the config, or asks for a command. |
| Ctrl + ` | Show or hide the scratch shell. `exit` in it ends it. |
| Ctrl + Shift + W | In an overlay or the scratch: close only that one (its program ends). |
| Alt + Shift + arrows | In the scratch: Down makes it higher, Up lower. In an overlay: Right or Down make it bigger, Left or Up smaller. 5% for each press. |

While an overlay or the scratch is shown, it gets the keys and the mouse; the panes under it get no clicks.
The sizes at the start are `overlay_size` and `scratch_size` in the config (see [CONFIG.md](CONFIG.md)). A profile with
`overlay = true` opens as an overlay too.

## Companions
A **companion** is a pane that belongs to another pane: the log of a server (`tail -f`), a watcher, a second shell.
It is not in the layout. When it is shown, it takes a strip of its pane (at the bottom or at the right). When it
is hidden, its program lives on and the pane gets all its place back. A pane can have many companions; one is
shown at a time. Companions close with their pane, and they move and zoom with it.

| Key | What it does |
|---|---|
| Ctrl + Shift + N | Type a command, and Enter runs it as a new companion of the active pane. An empty command opens a shell. |
| Ctrl + Shift + K | Hide or show the companion of the active pane. With none, it asks for a command, like Ctrl + Shift + N. |
| Ctrl + Shift + M | Show the next companion of the active pane. |
| Ctrl + Shift + W | In a companion: close only the companion (its program ends). |
| Alt + Shift + arrows | In a companion: move the line between it and its pane, so the strip gets bigger or smaller (5% for each press, for all companions). The other two arrows move the lines of its pane. |

A shown companion gets the keys. Click its pane, or hide the companion, to type in the pane again. `exit` in a
companion ends it. The place and the size at the start are `companion_side` and `companion_size` in the config (see
[CONFIG.md](CONFIG.md)). Companions are not saved in sessions.

## Scroll
| Key or mouse | What it does |
|---|---|
| Mouse wheel | Scroll 3 lines. In full-screen apps (vim, less) it sends arrow keys. |
| Shift + PageUp / PageDown | Scroll one page. |
| Shift + Home / End | Go to the top of the history / back to the bottom. |
| Any key that goes to the shell | Go back to the bottom. |

New output does not move the view while you are scrolled up. A thin bar on the right shows where you are.

## Select with the mouse
| Mouse | What it does |
|---|---|
| Drag | Select text. |
| Drag above or below the window | The view scrolls while you select. Farther away = faster. |
| Wheel while you drag | Scroll, and the selection follows the mouse. |
| Shift + click | Move the end of the selection. You can scroll far away first. |
| Double click | Select a word. URLs, paths (`C:\a\b.rs:42`), and git hashes are one word. |
| Triple click | Select a line (a wrapped line is one line). |
| Alt + drag | Select a block (a rectangle). |
| Click without a drag | Remove the selection. |
| Right click | Paste. |
| Ctrl + click on a link | Open the link in the browser. Hold Ctrl and move the mouse: a link is underlined, and the mouse shows a hand. |

## Links
fterm finds URLs in the text (`https://`, `http://`, `ftp://`, `mailto:`, `file://`), also when a long URL goes on to
the next line. It also knows OSC 8 links: a program can show a text (for example `PR #42`) with a hidden URL behind it.

| Key | What it does |
|---|---|
| Ctrl + click | Open the link under the mouse. |
| Ctrl + Shift + U | Put a label (`a`, `s`, `d`, …) on every link of the screen. |
| a label | Open its link. With more than 26 links a label has two letters. |
| Shift + a label | Copy its link. |
| Backspace / Esc | The last letter back / no labels. |

To be safe, fterm opens only `http`, `https`, `ftp`, `mailto`, and `file` links. It asks first when a link is a file,
or when the text of an OSC 8 link does not show its URL (Enter = open, C = copy, Esc = no).

The selection stays until you remove it. New output, also from apps that redraw the screen
(like Claude Code), does not remove it.

## Apps that use the mouse
Some apps ask for the mouse (vim with `set mouse=a`, htop, mc). Then clicks, drags, and the wheel go to the app.
**Hold Shift** to select text with the mouse anyway.

## Copy and paste
| Key | What it does |
|---|---|
| Ctrl + Shift + C, Ctrl + Insert | Copy the selection. |
| Ctrl + C | Copy, **when there is a selection**. With no selection it sends Ctrl+C to the app as usual. |
| Ctrl + Shift + V, Shift + Insert | Paste. |

Ins, Del, Home, End, PgUp, PgDn, and the arrows work on the numpad too (with NumLock off), for example
Shift + Ins on a laptop that has Ins only there. Other keys: `keys` in [CONFIG.md](CONFIG.md), for example
`{ key = "shift+delete", action = "copy" }`.

Copied text has no spaces at the ends of lines, and lines that were only wrapped by the screen are joined again.
The window title shows "Copied N lines" for a short time.

## History (like Far Manager)
fterm keeps the commands that you ran and the folders where you were (with shell integration).

**Alt + F8: commands.** The filter starts with the text that you already typed, so `git` + Alt+F8 shows git commands.

| Key | What it does |
|---|---|
| Type, Backspace | Filter the list (the chars must come in this order, like in the palette). |
| Up / Down, PageUp / PageDown, mouse wheel | Choose a command. |
| Enter | Put the command into the prompt **in place of** the text you typed. It does not run yet. |
| Shift + Enter | Put it there and run it. |
| Ctrl + D | Only commands from this folder (press again: all). |
| Ctrl + G | Only commands that ended well (exit code 0). |
| Ctrl + C | Copy the command. |
| Delete | Forget the command (all its runs). |
| Esc, click | Close the list. |

A failed command shows its exit code in red.

**Alt + F12: folders.** Often and recently used folders are at the top. Pinned folders (★) are always first.
A folder that is not there any more says "not found".

In a WSL pane both lists show only Linux folders (`/home/...`) and the commands that ran there; in a Windows pane
only Windows folders. So `cd` always gets a folder that the shell knows.

| Key | What it does |
|---|---|
| Enter | Go there (`cd`) in the active pane. |
| Shift + Enter | Open a new tab there. |
| Ctrl + Enter | Split the pane: the new pane starts there. |
| Ctrl + P | Pin or unpin the folder. |
| Delete | Forget the folder. |

When a program runs in the pane (not the prompt), Enter does not type into it: the text is copied, and a toast says so.

**Hints while you type** (turn on with `history = { hints = true }`): the rest of a command from the history
shows in grey after the cursor. **Right arrow** or **End** takes it. Any other key works as usual.

## A Review tab
An agent can show you a plan in a Review tab (see [REVIEW.md](REVIEW.md)): Up / Down choose an item, Space marks it Ok,
C comments, E changes the text, A adds an item, D removes, Shift + O marks the rest Ok, and S sends the review.

## The dock and its panels
The dock is an area at the right side of the window (or left, or bottom; see `panels` in [CONFIG.md](CONFIG.md)).
It has service panels: **Events** (all notifications) and **Agents** (every pane with Claude Code or another agent).

| Key or mouse | What it does |
|---|---|
| Ctrl + Shift + E | Show the Events panel and give it the keyboard. Press again to close the dock. |
| Ctrl + Shift + A | The same for the Agents panel. |
| Ctrl + Shift + B | Show or hide the dock (the keyboard stays in the terminal). |
| Ctrl + Shift + O | Move the keyboard between the terminal and the dock. |
| Up / Down, PageUp / PageDown, Home / End | Choose a row (when the dock has the keyboard). |
| Enter, click on a row | Go to the tab and pane of the row. The keyboard goes back to the terminal. |
| Tab, Left / Right, click on a panel name | The other panel. (In Events, Right reads the event.) |
| Space, Right | Events: read the whole event (its title, where it came from, and all its text). There Up / Down, PageUp / PageDown, Space, and the mouse wheel scroll it, Enter goes to its pane, Ctrl + C copies its text, and Esc, Left, or Backspace go back to the list. |
| F | Events: show only important events (warnings, errors, attention), or all again. |
| Space, Right | Agents: the details of the agent: its question, its todos, its cost and context, the files it changed, and its steps (the newest first). Ctrl + C copies them. |
| D | Agents: the changes in the folder of the agent (`git diff HEAD`) in an overlay over its pane. |
| H | Agents: the past conversations of the folder of the agent (fterm's own list, then the files of Claude Code). Enter brings one back in a new tab (`claude --resume`). |
| M | Events: mark all as read. |
| Esc, click in the terminal | The keyboard goes back to the terminal. The dock stays. |
| Drag the dock edge | Make the dock bigger or smaller. |
| Mouse wheel over the dock | Scroll the panel. |
| Click the number in the tab bar corner | It shows unread events (when the Events panel is not on the screen). The click opens the panel. |

New events have a bright title. They count as read when the Events panel goes away (or fterm goes to the back).

In the Agents panel the agents that wait for you come first, with their question. A working agent shows its todo
("1/3 · Writing tests"), the title shows the cost of the session, and the right side the context used. This needs
the hooks and the status line of fterm (see [CLAUDE.md](CLAUDE.md#set-up-the-hooks)); what shows can be changed
with `agents` and `on_agent_row` in [CONFIG.md](CONFIG.md#the-agents-panel).

## The AI panel
| Key | What it does |
|---|---|
| Ctrl + Shift + I | Open the AI panel (press again to close the dock). |
| Ctrl + Shift + X | Explain the last error: the last command and its output go to the AI. |
| Ctrl + Shift + G | Text to command: the task typed in the prompt becomes a command (it does not run). |
| Enter / Shift + Enter | Send / a new line. |
| Esc | Stop the answer; again = back to the terminal. |
| Ctrl + Shift + Enter | Put the last command of the answer into the prompt (it does not run). |
| Ctrl + C | Copy the last command of the answer. |
| Alt + O / Alt + S | Add the last output / the selection to the next question. |
| Up (empty input), PageUp, PageDown, Ctrl + L | The last question, scroll, a new chat. |

See [AI.md](AI.md).

## Typing
| Key | What it does |
|---|---|
| Shift + Enter | A new line without sending (for example, in Claude Code). It sends `ESC` + `Enter`. |
| IME (Chinese, Japanese, Korean, ...) | The text from the input method goes to the shell. The IME window opens at the cursor. |

When the window gets or loses focus, fterm tells the app (if the app asks for it).

## Copy mode (keyboard)
Press **Ctrl + Shift + Space** to start or stop copy mode. A yellow cursor shows.
It can go up into the history, and the view follows it.

| Key | What it does |
|---|---|
| Arrows, `h` `j` `k` `l` | Move. |
| `w` `b` `e` / `W` `B` `E` | Next word, word back, end of word. |
| `0` `^` `$`, Home, End | Start of line, first char, end of line. |
| `H` `M` `L` | Top, middle, bottom of the screen. |
| PageUp / PageDown, Ctrl+B / Ctrl+F | One page up / down. |
| Ctrl+U / Ctrl+D | Half a page up / down. |
| `g` / `G` | First line of the history / last line. |
| `{` `}` `%` | Paragraph up / down, matching bracket. |
| `v` / `V` / Ctrl+V | Select chars / lines / a block. Press again to stop. |
| `y`, Enter | Copy and leave copy mode. |
| Esc, `q` | Leave copy mode. |

Keys work on any keyboard layout (for example, Russian): fterm uses the key position.
