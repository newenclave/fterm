# fterm architecture

## Crates

| Crate | What it does | GPU? |
|---|---|---|
| `fterm-term` | Runs the shell in a pty. Keeps the text grid. Colors. Grid size. | No |
| `fterm-render` | Font, glyph atlas, and drawing the grid with wgpu. | Yes |
| `fterm` | The app: window, keys, events. It connects the other crates. | Yes |

`fterm-term` has no GPU and no window code. So we can test it with real
processes and real escape codes, and later use it from the CLI and MCP (Phase 7).

## Threads

```
 pty thread (alacritty_terminal)            window thread (winit)
 ───────────────────────────────            ─────────────────────
 read shell output
 parse it, change the grid   ──Redraw──▶    request_redraw()
 (Term is behind a lock)                    RedrawRequested:
                                              lock Term → build quads → draw
 write to the shell          ◀──bytes──     key press → encode_key() → Session::write
 resize the pty              ◀──size───     window resize → GridSize → Session::resize
 shell ended                 ──Exit────▶    close the window
```

- Events from the pty thread go to the window thread with an `EventLoopProxy`.
- The window thread draws only when something changes (`ControlFlow::Wait`).
- `Session::with_term` holds the lock only while we build one frame.

## One frame

1. `frame::build_frame` goes over the cells and makes a list of quads (`Instance`):
   backgrounds, cursor, glyphs, underlines. It has no GPU code, so the tests check it.
2. Each cell becomes a `GlyphKey`: the char, its zero-width chars (combining marks, VS16, ZWJ),
   bold/italic, and "wide" (2 cells).
3. A glyph that is not in an atlas yet is drawn:
   - box lines, blocks, and Braille (U+2500–259F, U+2800–28FF) by our own code (`builtin.rs`).
     They fill the whole cell, so cells meet with no gaps;
   - all other chars by the font (`font.rs`): the whole cluster is shaped at once, and it is
     made smaller (or, for emoji, bigger) to fit into its 1 or 2 cells.
4. There are two atlases: **mask** (R8, normal text and builtin chars) and **color** (RGBA, emoji).
   When an atlas is full, it grows 2 times and the frame is built again.
5. `renderer.rs` sends the quads to the GPU and draws all of them with one draw call.
6. `shader.wgsl` has three kinds of quads: solid (`0`), glyph (`1`, mask × text color), color glyph (`2`).

## Colors

- `fterm-term/src/colors.rs` has the palette: 16 colors (Catppuccin Mocha), the 256-color table,
  and the default fg, bg, and cursor.
- Apps can change colors (OSC 4, 10, 11). These changes win over the palette.
- The surface is sRGB, so we send linear colors to the GPU (`fterm-render/src/color.rs`).
- Text gamma: font glyph alpha goes through `text_alpha` (alpha^(1/1.45)), so light text on a dark
  background does not look thin. Builtin chars do not use it (their edges are exact).

## Fonts

- JetBrains Mono is inside the app (`assets/fonts`, license: `assets/fonts/OFL.txt`).
- System fonts are loaded too (about 90 ms). cosmic-text picks a fallback font by script:
  on Windows, for example, Segoe UI Emoji, Yu Gothic, Microsoft YaHei, Malgun Gothic, Segoe UI.
- A char that no font has is drawn as a hollow box.
- The cell size is whole pixels, so the cell backgrounds have no gaps between them.
- Tests use `Fonts::embedded_only`, so they give the same result on every machine.
  Tests that need system fonts (emoji, CJK) do nothing when the font is not there.

## Selection

- alacritty removes `term.selection` when an app erases or rewrites the selected lines.
  `fterm-term/src/select.rs` (`StickySelection`) keeps the user's selection and puts it back,
  moved by the lines that went into the history. `Session::with_term` calls it before every use.
- Change the selection only through `Session::with_term_mut` and `StickySelection::set`.
- Copy mode (`copy_mode.rs`) uses the vi mode of alacritty. After each move the view scrolls to the cursor.
- Limit: when the history is full (10 000 lines) and output is very fast, a restored selection can move by a few lines.

## Known limits

- The alacritty parser does not join graphemes: ZWJ families, skin tones, and flags are drawn as
  separate chars, and `❤️` gets 1 cell. See the roadmap.
- No bidi: right-to-left text is shown in grid order.
- On Windows, a command that ends at once can lose its output (ConPTY + alacritty loop). Phase 3b fixes it.
