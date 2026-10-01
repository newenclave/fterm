# Config

fterm reads a config file at start, and again each time you save it. You do not need to restart.

## Where is the file?
1. The path in the `FTERM_CONFIG` variable, if it is set.
2. Windows: `%APPDATA%\fterm\fterm.lua`.
3. Linux and macOS: `~/.config/fterm/fterm.lua`.

With no file, fterm uses the built-in settings. Press **Ctrl + Shift + ,** (comma) to open the file
in your editor. If there is no file yet, fterm makes one with examples.

## The language: Luau
The config is a **Luau** script (Luau is a fast Lua from Roblox). It must `return` a table.
You can use variables, functions, `if`, and loops:

```lua
local function tool(name, cmd, args)
  return { name = name, command = cmd, args = args }
end

local work = os.getenv and true   -- normal Lua code works
return {
  font = { size = if work then 13 else 16 },
  profiles = { tool("Claude", "claude"), tool("Ollama", "ollama", { "run", "llama3.2" }) },
}
```

The script runs in a **sandbox**: it cannot open files or start programs. It runs only when the file
is loaded and when you use a key or a command that calls a Lua function, never for every frame.

If the file has an error, fterm shows it in a box and keeps the last good config.

## All fields

| Field | Type | Default | What it does |
|---|---|---|---|
| `font.size` | number | `14` | Font size in pixels (it is made bigger on HiDPI screens). |
| `padding` | number | `6` | Empty space around the text, in pixels. |
| `scrollback` | number | `10000` | Lines of history. New tabs and panes use it. |
| `braille_style` | `"pixels"` or `"dots"` | `"pixels"` | Braille chars as square pixels with no gaps, or as round dots. |
| `colors` | table | Catppuccin Mocha | See "Colors". |
| `default_profile` | string | the first profile | The profile for new tabs and splits. |
| `profiles` | list | found by fterm | See "Profiles". |
| `keys` | list | see `KEYS.md` | See "Keys". |
| `commands` | list | none | More lines for the command palette. |
| `shell_integration` | true / false | `true` | Load the fterm script into PowerShell (see "Shell integration"). |
| `notifications` | table | see "Notifications" | Toasts, OS notifications, long commands. |
| `on_notification` | function | none | See every notification first: drop, change, or route it. |

### Colors
All fields can be left out. Colors are `#rrggbb` or `#rgb`.

```lua
colors = {
  background = "#1e1e2e",
  foreground = "#cdd6f4",
  cursor = "#f5e0dc",
  selection = "#585b70",
  ansi   = { "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de" },
  bright = { "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#a6adc8" },
}
```

`ansi` and `bright` are 8 colors: black, red, green, yellow, blue, magenta, cyan, white.
You can give only some of them: `bright = { [2] = "#ff0000" }` changes only bright red.

### Profiles
A profile is a program to start in a tab or a pane.

```lua
profiles = {
  { name = "PowerShell", command = "pwsh.exe" },
  { name = "Claude", command = "claude", cwd = "~/code/my-app" },
  { name = "Ollama", command = "ollama", args = { "run", "llama3.2" } },
  { name = "Build", command = "cmd.exe", args = { "/k", "cargo watch" }, env = { RUST_LOG = "debug" } },
  { name = "Ubuntu", wsl = "Ubuntu" },   -- a WSL distro: no command is needed
}
```

| Field | What it does |
|---|---|
| `name` | The name in the palette and in `spawn`. |
| `command` | The program. On Windows a `.cmd` or `.bat` file (for example, a tool from npm) also works. |
| `args` | A list of arguments. |
| `cwd` | The start folder. `~` is your home folder. |
| `env` | More environment variables. |
| `wsl` | A WSL distro (for example `"Ubuntu"`). fterm starts `wsl.exe -d Ubuntu --cd ~` and knows that the pane has Linux folders. |

With no `profiles` in the config, fterm finds them itself: PowerShell 7, Windows PowerShell, cmd,
Git Bash, one profile for each WSL distro (from `wsl.exe -l -q`; on Linux and macOS: your shell, bash, zsh, fish),
and the AI tools in your PATH:
`claude`, `opencode`, `ollama` (with `run llama3.2`), `openclaude`.

### Keys
```lua
keys = {
  { key = "ctrl+alt+c", action = { spawn = "Claude" } },                    -- a new tab
  { key = "ctrl+alt+shift+c", action = { spawn = "Claude", split = "right" } },
  { key = "ctrl+shift+t", action = "split_down" },                            -- change a default key
  { key = "ctrl+shift+w", action = "none" },                                  -- turn a key off
  { key = "ctrl+alt+g", action = function(fterm) fterm.send_text("git status\r") end },
}
```

**Key names:** modifiers `ctrl`, `shift`, `alt`, `win` (also `super`, `cmd`), then one key:
a letter, a digit, a symbol (`=` `-` `[` `]` `;` `'` `,` `.` `/` `\` `` ` ``), `plus`, `minus`,
`tab`, `enter`, `esc`, `space`, `backspace`, `delete`, `insert`, `home`, `end`, `pageup`, `pagedown`,
`up`, `down`, `left`, `right`, `f1` … `f24`. Keys work by their place on the keyboard, so they
also work on other layouts (for example, Russian).

**Actions:** a name from the list below, `{ spawn = "Profile", split = "right" | "down" }`
(no `split` = a new tab), a Lua function, or `"none"`.

| Name | What it does |
|---|---|
| `new_tab`, `close_pane`, `rename_tab` | Tabs and panes. |
| `next_tab`, `prev_tab`, `select_tab_1` … `select_tab_N`, `last_tab` | Go to a tab. |
| `move_tab_left`, `move_tab_right` | Move the tab. |
| `split_right`, `split_down`, `zoom` | Split panes. |
| `focus_left`, `focus_right`, `focus_up`, `focus_down` | Go to a pane. |
| `resize_left`, `resize_right`, `resize_up`, `resize_down` | Move the line between panes. |
| `copy`, `paste`, `copy_mode` | Copy and paste. |
| `scroll_page_up`, `scroll_page_down`, `scroll_top`, `scroll_bottom` | Scroll. |
| `command_palette` (Ctrl+Shift+P), `reload_config` (Ctrl+Shift+F5), `open_config` (Ctrl+Shift+,) | fterm itself. |
| `copy_claude_hooks` | Copy the Claude Code hooks for tab dots (see [CLAUDE.md](CLAUDE.md)). |
| `toggle_dock`, `panel_events`, `panel_agents`, `focus_dock` | The dock and its panels (see [KEYS.md](KEYS.md)). |
| `history_commands`, `history_dirs` | The command and folder history (Alt+F8, Alt+F12). |
| `toggle_remote_control` | API clients may (or may not) read and type into this pane (see [API.md](API.md)). |
| `panel_ai`, `explain_error`, `ask_ai_selection`, `set_ai_key`, `text_to_command` | The AI panel and text to command (see [AI.md](AI.md)). |
| `sessions` (Ctrl+Shift+S), `save_session_as`, `restore_session` | Saved sessions (see [Sessions](#sessions)). |

### Lua functions
A function gets an object `fterm` with these functions:

| Function | What it does |
|---|---|
| `fterm.spawn("Profile", { split = "right" })` | Start a profile (no name = the default profile; no `split` = a new tab). |
| `fterm.send_text("text\r")` | Type text into the active pane (`\r` = Enter). |
| `fterm.notify("text")` | Show a short message. |
| `fterm.copy("text")` | Put text into the clipboard. |
| `fterm.action("zoom")` | Run an action from the list above. |

```lua
keys = {
  -- Claude on the right, and the tests below it.
  { key = "ctrl+alt+d", action = function(fterm)
      fterm.spawn("Claude", { split = "right" })
      fterm.spawn(nil, { split = "down" })
      fterm.send_text("cargo test\r")
    end },
}
```

### Commands
```lua
commands = {
  { name = "Git status", action = function(fterm) fterm.send_text("git status\r") end },
  { name = "Claude here", action = { spawn = "Claude", split = "right" } },
}
```

Commands show in the command palette (Ctrl + Shift + P), next to all actions and profiles.

## Panels
The dock is an area at a window edge with service panels (Events, Agents). It does not cover the terminal:
the panes get smaller.

```lua
panels = {
  dock = "right",          -- "right" (the default), "left", or "bottom"
  size = 0.28,             -- the dock part of the window, from 0.1 to 0.9
  open = { "events" },     -- the panel that is open at start; {} or no `open` = the dock is closed
},
```

Panels: `events` (all notifications, newest first) and `agents` (every pane with an agent state).
You can also drag the dock edge with the mouse. Keys: see [KEYS.md](KEYS.md).

## Notifications
fterm collects notifications from many places:
- apps in a pane (OSC 9, OSC 99, OSC 777 — for example from scripts or Claude Code hooks);
- a long command that ended while you did not see its pane;
- `fterm.notify` in your config;
- fterm itself (for example, an error in the config).

All of them go to the **Events panel** (see [Panels](#panels)). They also show as **toasts**: small boxes in a corner. Toasts never take the focus, your keys always go to the terminal,
and the mouse only works on the toast itself:
- the mouse over a toast stops its timer;
- a click goes to its tab and pane;
- `×` closes it.

Info and success toasts hide after 4 s, warnings after 8 s, errors and "attention" stay until you close them.

```lua
notifications = {
  toasts = "bottom_right",      -- "top_right", "bottom_left", "top_left", "bottom", or false (no toasts)
  max_visible = 4,
  os = false,                   -- system notifications: OFF by default
                                -- true / "always", "when_unfocused",
                                -- or { when = "when_unfocused", levels = { "attention", "error" } }
  long_command = 10,            -- seconds; 0 = no "command finished" notifications
  bell = "ignore",              -- "notify" = the bell makes a notification
  flash = true,                 -- flash the taskbar for "attention" when fterm is not in front
},
```

Levels: `info`, `success`, `warning`, `error`, `attention`.

### on_notification
This function sees every notification before it shows. Return `nil` to drop it, or return the table
(you can change it). Fields: `title`, `body`, `level`, `source` (`terminal`, `command`, `agent`, `lua`, `app`),
`pane` (a number or nil). Set `os = true` or `false` to choose the OS notification for this one, and
`toast = false` to show no toast.

```lua
on_notification = function(n, fterm)
  if n.body:find("heartbeat") then return nil end           -- drop noise
  if n.source == "command" and n.level == "error" then
    n.os = true                                             -- failed builds also go to the OS
  end
  return n
end,
```

### fterm.notify
```lua
fterm.notify("Saved")                                                  -- info
fterm.notify({ title = "Deploy", body = "done", level = "success" })
```

### on_agent
This function runs when an agent in a pane changes its state (see [CLAUDE.md](CLAUDE.md)).
Fields: `pane`, `state` (`working`, `waiting`, `done`, `error`, `idle`), `previous`, `message`, and `name` (the tab title).
It can call `fterm.notify`, `fterm.spawn`, and the other functions. Return `false` to stop the normal notification.

```lua
on_agent = function(a, fterm)
  if a.state == "done" and a.previous == "working" then
    fterm.notify({ title = a.name .. " is ready", level = "success" })
    return false                                            -- my notification, not the normal one
  end
end,
```

## History
fterm saves the commands that you run and the folders where you were (it needs shell integration, see below).
Alt+F8 shows the commands and Alt+F12 the folders (see [KEYS.md](KEYS.md)).

```lua
history = {
  enabled = true,          -- false = save nothing
  commands = 10000,        -- how many commands to keep
  dirs = 500,              -- how many folders to keep (pinned folders always stay)
  ignore_space = true,     -- a command that starts with a space is not saved
  hints = false,           -- grey hints from the history while you type (Right arrow or End takes it)
},
```

The files are in `%APPDATA%\fterm\history\` (Linux and macOS: `~/.local/share/fterm/history/`):
`commands.jsonl` and `dirs.jsonl`, one JSON record per line. They are plain text: you can read them,
change them, or delete them. Many fterm windows can write to them at the same time. When a file gets
two times bigger than the limit, fterm writes it again with only the newest records.

**Hints** (`hints = true`): when you type at the prompt, fterm shows the rest of the newest command that
starts with your text (commands from this folder first) in grey after the cursor. Right arrow or End takes it.
They are off by default, because PowerShell 7 (PSReadLine), fish, and zsh plugins have their own hints.
When the shell draws its own hint, fterm does not draw one.

### on_history
This function sees every command before it is saved. Fields: `cmd`, `cwd`, `exit`, `shell`.
Return `false` to not save it, or return the table (you can change `cmd`). `nil` saves it as it is.

```lua
on_history = function(h)
  if h.cmd:find("token") or h.cmd:find("password") then return false end   -- never save secrets
end,
```

## The window title
Like WezTerm, the window title shows the tab number and the number of tabs, then the title of the active tab:
`[2/5] claude`. With one tab there is no number. When agents in **other** tabs wait for you or failed,
the title says so: `[2/5] claude — ⏳ 1 waiting · ✗ 1 failed`. So you see it in the taskbar too.

### window_title
Make your own title. `t` has `tab`, `tabs`, `title`, `agent` (the state in the active tab, or nil),
`waiting`, `failed`, and `default` (the title fterm would show). Return a string, or `nil` for the default.

```lua
window_title = function(t)
  if t.waiting > 0 then return "⏳ " .. t.default end
  return t.title .. "  (" .. t.tab .. "/" .. t.tabs .. ")"
end,
```

## Closing the window
The window × (and Alt+F4, and closing the last tab) asks first when something runs:

```lua
confirm_close = "running",   -- the default: ask when a program runs in a pane, or an agent works or waits
                             -- "always" = always ask, "never" = never ask
```

A pane "runs something" when a program runs in its shell (for example `cargo build`), or when the pane
itself is not a shell (for example a `claude` profile).

### on_close_window
This function decides before the rule. `info` has `tabs`, `panes`, `running` (a list of `{ tab, program }`),
and `agents` (a list of `{ tab, name, state }`). Return `true` to close at once, `false` to keep the window
open, or `nil` for the `confirm_close` rule.

```lua
on_close_window = function(info)
  for _, a in ipairs(info.agents) do
    if a.state == "working" then return false end   -- never close while an agent works
  end
  if #info.running == 0 then return true end        -- only idle shells: close without a question
end,
```

When the function says `false`, a toast tells you. Press × again in the next 5 seconds to get the normal
question, so a wrong function can never lock the window.

## Sessions
Like a browser, fterm can bring back the window you had before: the tabs, the splits, the folder of each pane,
the dock, and the window size.

```lua
restore = "ask",   -- the default: at start, ask "Restore the last session?" (Enter = yes, Esc = no)
                   -- "always" = restore without a question, "never" = do not save sessions
```

- fterm saves the window every 30 seconds and when it closes. So after a crash you lose 30 seconds at most.
- fterm asks only when no other fterm window is open.
- Each pane starts its profile in its old folder. When a program ran in the pane (for example `npm run dev`),
  its command is in the prompt again, and you press Enter to run it. Claude Code comes back as
  `claude --continue`, so you get its conversation back. This needs shell integration (fterm knows the command
  from it).

```lua
restore_programs = "prompt",   -- the default: put the command into the prompt; "run" = run it at once; "never"
restore_agents = "prompt",     -- the same for Claude Code (`claude --continue`)
restore_history = 200,         -- lines of old text that a restored pane shows in grey; 0 = off
```

- A restored pane shows its old text in grey, a line "── restored · saved 5 min ago ──", and then the new
  prompt. When you type, the view goes down as usual; scroll up to see the old text again.
- The old text is saved in the session file as plain text. If your screens can have secrets, use
  `restore_history = 0`.

- **Ctrl + Shift + S** (`sessions`) shows the list of sessions: your named sessions (★) first, then the last
  20 closed windows, newest first. Enter = restore, Delete = forget.
- **Save session as…** (`save_session_as`, in the palette) saves this window with a name. A named session
  stays in the list after you restore it; a closed one leaves the list.
- **Restore the last session** (`restore_session`, in the palette) opens the newest closed window.
- The files are in `%LOCALAPPDATA%\fterm\sessions\` (or the folder in `FTERM_SESSION_DIR`).

### on_restore
This function gets a session before it opens. The session is a table with the same fields as the session file:
`tabs` (each with `title`, `active`, and `layout`), `active_tab`, `window`, and `dock`. A `layout` is
`{ pane = { profile, cwd, program, ran, text } }` or `{ split = { direction, ratio, first, second } }`.
Return the table (changed or not) to restore it, `false` to not restore it (it stays in the list),
or `nil` for no change.

```lua
on_restore = function(s)
  local keep = {}
  for _, tab in ipairs(s.tabs) do
    local pane = tab.layout.pane
    if not (pane and pane.cwd and pane.cwd:find("Temp")) then   -- no tabs from temp folders
      table.insert(keep, tab)
    end
  end
  s.tabs = keep           -- no tabs left = nothing to restore
  s.window = nil          -- keep the window where it is
  return s
end,
```

When the function fails, fterm shows the error and restores the session as it was.

## Shell integration
With shell integration, the shell tells fterm:
- the current folder (OSC 7). New tabs and splits start in the folder of the active pane;
- when a command starts and ends, and its exit code (OSC 133);
- the text of each command (OSC 633;E, the same as VS Code), for the command history.

**PowerShell** (5.1 and 7): fterm loads its script by itself (after your profile). Your prompt does not change.
Turn it off with `shell_integration = false`.

**bash and zsh:** fterm writes the scripts to `%LOCALAPPDATA%\fterm\shell\` (Windows) or
`~/.local/share/fterm/shell/`. Add one line to your rc file:

```sh
# ~/.bashrc
[ "$TERM_PROGRAM" = fterm ] && source ~/.local/share/fterm/shell/fterm.bash
# ~/.zshrc
[[ "$TERM_PROGRAM" == fterm ]] && source ~/.local/share/fterm/shell/fterm.zsh
```

Every pane gets `TERM_PROGRAM=fterm`.

## What comes later
Functions for events (`on_output`, `on_agent_event`, …) come with Phase 6 (events and shell integration).
