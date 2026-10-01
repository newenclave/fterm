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
}
```

| Field | What it does |
|---|---|
| `name` | The name in the palette and in `spawn`. |
| `command` | The program. On Windows a `.cmd` or `.bat` file (for example, a tool from npm) also works. |
| `args` | A list of arguments. |
| `cwd` | The start folder. `~` is your home folder. |
| `env` | More environment variables. |

With no `profiles` in the config, fterm finds them itself: PowerShell 7, Windows PowerShell, cmd,
Git Bash, WSL (on Linux and macOS: your shell, bash, zsh, fish), and the AI tools in your PATH:
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
| `command_palette`, `reload_config`, `open_config` | fterm itself. |

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

## Shell integration
With shell integration, the shell tells fterm:
- the current folder (OSC 7). New tabs and splits start in the folder of the active pane;
- when a command starts and ends, and its exit code (OSC 133).

**PowerShell** (5.1 and 7): fterm loads its script by itself (after your profile). Your prompt does not change.
Turn it off with `shell_integration = false`.

**bash and zsh:** fterm writes the scripts to `%LOCALAPPDATA%term\shell\` (Windows) or
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
