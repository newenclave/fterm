# The AI panel

Ask an AI about your terminal work without leaving fterm. The answer comes into a panel of the dock,
next to Events and Agents.

## Open it
- `Ctrl+Shift+I`, or the command palette: **AI panel**. Press `Ctrl+Shift+I` again to close the dock.
- Type your question and press **Enter**. `Shift+Enter` makes a new line.
- The answer grows while it comes. **Esc** stops it; a second Esc gives the keyboard back to the terminal.
- `Up` in an empty input brings back your last question. `PageUp` / `PageDown` and the mouse wheel scroll the chat.
- `Ctrl+V` pastes into the question. `Ctrl+L` starts a new chat.
- When the answer is ready and the panel is not on the screen (or fterm is in the back), a toast tells you.

With every question fterm sends: your OS, the shell of the active pane, and its folder.
Nothing else from your terminal goes out, unless you add it (see Context).

## Text to command
Write what you want in the prompt of your shell, in normal words, and press **Ctrl+Shift+G**:

```
PS C:\work> find the 10 biggest files here        <- you type this, then Ctrl+Shift+G
PS C:\work> Get-ChildItem -File | Sort-Object Length -Descending | Select-Object -First 10
```

- The AI gets your task, the shell of the pane, the OS, and the folder, and gives one command for that shell.
- The command takes the place of your text. **It does not run**: read it, change it, and press Enter yourself.
  (In PowerShell, `Ctrl+Z` brings back your text.)
- While it waits, grey text after the cursor says "asking AI…". **Esc** stops it.
- When you change the prompt while it waits, fterm does not touch it: the command goes to the clipboard.
- Use another (for example a bigger) model only for this: `ai = { command_model = "claude-sonnet-5" }`.

## Context
Chips over the input show what goes with the next question. Only what you see there goes out.

| Key | What it does |
|---|---|
| `Ctrl+Shift+X` (**Explain the last error**) | Sends the last command, its exit code, and its output with the question "Why did this command fail?" at once. |
| **Ask AI about the selection** (palette) | Opens the panel with the selected text as a chip. You write the question. |
| `Alt+O` (in the panel) | Adds the last command and its output. |
| `Alt+S` (in the panel) | Adds the selected text. |
| `Backspace` in an empty input | Takes away the last chip. |

A long output goes with its last 200 lines. The last command and its output need shell integration
(PowerShell has it by itself; see [CONFIG.md](CONFIG.md)).

## Commands from answers
- `Ctrl+Shift+Enter` puts the last command of the answer (its last code block) into the prompt of the active pane,
  in place of what you typed there. It does not run: you read it and press Enter yourself.
- `Ctrl+C` (in the panel) copies that command (or the whole last answer when it has no code).

## Hide secrets: on_ai_request
This function sees every question before it goes out. `r` has `question`, `text` (what goes out: the context
and the question), `provider`, and `model`. Return `false` to not send it, or return `r` with a changed `text`.

```lua
on_ai_request = function(r)
  r.text = r.text:gsub("password=%S+", "password=***"):gsub("ghp_%w+", "ghp_***")
  return r
end,
```

## The provider and the key
The default is **Anthropic** with **Claude Haiku 4.5** (`claude-haiku-4-5-20251001`): fast and cheap.

**The key:** the command palette → **Set the AI key**, then paste it (`Ctrl+V`) and press Enter.
fterm saves it in the Windows Credential Manager (on macOS: the Keychain), not in a file.
Or set the env var `ANTHROPIC_API_KEY` before you start fterm. fterm never writes keys into its log.

## Other providers
```lua
ai = {
  provider = "ollama",                       -- which one to use
  system = "Answer in Russian.",             -- your own instructions for every question
  max_tokens = 2048,
  providers = {
    anthropic = { model = "claude-sonnet-5" },             -- change the model
    ollama = { model = "qwen2.5-coder:1.5b" },             -- local, no key
    openrouter = { model = "anthropic/claude-haiku-4.5" }, -- key: OPENROUTER_API_KEY or "Set the AI key"
    lmstudio = { kind = "openai", url = "http://localhost:1234/v1", model = "local-model", key = false },
  },
},
```

Ready presets: `anthropic`, `ollama` (`http://localhost:11434/v1`, no key), `openrouter`, `openai`.
For `openrouter` and `openai`, give a `model`. A new provider needs `kind` (`"anthropic"` or `"openai"`),
`url`, and `model`. `key = false` means "no key"; `key = "MY_ENV_VAR"` reads the key from that env var.

**Ollama:** install it, then get a small model, for example `ollama pull qwen2.5-coder:1.5b`,
and set `provider = "ollama"` with that model.

## Try a key from the command line
```
cargo run -p fterm-ai --example ask -- "Why does git say detached HEAD?"
cargo run -p fterm-ai --example ask -- --ollama qwen2.5-coder:1.5b "Hello"
```
