# Pictures in panes

fterm can show pictures (PNG, JPEG, BMP) in any pane. A picture takes a rectangle of cells, for example
40×12. The cells are its frame: the text around it stays in its place, and the picture scrolls with the text.

There are two kinds:
- **A picture in the output** of a pane, between the lines of text. A program writes it, or you run
  `ftermctl image`.
- **A picture pane**: a pane with no program. Its picture fills the pane and keeps its shape, also when the
  pane gets a new size.

## A picture in the output
Run it in the pane where you want the picture:

```sh
ftermctl image cat.png             # 12 rows high (the width keeps the shape)
ftermctl image --height 6 cat.png  # 6 rows high
```

`ftermctl image` asks no window, so it works the same in WSL and over SSH (with `ftermctl` on that machine).

Programs can show pictures too, with the iTerm2 sequence `OSC 1337 ; File=...`. On Windows this is the only
picture sequence that gets through ConPTY (Sixel and Kitty pictures do not).

**On Windows the program must make room for the picture.** ConPTY does not know that a picture takes lines, so
its cursor stays where the picture starts. fterm keeps its cursor there too. The program then writes new lines
(`ftermctl image` does this). If a program does not do it, its next text goes over the picture, and the picture
is gone. On Linux and macOS (no ConPTY) fterm moves the cursor below the picture, as iTerm2 does.

A picture goes away with its cells:
- when its lines leave the history (the oldest lines go when the history is full);
- after `cls` or `clear`;
- when a program writes text on its first cell.

When the pane gets a new width, the picture stays on its line.

`ftermctl image` also puts a small mark on the first cell of the picture (an OSC 8 link on a space). ConPTY keeps
such links, so the picture stays when ConPTY paints the screen again (it does that after a resize). A program
that writes only `OSC 1337` has no such mark: with the ConPTY of Windows its picture can go away after a resize.

## A picture pane
- The command palette: **Show the picture in the clipboard** (`show_clipboard_picture`), for example a
  screenshot that you just took.
- The command palette: **Show a screenshot of this pane** (`show_pane_screenshot`): a picture of the active pane
  next to it.
- From a script: `ftermctl show-image cat.png` (`--down`, `--pane N`).
- The API: `show_image` with `data` (the file in base64) or `file` (a path on the fterm machine), `place`
  (`"right"` or `"down"`), and `pane` (see [API.md](API.md)).
- An agent: the MCP tool `show_image` with `path` or `data` (see [MCP.md](MCP.md)). The output of an agent such
  as Claude Code is a TUI that draws over its cells, so an agent shows a picture in a pane next to it.

A picture pane is not saved in a session: it does not come back after a restart.

## Mark up a picture
In a picture pane you can draw on the picture, for example to show an agent what is wrong. The last row of the
pane is the tool bar: it shows the tool, the color, and the keys.

| Key | What it does |
|---|---|
| `P` | Pen: draw with the mouse. |
| `R` | Box: drag from one corner to the other. |
| `A` | Arrow: drag from its start to its tip. |
| `T` | Text: click where the text goes, type it, then Enter. |
| `H` | Hide: drag over a secret. It gets big pixels (a blur can sometimes be undone, big pixels cannot). |
| `C` | Crop: drag over the part to keep. Crop again to cut more. |
| `1`–`4` | The color: red, yellow, green, blue. |
| `Ctrl+Z`, `Ctrl+Y` | Undo, redo. |
| `Esc` | Drop the mark that you draw now, or the text that you type. |
| `Enter` | Done (see below). |

The cursor is a cross over the picture. Lines and text are bigger on a big picture, so you see them on a big
screenshot too.

**Enter** saves the picture with its marks as a PNG in `data_dir/shots` (or in the temp folder), puts it into the
clipboard, pastes its **path** into the pane that the picture was opened next to, and closes the picture pane. An
agent like Claude Code reads a picture from a path in your prompt, so you only add your words.

A typical way: take a screenshot (Win+Shift+S), run **Show the picture in the clipboard** in the palette next to the
pane of the agent, mark it, press Enter, and write your question after the path.

### An agent asks you to mark up a picture
An agent can show you a picture with a question and wait for your marks: `show_image` with `question` and
`wait: true` (see [API.md](API.md) and [MCP.md](MCP.md)). The question is on the tool bar. When you press Enter,
the agent gets the picture with your marks (it does not go to the clipboard, and nothing is pasted). When you close
the picture pane, the agent gets no picture. From a script: `ftermctl show-image --wait --question "Mark it" FILE`
prints the path of the marked-up picture.

## Pixels or Braille
`pictures` in `fterm.lua` (see [CONFIG.md](CONFIG.md)):

| Value | What you see |
|---|---|
| `"pixels"` (default) | The picture with its real pixels, scaled to its cells. |
| `"braille"` | The picture as Braille dots: 2×4 dots and one color in each cell. |
| `"off"` | No pictures (the cells stay empty). |

A picture that is too big for the GPU is drawn as Braille too.

## Limits
- A picture file can be at most 24 MiB (32 MiB of base64) and 40 million pixels. A bigger one is not shown, and
  fterm writes a line to its log.
- A side longer than 4096 pixels is scaled down.
- The pictures of one pane use at most 256 MiB of memory; the oldest go first.
- GIF and WebP do not work yet.
