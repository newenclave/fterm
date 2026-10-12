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
- Closing a mux pane or its tab (×, Ctrl+Shift+W, or the API) ends its program in the server. This is also true
  for the last tab: then the window closes, and the panes of that tab end.
- When you close the window itself (its × or Alt+F4), the mux panes go on, and fterm does not ask about their
  programs.

## Stop the server
- The palette: **Stop the mux server (its panes end)**. fterm asks first and says how many panes end.
- `ftermctl mux stop` (no window needed). `ftermctl mux list` shows the panes of the server: their program, if a
  window shows them, and what runs in them.

Both end all panes of the server, also the ones that no window shows, and then the server.

Only the current user can connect to the server (the same rule as for the API). `FTERM_MUX_SOCKET=<name>`
starts and uses a second server with its own name (for tests).

## Panes on other machines (SSH)
The same works with a mux server on another machine: its programs run there, and fterm shows them.

1. Put `ftermctl` on that machine, in its PATH (take it from the fterm archive for Linux or macOS). Only ftermctl
   is needed there: it has no window and needs no GPU.
2. Make sure that `ssh user@host` works with no password question: a key or the ssh agent. fterm has no console
   to ask for a password.
3. Add the machine to `fterm.lua`:

```lua
remotes = {
  { name = "prod", host = "me@prod.example.com" },
  { name = "build", host = "build", ssh_args = { "-p", "2222" }, ftermctl = "~/bin/ftermctl" },
  -- Any command that runs `ftermctl mux proxy` there (WSL, a jump host, docker exec):
  { name = "wsl", command = { "wsl", "-d", "Ubuntu", "--", "ftermctl", "mux", "proxy" } },
},
profiles = {
  { name = "prod htop", remote = "prod", command = "htop" },   -- a profile that opens on that machine
},
```

Then the palette has **Connect: prod**. It runs `ssh -T -o BatchMode=yes me@prod.example.com ftermctl mux proxy`.
On that machine `ftermctl mux proxy` starts the mux server there (`ftermctl mux serve`) when it does not run, and
connects to it. The panes of that server that no window shows come back in a workspace called `prod`; with none,
a new tab opens there. **Connect over SSH…** asks for `user@host` for a machine that is not in the config.

- New tabs and splits of a pane on `prod` open on `prod` too. A pane there starts the shell of the user there, in
  the home folder.
- When the connection ends (the network goes, or you close the window), the panes there go on running. fterm
  says so, and **Connect: prod** brings them back.
- Closing such a pane or its tab ends its program there. `ftermctl mux stop` on that machine stops its server.
- While fterm connects, a box says "Connecting to prod…", and the window still works. Esc stops the connection.
  ssh gives up on a host that does not answer after 10 seconds.
- When something is wrong (no ftermctl there, ssh refused the key), the toast shows what ssh or the other side said.
- Some Linux machines end all programs of a user when the user logs out (systemd `KillUserProcesses=yes`). Then
  the server there ends too when the connection ends. `loginctl enable-linger <user>` on that machine stops this
  (tmux needs the same).
- WSL ends the programs that a `wsl.exe` started when that `wsl.exe` ends. So for WSL, keep the server running
  yourself, for example in a pane: `wsl -d Ubuntu -- ftermctl mux serve`.

## Now, in the first version
- Shell integration (the folder, the command marks) works in a remote pane only when its shell sends it. The
  fterm scripts are not on the other machine.
- One window shows a pane at a time. When a second window takes a pane, the first one does not see its output
  any more.
- `FTERM_PANE_ID` and `FTERM_SOCKET` in a mux pane are the values of the window that started it. After the pane
  comes back in a new window, hooks and `ftermctl` in it still talk to the old window (which is gone).
- A full-screen program (vim, htop) gets its scroll region back when it draws again for the new size of the
  window.
