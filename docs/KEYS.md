# Keys and mouse

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

The selection stays until you remove it. New output, also from apps that redraw the screen
(like Claude Code), does not remove it.

## Copy and paste
| Key | What it does |
|---|---|
| Ctrl + Shift + C | Copy the selection. |
| Ctrl + C | Copy, **when there is a selection**. With no selection it sends Ctrl+C to the app as usual. |
| Ctrl + Shift + V, Shift + Insert | Paste. |

Copied text has no spaces at the ends of lines, and lines that were only wrapped by the screen are joined again.
The window title shows "Copied N lines" for a short time.

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
