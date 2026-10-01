-- fterm config. This is a Luau script: you can use variables, functions, `if`, and loops.
-- fterm reads it again when you save the file. Remove the `--` to turn a line on.
-- All fields: docs/CONFIG.md

return {
  font = { size = 14 },
  padding = 6,
  scrollback = 10000,          -- lines of history (for new tabs)
  braille_style = "pixels",    -- "pixels" (no gaps) or "dots" (round dots)

  colors = {
    -- background = "#1e1e2e",
    -- foreground = "#cdd6f4",
    -- cursor = "#f5e0dc",
    -- selection = "#585b70",
    -- ansi = { "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de" },
    -- bright = { "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#a6adc8" },
  },

  -- With no profiles here, fterm finds shells and AI tools (claude, opencode, ollama) itself.
  -- default_profile = "PowerShell",
  -- profiles = {
  --   { name = "PowerShell", command = "pwsh.exe" },
  --   { name = "Claude", command = "claude", cwd = "~/code" },
  --   { name = "Ollama", command = "ollama", args = { "run", "llama3.2" } },
  -- },

  keys = {
    -- { key = "ctrl+alt+c", action = { spawn = "Claude" } },                    -- Claude in a new tab
    -- { key = "ctrl+alt+shift+c", action = { spawn = "Claude", split = "right" } },
    -- { key = "ctrl+shift+w", action = "none" },                                  -- turn a key off
    -- { key = "ctrl+alt+g", action = function(fterm) fterm.send_text("git status\r") end },
  },

  commands = {
    -- { name = "Git status", action = function(fterm) fterm.send_text("git status\r") end },
  },

  -- The dock with service panels (Events, Agents). Ctrl+Shift+E / Ctrl+Shift+A open them.
  panels = {
    dock = "right",          -- "right", "left", or "bottom"
    size = 0.28,
    open = {},               -- for example { "events" } to open it at start
  },

  -- The command and folder history (Alt+F8, Alt+F12). A command that starts with a space is not saved.
  -- history = { enabled = true, commands = 10000, dirs = 500, ignore_space = true, hints = false },
  -- on_history = function(h) if h.cmd:find("token") then return false end end,

  -- The window title. The default is "[2/5] title — ⏳ 1 waiting" (like WezTerm).
  -- window_title = function(t) return t.title .. " (" .. t.tab .. "/" .. t.tabs .. ")" end,

  -- The window x asks first when a program or an agent runs: "running" (default), "always", "never".
  -- confirm_close = "running",
  -- on_close_window = function(info) if #info.running == 0 then return true end end,

  -- Sessions (Ctrl+Shift+S). At start: "ask" (default), "always", "never".
  -- restore = "ask",
  -- A program that ran in a pane comes back: "prompt" (default, Enter runs it), "run", "never".
  -- Claude Code comes back as `claude --continue`.
  -- restore_programs = "prompt",
  -- restore_agents = "prompt",
  -- Lines of old text that a restored pane shows in grey (0 = off; the text is saved as plain text).
  -- restore_history = 200,
  -- Change or stop a session before it opens (see docs/CONFIG.md): return it, false, or nil.
  -- on_restore = function(s) s.window = nil return s end,

  -- Toasts and OS notifications. OS notifications are off: they can be annoying.
  -- notifications = { toasts = "bottom_right", os = false, long_command = 10 },

  -- The AI panel (Ctrl+Shift+I) and "Explain the last error" (Ctrl+Shift+X). See docs/AI.md.
  -- The key: the command palette -> "Set the AI key" (or the env var ANTHROPIC_API_KEY).
  ai = {
    provider = "anthropic",  -- "anthropic", "ollama", "openrouter", "openai", or your own
    providers = {
      anthropic = { model = "claude-haiku-4-5-20251001" },
      ollama = { model = "qwen2.5-coder:1.5b" },  -- local; first run: ollama pull qwen2.5-coder:1.5b
    },
    -- system = "Answer in Russian.",
  },

  -- An agent (for example Claude Code) changed its state. Return false = no normal notification.
  -- on_agent = function(a, fterm)
  --   if a.state == "done" then fterm.notify({ title = a.name .. " is ready", level = "success" }) end
  -- end,
}
