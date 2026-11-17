# Forms

An agent (Claude Code, a script) can show you a **form**: fields with values that you can change. You save
it, and the agent gets the new values. So you and an agent can change settings together: of fterm, of git,
of any program. The agent reads and writes the settings file itself. fterm only shows the form and gives back
what you chose, so every file format works.

## How it looks
A form opens like a plan [Review](REVIEW.md): by default it is a box over the pane of the agent that asked, with
a yellow dot on the tab and a notification. `plan_review.place` and `plan_review.focus` in the config are for
forms too.

The form gets your keys only while the pane of the agent is your active pane. When you type in another pane of
that tab, your keys stay there. Click the notification, or go to the pane of the agent, to fill it in.

At the top are the title, who asked ("from claude"), and a short text (for example the name of the file). Then
come the fields: a label on the left, the value on the right. A field can have a help line, which shows when
the field has the focus. At the bottom are the buttons **Save** and **Cancel** and a line with the keys.

## Keys
| Key | What it does |
|---|---|
| Tab, Shift+Tab | the next field, the field before (the buttons too) |
| ↑ ↓ | the field above or below; in an input with many lines: the line above or below |
| Space | a checkbox on or off; the next option of a radio group; opens a list |
| ← → | the option before or after (radio group, list); in a text: the cursor |
| Enter | on a list: open it; on a button: press it; in an input with many lines: a new line; else the next field |
| Ctrl+S or Ctrl+Enter | save |
| Esc | cancel; when you changed something, press it a second time |
| Delete | a checkbox, radio group, or list that may be "not set" (`clearable`): make it "not set" (in a text, Delete deletes a char) |
| Ctrl+V, Ctrl+Shift+V, Shift+Insert | paste |

In an open list: type to find an option, ↑ ↓ to choose, Enter to take it, Esc to close the list.

You can not save while a field has a problem (for example "Give a value"): the form goes to that field and
shows the problem under it.

## The request
A form is a JSON object:

```json
{
  "title": "Git settings",
  "text": "~/.gitconfig",
  "fields": [
    { "type": "group", "label": "User" },
    { "id": "user.name", "type": "text", "label": "Name", "value": "Ann", "required": true },
    { "id": "user.email", "type": "text", "label": "Email", "pattern": "[^@ ]+@[^@ ]+",
      "help": "the email of your commits" },
    { "id": "commit.gpgsign", "type": "checkbox", "label": "Sign commits", "value": true },
    { "id": "commit.template", "type": "multiline", "label": "Commit template", "rows": 5 },
    { "id": "core.editor", "type": "radio", "label": "Editor",
      "options": ["vim", { "value": "code --wait", "label": "VS Code" }], "value": "vim" },
    { "id": "init.defaultBranch", "type": "select", "label": "Default branch",
      "options": ["main", "master", "trunk"], "value": "main" },
    { "id": "gc.auto", "type": "number", "label": "Auto gc", "value": 6700, "min": 0, "integer": true }
  ]
}
```

The field types:

| Type | The value | Its own keys |
|---|---|---|
| `group` | none (a heading) | `label` |
| `checkbox` | `true` or `false` | |
| `text` | one line of text | `required`, `pattern`, `placeholder`, `max` (chars) |
| `password` | one line, drawn as dots (the agent still gets the text) | the same as `text` |
| `number` | a number | `min`, `max`, `integer`, `required` |
| `multiline` | text with many lines | `rows` (how many are seen: 1 to 20, default 5), `required`, `pattern`, `placeholder`, `max` |
| `radio` | one of the options (up to 8, all are seen) | `options`, `required` |
| `select` | one of the options (up to 200, in a list that opens) | `options`, `required` |

The keys of every field (but `group`):
- `id`: any text (up to 200 chars), and every field has its own. The agent keeps its own map from the ids to
  the keys of its file, so a key like `url."https://x".insteadOf` needs no escaping.
- `label` (default: the id), `help` (a line under the field).
- `value`: the value now. No value (or `null`) shows as "(not set)".
- `clearable`: you may make the field "not set" (Delete). An empty text in such a field is "not set" too.
- An option is a text, a number, or `{ "value": ..., "label": "..." }`.

The rules:
- **A field that you do not change comes back exactly as it came**: the same type, the same text (also tabs and
  CR LF line ends), an integer stays an integer.
- A value that is not one of the options shows as one more option, with "(now)".
- `required`: the field must have a value before you can save, also when it had none.
- `pattern`, `max`, `min`, and `integer` are checked only for a field that you changed: an old value that breaks
  them never stops you. A `pattern` must match the whole value.
- A text field is one line: a paste with new lines becomes one line.
- The most: 200 fields, 64 KB of text in a field, a pattern of 1024 chars.
- A bad request is an error that says where: `fields[3].options[1]: ...`.

## The answer
You saved:

```json
{ "status": "saved", "values": { "user.name": "Ann", "commit.gpgsign": false }, "changed": ["commit.gpgsign"] }
```

`values` has all fields; `changed` has the ids whose value is not the one of the request.

You did not save (you cancelled, the time ran out, or the pane closed):

```json
{ "status": "cancelled", "changed": [] }
```

There are no `values` then, so an agent can not write them by mistake. When the time ran out, there is
`"timed_out": true` too.

## Use it
- **An agent with MCP:** the tool `show_form` (`title`, `text`, `fields`, `timeout_seconds`). When the agent
  stops waiting (its MCP client cancels the call, or the agent ends), the form closes.
- **A script:** `ftermctl form spec.json` (or `-` to read the JSON from stdin), `--timeout S`. It prints the
  answer as JSON. The exit code is 0 when you saved, 2 when you did not (or the command line was wrong),
  and 1 on another error.
- **The API:** the call `form` with the request and `from`, `pane`, and `timeout_ms` (see [API.md](API.md)).
  The time is 30 minutes by default, at most a day.

A form at a pane that is not the agent's own (`pane`) needs your yes first, as for `send_text` there (see
[API.md](API.md#who-may-do-what)). When the request gives a name (`from`) that is not the name of the program
that asked, the form shows both: `from claude (ftermctl)`.

The agent gets the text of every field, also of a `password` field (the dots only hide it on the screen). So the
agent, its AI provider, and its log see it.

## For agents: change a settings file with a form
1. Read the file. Make a field for each setting, with the value that is in the file now: a number as `number`,
   a setting that is not in the file as no value. Keep your map from each `id` to its key.
2. Show the form. On anything but `saved`, write nothing.
3. On `saved`: **read the file again**. For each id in `changed`: if the file still has the value that the form
   showed, set the new value (`null` = remove the setting). If the file changed in the meantime, ask the user
   before you write that key.
4. Make a copy of the file first (`<file>.bak-fterm`). Then change only those keys. Keep the rest of the file and
   its comments, and escape the values for the format (quotes in TOML and Lua, no new lines in .ini files).
5. For `fterm.lua`: the names and the defaults of the settings are in [CONFIG.md](CONFIG.md) and in
   `crates/fterm-config/src/sample.lua` (in the fterm repository). After you write it, fterm loads it again and
   shows the user "Config reloaded" or "Config error". You can not see it: ask the user. On an error, put the
   copy back.
