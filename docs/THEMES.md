# Themes

A theme gives fterm all its colors: the terminal (the text, the background, the 16 colors) and the UI
(the tab bar, the dock, toasts, the palette, the frames of panes). The default theme is **Catppuccin Mocha**.
**Catppuccin Latte** (a light theme) is built in too.

## Choose a theme
In `fterm.lua`:

```lua
theme = "Catppuccin Latte",                                       -- a built-in theme or a file
theme = { light = "Catppuccin Latte", dark = "Catppuccin Mocha" }, -- follow the light or dark mode of the system
```

fterm changes the theme at once when you save the file.

You can also change it while fterm runs. This change lasts until fterm closes or you change the config:
- the command palette (Ctrl+Shift+P) → **Theme…**, then Enter;
- `ftermctl theme "Catppuccin Latte"` (`ftermctl theme` lists the themes; `*` is the theme in use);
- the API method `set_theme` and the MCP tool `set_theme` (see [API.md](API.md) and [MCP.md](MCP.md)).

## Your own themes
Put a `.json` file into a folder `themes` next to `fterm.lua` (or into `themes` in your `data_dir`).
Then use its file name or the `name` in it: `theme = "nord"` or `theme = "Nord"`.
fterm reads the file again when you save it.

```json
{
  "name": "Nord",
  "terminal": {
    "background": "#2e3440",
    "foreground": "#d8dee9",
    "cursor": "#d8dee9",
    "selection": "#434c5e",
    "ansi":   ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0"],
    "bright": ["#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"]
  },
  "ui": {
    "accent": "#88c0d0"
  }
}
```

All parts are optional:
- No `terminal` colors: they are the colors of Catppuccin Mocha.
- No `ui` colors: fterm makes them from the `terminal` colors. So a theme with only the 16 colors,
  the background, and the text color already colors the whole UI.
- `ansi` and `bright` are lists of 8 colors: black, red, green, yellow, blue, magenta, cyan, white.
- A color is `#rrggbb` or `#rgb`.

The same theme can be in `fterm.lua`, as a Lua table or as JSON text:

```lua
theme = {
  name = "Mine",
  terminal = { background = "#101418", foreground = "#e0e0e0" },
  ui = { accent = "#ff8800" },
},
-- or
theme = [[ { "name": "Mine", "ui": { "accent": "#ff8800" } } ]],
```

`colors = { ... }` still works. It changes some terminal colors of the theme (see [CONFIG.md](CONFIG.md#colors)).

## Windows Terminal color schemes
A color scheme from Windows Terminal is a theme too. There are many ready schemes on the internet
(for example on windowsterminalthemes.dev). Save one as a `.json` file in the `themes` folder:

```json
{
  "name": "Dracula",
  "background": "#282A36", "foreground": "#F8F8F2",
  "cursorColor": "#F8F8F2", "selectionBackground": "#44475A",
  "black": "#21222C", "red": "#FF5555", "green": "#50FA7B", "yellow": "#F1FA8C",
  "blue": "#BD93F9", "purple": "#FF79C6", "cyan": "#8BE9FD", "white": "#F8F8F2",
  "brightBlack": "#6272A4", "brightRed": "#FF6E6E", "brightGreen": "#69FF94", "brightYellow": "#FFFFA5",
  "brightBlue": "#D6ACFF", "brightPurple": "#FF92DF", "brightCyan": "#A4FFFF", "brightWhite": "#FFFFFF"
}
```

fterm makes the UI colors from it.

## The UI colors
Each UI color has a role. The table shows where each role is used, and how fterm makes it when the theme
does not give it (`bg` = the background, `fg` = the text color).

| Role | Where | Made from |
|---|---|---|
| `surface` | the tab bar, the dock | `bg`, 20% darker |
| `surface_active` | the active tab | `bg` |
| `overlay` | a tab under the mouse, the palette, message boxes, toasts, the dock edge, scroll bars | `bg` with 12% of `fg` |
| `selected` | selected rows, the lines between panes | `bg` with 20% of `fg` |
| `accent` | the line on the active tab, box borders, the frame of the active pane | magenta |
| `text` | the active tab, the text in boxes | `fg` |
| `text_dim` | other tabs, hints | `fg` with 35% of `bg` |
| `text_ghost` | the grey hint from the history while you type | `fg` with 55% of `bg` |
| `scrollbar` | the scroll indicator of a pane | `fg` with 45% of `bg` |
| `copy_cursor` | the cursor in copy mode | yellow |
| `code_bg`, `input_bg`, `chip_bg` | the AI chat: code, the input box, the context chips | `bg` and `fg` mixes |
| `info`, `success`, `warning`, `error`, `attention` | toasts, the Events panel, the dot in the tab bar | blue, green, yellow, red, `accent` |
| `agent_working`, `agent_waiting`, `agent_done`, `agent_error` | the agent dots on tabs and in the Agents panel | blue, yellow, green, red |

The built-in themes give every role. You can see them in the fterm repository:
[assets/themes](../assets/themes). Copy one and change it to make your own theme.

## The colors of programs (harmonize)
A theme changes the 16 colors of the terminal. Many programs use them (`ls`, `git`, PowerShell), so they
follow the theme. But some programs choose their own exact colors (truecolor, or the 256-color table), for
example the red and green lines of a diff in Claude Code. Those colors stay as the program chose them, so in a
gray or a light theme they can look wrong, or be hard to read.

`harmonize` in a theme makes them fit:

```json
{ "name": "Graphite", "terminal": { ... }, "harmonize": { "strength": 0.8, "min_contrast": 3.0 } }
```

- `strength`: 0 = the colors of programs as they are (the default), 1 = fully in the style of the theme.
  0.5 or 0.6 keeps them close to their own colors.
- `min_contrast`: the lowest contrast of text against its background (1 to 21; the default 3).

What fterm does with a color of a program:
- **Hue:** it goes to the nearest color of the theme. A red stays red, but it is the theme's red.
- **Strength of the color:** it goes to that of the theme color. So a gray theme makes all colors gray, and a
  pastel theme makes them pastel. A gray stays a gray.
- **Lightness:** programs expect a dark background. fterm keeps how much lighter than the background a color
  is. On a light theme this goes the other way: light gray text becomes dark gray text, and a dark red line
  of a diff becomes a light red one.
- **Readable:** at the end, text gets at least `min_contrast` against its background.

The 16 colors of the theme do not change. The results are kept in a cache, so this does not make fterm slower.

You can change it without a new theme, and turn it off for some programs:

```lua
harmonize = { strength = 0.6 },          -- in fterm.lua: changes the value of the theme
profiles = {
  { name = "btop", command = "btop", harmonize = false },   -- this program keeps its exact colors
},
```

To find a good strength, try it live: **Ctrl + Shift + ]** makes it stronger and **Ctrl + Shift + [** weaker,
in steps of 0.1 (the actions `harmonize_more` and `harmonize_less`, also in the palette). A toast says the value
and the line for `fterm.lua` that keeps it. The value lasts until the config changes or fterm closes.

The action `toggle_original_colors` ("Original colors of programs on or off" in the palette) shows the
colors of programs as they are, and back. Give it a key to compare fast:
`keys = { { key = "ctrl+shift+f8", action = "toggle_original_colors" } }`.

## When something is wrong
A theme with a bad color, an unknown key, or bad JSON is not used. fterm keeps the last good theme and shows
an error toast that says where the problem is (for example `ui.acent is not a UI role`).
