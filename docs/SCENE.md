# Braille scenes

A scene is a pane with no program. Agents and scripts draw into it: charts, graphs, simple games.
Every cell is a 2×4 grid of dots (Braille chars `U+2800`–`U+28FF`), so a pane of 80×24 cells is a screen
of 160×96 dots. fterm draws Braille as square pixels with no gaps (`braille_style = "pixels"`, see
[CONFIG.md](CONFIG.md)), so a scene looks like a small picture.

A scene pane is a normal pane: split, zoom, focus, resize, select, and copy work as in any pane.
When it gets bigger or smaller, what fits stays. A saved session brings it back empty.

## Open a scene
- The command palette: **New Braille scene (split right)** (`new_scene`).
- From a script: `ftermctl scene` (or `--down`, `--pane N`). It prints the pane id and the size in dots.
- The API: `scene_open` (see [API.md](API.md)). A client may draw at once into a scene it opened.

## Draw
`ftermctl draw [--pane N] JSON` sends one command or a list. With `-` the commands come on stdin
(for big batches):

```sh
ftermctl draw --pane 4 '[{"op":"clear"},{"op":"color","color":"#40c0ff"},{"op":"circle","x":40,"y":40,"r":30}]'
python make_chart.py | ftermctl draw --pane 4 -
```

Coordinates are dots: `x` from the left, `y` from the top. `text` uses cells (`col`, `row`).
Shapes that go out of the scene are cut at its edge.

| Command | Fields | What it does |
|---|---|---|
| `color` | `color`: `"#rrggbb"`, or no field | The color for the next commands; no color = the normal text color. A color is for a whole cell. |
| `dot` | `x`, `y` | One dot. |
| `undot` | `x`, `y` | Takes one dot away. |
| `line` | `x0`, `y0`, `x1`, `y1` | A line (both ends too). |
| `rect` | `x`, `y`, `w`, `h`, `fill` (default false) | A rectangle with its top left corner at `x`, `y`. |
| `circle` | `x`, `y`, `r`, `fill` (default false) | A circle around `x`, `y`. `r` is in dots up and down; the circle is round on the screen (see `aspect`). |
| `text` | `col`, `row`, `text` | Text in cells. It covers the dots of those cells. Control chars become spaces, and chars of two cells (CJK, emoji) become `?`. |
| `clear` | | No dots, no text, no colors. |

A wrong command says where it is and what is wrong (`[1]: unknown variant "fly"`), and a list with a bad
color draws nothing. The answer of `scene_draw` has the size (`cols`, `rows`, `width`, `height` in dots),
so a script can fit its picture to the pane, and `aspect`: the height of a dot / its width on the screen
(about 1.2 for most fonts). Dots are not square, so a script that wants a true square makes it
`aspect` times wider than tall; `circle` does this by itself.

## A live chart
Draw again a few times a second: clear, then the axes and the line. One `scene_draw` with the whole
list draws in one step, so the chart does not blink.
