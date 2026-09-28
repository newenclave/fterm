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
2. A glyph that is not in the atlas yet is drawn with cosmic-text and copied to the atlas texture.
   When the atlas is full, it grows 2 times and the frame is built again.
3. `renderer.rs` sends the quads to the GPU and draws all of them with one draw call.
4. `shader.wgsl` has two kinds of quads: solid (`kind = 0`) and glyph (`kind = 1`).

## Colors

- `fterm-term/src/colors.rs` has the palette: 16 colors (Catppuccin Mocha), the 256-color table,
  and the default fg, bg, and cursor.
- Apps can change colors (OSC 4, 10, 11). These changes win over the palette.
- The surface is sRGB, so we send linear colors to the GPU (`fterm-render/src/color.rs`).

## Font

- JetBrains Mono is inside the app (`assets/fonts`, license: `assets/fonts/OFL.txt`).
- The cell size is whole pixels, so the cell backgrounds have no gaps between them.
- Fallback fonts (emoji, CJK) come in Phase 2.
