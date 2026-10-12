# Panes that live on (the mux server)

Like tmux: the programs of mux panes run in a small server process, not in the window. Close the window, and
they go on running. Open fterm again, and the panes come back with their screen, their history, and the
programs still running in them.

## Turn it on
In `fterm.lua`:

```lua
mux = true,          -- all new panes are mux panes
profiles = {
  { name = "Claude", command = "claude", mux = true },   -- or only the panes of some profiles
  { name = "cmd", command = "cmd.exe", mux = false },    -- a profile can say no too
},
```

Without `mux`, nothing changes: panes live in the window, as before.

## How it works
- The first mux pane starts the server: `fterm --mux-server`, in the background, with no window. Its programs
  run there. The server ends by itself one minute after it has no panes and no windows.
- The window keeps its own copy of the screen of each mux pane. So selection, links, copy mode, the history,
  shell integration, and agent states work as in a normal pane.
- When the window closes, the server keeps the layout of its tabs (tabs, splits, workspaces).
- When fterm starts and the server has panes that no window shows, they come back in that layout, and a toast
  says how many. The other panes of that layout start again, as in a restored session.
- Closing a mux pane (×, Ctrl+Shift+W, or the API) ends its program in the server.
- When you close the window, fterm does not ask about programs in mux panes: they go on.

Only the current user can connect to the server (the same rule as for the API). `FTERM_MUX_SOCKET=<name>`
starts and uses a second server with its own name (for tests).

## Now, in the first version
- It works on one machine. Panes on other machines (over SSH) come later, with the same protocol.
- One window shows a pane at a time. When a second window takes a pane, the first one does not see its output
  any more.
- `FTERM_PANE_ID` and `FTERM_SOCKET` in a mux pane are the values of the window that started it. After the pane
  comes back in a new window, hooks and `ftermctl` in it still talk to the old window (which is gone).
- A full-screen program (vim, htop) gets its scroll region back when it draws again for the new size of the
  window.
