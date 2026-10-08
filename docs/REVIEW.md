# Review a plan

An agent (Claude Code, OpenCode, a script) can show you its plan in a **Review tab**. You go through the plan
item by item: mark an item Ok, write a comment, change its text, add an item, or remove one. Then you send it,
and the agent gets your answer: "approved", or the list of your changes and comments.

## How it looks
A new tab "Review: <the title>" opens, with a yellow dot and a notification. It becomes the active tab when
you look at the pane that asked; else it waits until you go there (click the notification, or the tab).

The tab shows the title, who asked, and every item of the plan with its number: headings, paragraphs,
list items (nested items are further right), and code blocks. The agent gets the same numbers in the answer.
A mark on the left says what you did:

| Mark | Meaning |
|---|---|
| `✓` | Ok |
| `»` | a comment (it is under the item) |
| `~` | you changed the text ("was: …" is under it) |
| `✗` | remove this item (the text is crossed out) |
| `+` | an item that you added |

## Keys
| Key | What it does |
|---|---|
| Up / Down, PageUp / PageDown, Home / End | Choose an item. |
| Space, O | Ok on or off. |
| Shift + O | All items with no mark become Ok. |
| C | A comment on the item (it opens with the old comment). |
| E | Change the text of the item. |
| A | Add a new item after this one. |
| D, Delete | Remove on or off (an added item goes away at once). |
| S, Ctrl + Enter | Send the review to the agent. The tab closes. |
| Ctrl + Shift + W | Close the tab with no answer ("cancelled"). |

In a comment or a text: Enter saves, Shift + Enter makes a new line, Esc cancels.
Other keys (tabs, the palette) work as usual.

## The answer
- **approved**: you made no comment and no change (Ok marks or no marks).
- **changes**: there is at least one comment, change, new item, or removed item. The feedback text names the
  items by number, for example:
  ```
  Changes requested by the user (in the fterm review):
  - Item 5 ("Config: theme = name"): comment: use a table too
  - Item 6 ("light and dark by the system"): change the text to: "light and dark by the theme"
  - Item 7 ("Renderer without colors"): remove it
  - After item 7, add: "add docs"
  The other items are fine.
  ```
- **cancelled**: you closed the tab, or the time ran out (one hour by default; then also `timed_out`).

## Claude Code plan mode
In the command palette run **Install Claude Code hooks (agent states)** (see [CLAUDE.md](CLAUDE.md)). It also adds
a `PreToolUse` hook for `ExitPlanMode`. Then, when Claude finishes a plan in plan mode:
- fterm shows the plan in a Review tab;
- **approved** → the plan is accepted, and Claude starts the work;
- **changes** → Claude gets your feedback, stays in plan mode, and makes a new plan (which comes back to you);
- **cancelled**, or Claude runs outside fterm → Claude shows its own plan dialog, as usual.

The hook waits for you up to one hour. It runs `ftermctl review --hook` (ftermctl is next to fterm.exe).

## Other agents and scripts
- MCP tool `review_plan` with `plan` (markdown) and `title` (see [MCP.md](MCP.md)).
- `ftermctl review plan.md [--title T] [--timeout S]` (or `-` for stdin): it waits, prints the feedback, and
  exits with 0 (approved), 1 (changes), or 2 (cancelled). `--json` prints everything as JSON.
- The API method `review` (see [API.md](API.md)): `text` (markdown) or `items` (a list of texts).
