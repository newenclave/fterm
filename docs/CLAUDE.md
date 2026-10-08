# Claude Code in fterm

fterm can show what Claude Code does in each tab:

| Dot on the tab | State | When |
|---|---|---|
| blue | `working` | you sent a prompt, or Claude used a tool |
| yellow | `waiting` | Claude needs your answer (a permission question, a form) |
| green | `done` | Claude finished the answer |
| red | `error` | the answer ended with an API error |

When the state changes and you do not see that pane (it is in another tab, or fterm is not in front),
fterm shows a notification: "… waits for you", "… is done", or "… failed".
The dot for `done` and `error` goes away when you look at the pane.

## With no hooks
fterm also finds an agent itself. When `claude`, `opencode`, `codex`, `aider`, or `gemini` runs in a pane
(fterm knows the command with shell integration), the pane is in the Agents panel. Its state comes from the
window title that the tool sets: Claude Code shows a spinner while it works (`working`) and `✳` when it waits
for your next prompt (`done`). The rest of the title (what it works on) is its message.

This is less exact than hooks: fterm cannot see a permission question (`waiting`) or an error. When the
hooks are set up, fterm uses only them for that pane.

## Set up the hooks

Claude Code tells fterm its state with **hooks**. Each hook prints one line of JSON with
`terminalSequence`, so Claude Code writes a short escape sequence to the terminal:
`ESC ] 777 ; fterm-agent ; <state> ; <message> BEL`. There is no script and no extra program.

**The easy way:** open the command palette (`Ctrl+Shift+P`) and run **Install Claude Code hooks (agent states)**.
fterm shows which file and which hooks change, and asks first. It adds the hooks to `settings.json` in
`CLAUDE_CONFIG_DIR` or in `~/.claude`. Your other settings and hooks stay, and the old file is kept as
`settings.json.bak-fterm`. A second time changes nothing. Then start `claude` again.

**By hand:**
1. In fterm, open the command palette (`Ctrl+Shift+P`) and run **Copy Claude Code hooks (settings.json)**.
   Or take the file [assets/claude/hooks.json](../assets/claude/hooks.json).
2. Open `~/.claude/settings.json` (on Windows: `%USERPROFILE%\.claude\settings.json`).
3. Put the `"hooks"` part into it. If you already have hooks, add the new entries to your lists and keep yours.
4. Start `claude` in fterm.

One hook looks like this:

```json
"Stop": [
  {
    "hooks": [
      {
        "type": "command",
        "command": "echo '{\"terminalSequence\":\"\\u001b]777;fterm-agent;done;\\u0007\"}'"
      }
    ]
  }
]
```

The command is the same for bash (Git for Windows, Linux, macOS) and for PowerShell:
the text in `'...'` is not changed by either shell.

The hooks and their states:

| Hook | State |
|---|---|
| `UserPromptSubmit` | `working` |
| `PostToolUse` | `working` (after you answer a permission question, the dot is blue again) |
| `Notification` (`permission_prompt`, `elicitation_dialog`, `elicitation_url_dialog`, `agent_needs_input`) | `waiting` |
| `Stop` | `done` |
| `StopFailure` | `error` |
| `SessionEnd` | `idle` (no dot) |

`PostToolUse` runs after every tool, so it starts a small shell each time. If you do not want that, delete it.
Then the dot stays yellow after a permission question until Claude finishes.

## The fterm skill
Claude Code can also learn how to use fterm: the command palette → **Install the fterm skill for Claude Code**
writes `~/.claude/skills/fterm/SKILL.md` (or in `CLAUDE_CONFIG_DIR`). Claude then knows the recipes and the rules
(run tests in a pane, wait for an event, talk to another agent, draw a chart) when a task needs them.
fterm updates only its own file; a file of yours with the same name stays, and the skill goes to the clipboard.
`ftermctl guide --skill` prints the same file.

## Notes

- Hooks only work in an interactive Claude Code session (not with `claude -p`).
- In other terminals the sequence does nothing, so the same `settings.json` is safe everywhere.
- Every pane has the env var `FTERM_PANE_ID`. Hooks get it too, so your own hook scripts know their pane.
- You can react to state changes in your config with `on_agent` (see [CONFIG.md](CONFIG.md)).
- Other tools (OpenCode, Aider, your scripts) can use the same sequence:
  ```sh
  printf '\033]777;fterm-agent;waiting;Please check the plan\007'
  ```
  States: `working`, `waiting`, `done`, `error`, `idle`. The message is optional.
