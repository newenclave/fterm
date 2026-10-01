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
Nothing else from your terminal goes out (more context comes in the next step, and only when you add it).

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
