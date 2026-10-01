# Keys and mouse

These are the default keys. You can change them in the config (see [CONFIG.md](CONFIG.md)).
**Ctrl + Shift + P** opens the command palette. **Ctrl + Shift + ,** opens the config file.

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
| Ctrl + click on a URL | Open the link in the browser. |

The selection stays until you remove it. New output, also from apps that redraw the screen
(like Claude Code), does not remove it.

## Apps that use the mouse
Some apps ask for the mouse (vim with `set mouse=a`, htop, mc). Then clicks, drags, and the wheel go to the app.
**Hold Shift** to select text with the mouse anyway.

## Copy and paste
| Key | What it does |
|---|---|
| Ctrl + Shift + C | Copy the selection. |
| Ctrl + C | Copy, **when there is a selection**. With no selection it sends Ctrl+C to the app as usual. |
| Ctrl + Shift + V, Shift + Insert | Paste. |

Copied text has no spaces at the ends of lines, and lines that were only wrapped by the screen are joined again.
The window title shows "Copied N lines" for a short time.

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
| Tab, Left / Right, click on a panel name | The other panel. |
| F | Events: show only important events (warnings, errors, attention), or all again. |
| M | Events: mark all as read. |
| Esc, click in the terminal | The keyboard goes back to the terminal. The dock stays. |
| Drag the dock edge | Make the dock bigger or smaller. |
| Mouse wheel over the dock | Scroll the panel. |
| Click the number in the tab bar corner | It shows unread events (when the Events panel is not on the screen). The click opens the panel. |

New events have a bright title. They count as read when the Events panel goes away (or fterm goes to the back).

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
