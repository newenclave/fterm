fterm - a GPU terminal for people and AI agents

Start fterm.exe (Linux and macOS: ./fterm). There is no install.

- fterm.lua next to the program is its config. Its data (the history, the
  sessions, the shell scripts) goes into the "data" folder next to it, so the
  whole folder can move (for example to a USB drive).
- ftermctl is the command line tool and the MCP server for agents:
  "ftermctl help", "ftermctl guide".
- Ctrl+Shift+P: the command palette. Ctrl+Shift+, opens the config.
- themes: color themes. Copy one, change it, and choose it with
  theme = "its name" in fterm.lua (or "Theme..." in the palette).

Linux: fterm.desktop and fterm.png are for a menu entry; copy them to
~/.local/share/applications and ~/.local/share/icons, and put the full path of
fterm into the Exec line.

Docs: https://github.com/newenclave/fterm
License: MIT (LICENSE). Other licenses: THIRD-PARTY-NOTICES.md, OFL.txt.
