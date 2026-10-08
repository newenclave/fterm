//! A review of a list of items (for example a plan from an agent): the user walks the items and marks each
//! one Ok, comments on it, changes its text, adds an item, or removes one. The result goes back to the agent.
//! This file has no window code: the app draws `render` into a tab and gives it the keys.

use serde_json::{Value, json};

use crate::ai_chat::InputBox;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Heading,
    Item,
    Text,
    Code,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mark {
    #[default]
    None,
    Ok,
    Remove,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewItem {
    pub kind: Kind,
    /// How deep a list item is (0 = top level).
    pub depth: usize,
    pub text: String,
    pub mark: Mark,
    pub comment: Option<String>,
    /// The new text, when the user changed it.
    pub edited: Option<String>,
    /// The user added this item.
    pub added: bool,
}

impl ReviewItem {
    fn new(kind: Kind, depth: usize, text: String) -> Self {
        Self {
            kind,
            depth,
            text,
            mark: Mark::None,
            comment: None,
            edited: None,
            added: false,
        }
    }
}

/// A list item line: (indent, the text after the marker). Markers: `-`, `*`, `+`, `1.`, `1)`.
fn list_item(line: &str) -> Option<(usize, &str)> {
    let indent = line.len() - line.trim_start().len();
    let rest = line.trim_start();
    let after = if let Some(r) = rest.strip_prefix(['-', '*', '+']) {
        r
    } else {
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return None;
        }
        rest[digits..].strip_prefix(['.', ')'])?
    };
    after.strip_prefix(' ').map(|text| (indent, text.trim()))
}

/// The items of a markdown text: headings, list items (nested by indent), paragraphs, and code blocks.
pub fn parse_markdown(text: &str) -> Vec<ReviewItem> {
    let mut items: Vec<ReviewItem> = Vec::new();
    // The open item that more lines join (a paragraph or a list item), with its indent.
    let mut open: Option<usize> = None;
    let mut indents: Vec<usize> = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            open = None;
            continue;
        }
        if trimmed.starts_with("```") {
            let mut code = Vec::new();
            for line in lines.by_ref() {
                if line.trim_start().starts_with("```") {
                    break;
                }
                code.push(line);
            }
            items.push(ReviewItem::new(Kind::Code, 0, code.join("\n")));
            open = None;
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#') {
            let heading = heading.trim_start_matches('#').trim();
            items.push(ReviewItem::new(Kind::Heading, 0, heading.to_owned()));
            open = None;
            indents.clear();
            continue;
        }
        if let Some((indent, text)) = list_item(line) {
            // The depth: how many open list levels have a smaller indent.
            while indents.last().is_some_and(|&i| i >= indent) {
                indents.pop();
            }
            let depth = indents.len();
            indents.push(indent);
            items.push(ReviewItem::new(Kind::Item, depth, text.to_owned()));
            open = Some(items.len() - 1);
            continue;
        }
        match open {
            // A line that goes on the open paragraph or list item.
            Some(i) => {
                items[i].text.push('\n');
                items[i].text.push_str(trimmed);
            }
            None => {
                items.push(ReviewItem::new(Kind::Text, 0, trimmed.to_owned()));
                open = Some(items.len() - 1);
                indents.clear();
            }
        }
    }
    items
}

/// Items from a plain list of texts.
pub fn from_items(items: &[String]) -> Vec<ReviewItem> {
    items
        .iter()
        .map(|text| ReviewItem::new(Kind::Item, 0, text.trim().to_owned()))
        .collect()
}

/// A short form of a text for the feedback: its first line, at most 60 chars.
fn short(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() > 60 {
        format!("{}…", line.chars().take(59).collect::<String>())
    } else {
        line.to_owned()
    }
}

/// What the keys do now.
#[derive(Clone, Debug, Default)]
pub enum Mode {
    #[default]
    Browse,
    Comment(InputBox),
    Edit(InputBox),
    Add(InputBox),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// No comments and no changes.
    Approved,
    Changes,
    /// The user closed the review with no answer.
    Cancelled,
}

impl Decision {
    pub fn name(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Changes => "changes",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Review {
    pub title: String,
    /// Who asked (for example "claude").
    pub from: String,
    pub items: Vec<ReviewItem>,
    pub selected: usize,
    /// The first line on the screen (the renderer keeps the selection visible).
    pub scroll: usize,
    pub mode: Mode,
}

impl Review {
    pub fn new(title: &str, from: &str, items: Vec<ReviewItem>) -> Self {
        Self {
            title: title.to_owned(),
            from: from.to_owned(),
            items,
            selected: 0,
            scroll: 0,
            mode: Mode::Browse,
        }
    }

    /// Moves the selection.
    pub fn move_by(&mut self, delta: isize) {
        let last = self.items.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    fn current(&mut self) -> Option<&mut ReviewItem> {
        self.items.get_mut(self.selected)
    }

    /// Ok on or off for the selected item.
    pub fn toggle_ok(&mut self) {
        if let Some(item) = self.current() {
            item.mark = if item.mark == Mark::Ok {
                Mark::None
            } else {
                Mark::Ok
            };
        }
    }

    /// Remove on or off for the selected item; an added item goes away at once.
    pub fn toggle_remove(&mut self) {
        if self.items.get(self.selected).is_some_and(|i| i.added) {
            self.items.remove(self.selected);
            self.move_by(0);
            return;
        }
        if let Some(item) = self.current() {
            item.mark = if item.mark == Mark::Remove {
                Mark::None
            } else {
                Mark::Remove
            };
        }
    }

    /// All items with no mark and no change become Ok.
    pub fn rest_ok(&mut self) {
        for item in &mut self.items {
            if item.mark == Mark::None && item.comment.is_none() && item.edited.is_none() {
                item.mark = Mark::Ok;
            }
        }
    }

    fn open_input(&mut self, text: &str, mode: fn(InputBox) -> Mode) {
        if self.items.is_empty() && !matches!(mode(InputBox::default()), Mode::Add(_)) {
            return;
        }
        let mut input = InputBox::default();
        input.set(text);
        self.mode = mode(input);
    }

    /// Starts a comment on the selected item (with its old comment).
    pub fn start_comment(&mut self) {
        let old = self
            .items
            .get(self.selected)
            .and_then(|i| i.comment.clone())
            .unwrap_or_default();
        self.open_input(&old, Mode::Comment);
    }

    /// Starts a change of the text of the selected item.
    pub fn start_edit(&mut self) {
        let old = self
            .items
            .get(self.selected)
            .map(|i| i.edited.clone().unwrap_or_else(|| i.text.clone()))
            .unwrap_or_default();
        self.open_input(&old, Mode::Edit);
    }

    /// Starts a new item after the selected one.
    pub fn start_add(&mut self) {
        self.open_input("", Mode::Add);
    }

    /// The open input, if any.
    pub fn input_mut(&mut self) -> Option<&mut InputBox> {
        match &mut self.mode {
            Mode::Browse => None,
            Mode::Comment(input) | Mode::Edit(input) | Mode::Add(input) => Some(input),
        }
    }

    /// Saves the open input.
    pub fn save_input(&mut self) {
        let mode = std::mem::take(&mut self.mode);
        let selected = self.selected;
        match mode {
            Mode::Browse => {}
            Mode::Comment(input) => {
                let text = input.text.trim().to_owned();
                if let Some(item) = self.items.get_mut(selected) {
                    item.comment = (!text.is_empty()).then_some(text);
                }
            }
            Mode::Edit(input) => {
                let text = input.text.trim().to_owned();
                if let Some(item) = self.items.get_mut(selected) {
                    item.edited = (!text.is_empty() && text != item.text).then_some(text);
                }
            }
            Mode::Add(input) => {
                let text = input.text.trim().to_owned();
                if text.is_empty() {
                    return;
                }
                let depth = self.items.get(selected).map_or(0, |i| i.depth);
                let mut item = ReviewItem::new(Kind::Item, depth, text);
                item.added = true;
                let at = if self.items.is_empty() {
                    0
                } else {
                    selected + 1
                };
                self.items.insert(at, item);
                self.selected = at;
            }
        }
    }

    /// Closes the open input with no change.
    pub fn cancel_input(&mut self) {
        self.mode = Mode::Browse;
    }

    /// Approved when nothing was commented, changed, added, or removed.
    pub fn decision(&self) -> Decision {
        let changed = self.items.iter().any(|i| {
            i.added || i.mark == Mark::Remove || i.comment.is_some() || i.edited.is_some()
        });
        if changed {
            Decision::Changes
        } else {
            Decision::Approved
        }
    }

    /// The answer for the agent, in short markdown. Items are numbered as the agent sent them.
    pub fn feedback(&self, decision: Decision) -> String {
        match decision {
            Decision::Approved => {
                return "Approved: the user is fine with the whole plan.".to_owned();
            }
            Decision::Cancelled => {
                return "The user closed the review with no answer.".to_owned();
            }
            Decision::Changes => {}
        }
        let mut lines = vec!["Changes requested by the user (in the fterm review):".to_owned()];
        let mut number = 0;
        for item in &self.items {
            if item.added {
                let place = if number == 0 {
                    "At the start".to_owned()
                } else {
                    format!("After item {number}")
                };
                lines.push(format!("- {place}, add: \"{}\"", item.text));
                if let Some(comment) = &item.comment {
                    lines.push(format!("  (comment: {comment})"));
                }
                continue;
            }
            number += 1;
            let name = format!("Item {number} (\"{}\")", short(&item.text));
            if let Some(comment) = &item.comment {
                lines.push(format!("- {name}: comment: {comment}"));
            }
            if let Some(edited) = &item.edited {
                lines.push(format!("- {name}: change the text to: \"{edited}\""));
            }
            if item.mark == Mark::Remove {
                lines.push(format!("- {name}: remove it"));
            }
        }
        lines.push("The other items are fine.".to_owned());
        lines.join("\n")
    }

    /// The answer as JSON: the decision, the feedback text, and every item.
    pub fn result(&self, decision: Decision) -> Value {
        let items: Vec<Value> = self
            .items
            .iter()
            .map(|i| {
                json!({
                    "text": i.text,
                    "mark": match i.mark {
                        Mark::None => "none",
                        Mark::Ok => "ok",
                        Mark::Remove => "remove",
                    },
                    "comment": i.comment,
                    "edited": i.edited,
                    "added": i.added,
                })
            })
            .collect();
        json!({
            "decision": decision.name(),
            "feedback": self.feedback(decision),
            "items": items,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str = "# Themes for fterm

Some context
on two lines.

## Steps
1. Theme model (theme.rs)
   with tests
2. Config
   - a name
   - a table
- Docs

```rust
let x = 1;
```
";

    fn texts(items: &[ReviewItem]) -> Vec<(Kind, usize, &str)> {
        items
            .iter()
            .map(|i| (i.kind, i.depth, i.text.as_str()))
            .collect()
    }

    fn type_text(review: &mut Review, text: &str) {
        let input = review.input_mut().expect("an input is open");
        input.set("");
        input.insert(text);
    }

    #[test]
    fn markdown_becomes_items() {
        let items = parse_markdown(PLAN);
        assert_eq!(
            texts(&items),
            vec![
                (Kind::Heading, 0, "Themes for fterm"),
                (Kind::Text, 0, "Some context\non two lines."),
                (Kind::Heading, 0, "Steps"),
                (Kind::Item, 0, "Theme model (theme.rs)\nwith tests"),
                (Kind::Item, 0, "Config"),
                (Kind::Item, 1, "a name"),
                (Kind::Item, 1, "a table"),
                (Kind::Item, 0, "Docs"),
                (Kind::Code, 0, "let x = 1;"),
            ]
        );
        assert!(parse_markdown("\n\n").is_empty());
        let plain = from_items(&["one".to_owned(), "two".to_owned()]);
        assert_eq!(
            texts(&plain),
            vec![(Kind::Item, 0, "one"), (Kind::Item, 0, "two")]
        );
    }

    fn review() -> Review {
        Review::new(
            "plan",
            "claude",
            from_items(&["one".into(), "two".into(), "three".into()]),
        )
    }

    #[test]
    fn moving_stays_in_the_list() {
        let mut r = review();
        r.move_by(-1);
        assert_eq!(r.selected, 0);
        r.move_by(5);
        assert_eq!(r.selected, 2);
        r.move_by(-1);
        assert_eq!(r.selected, 1);
    }

    #[test]
    fn ok_and_remove_toggle() {
        let mut r = review();
        r.toggle_ok();
        assert_eq!(r.items[0].mark, Mark::Ok);
        r.toggle_ok();
        assert_eq!(r.items[0].mark, Mark::None);
        r.toggle_remove();
        assert_eq!(r.items[0].mark, Mark::Remove);
        r.toggle_ok();
        assert_eq!(r.items[0].mark, Mark::Ok, "Ok takes the place of Remove");
        r.move_by(1);
        r.rest_ok();
        assert!(r.items.iter().all(|i| i.mark == Mark::Ok));
        assert_eq!(r.decision(), Decision::Approved);
    }

    #[test]
    fn comment_edit_and_add() {
        let mut r = review();
        r.move_by(1);
        r.start_comment();
        type_text(&mut r, "why two?");
        r.save_input();
        assert!(matches!(r.mode, Mode::Browse));
        assert_eq!(r.items[1].comment.as_deref(), Some("why two?"));
        // The comment opens again with its text; an empty one goes away.
        r.start_comment();
        assert_eq!(r.input_mut().unwrap().text, "why two?");
        type_text(&mut r, "  ");
        r.save_input();
        assert_eq!(r.items[1].comment, None);

        r.start_edit();
        assert_eq!(r.input_mut().unwrap().text, "two");
        type_text(&mut r, "2nd");
        r.save_input();
        assert_eq!(r.items[1].edited.as_deref(), Some("2nd"));
        // The same text again is no change.
        r.start_edit();
        type_text(&mut r, "two");
        r.save_input();
        assert_eq!(r.items[1].edited, None);

        r.start_add();
        type_text(&mut r, "two and a half");
        r.save_input();
        assert_eq!(r.items.len(), 4);
        assert_eq!(r.selected, 2, "the new item is selected");
        assert!(r.items[2].added);
        assert_eq!(r.items[2].text, "two and a half");
        // Remove on an added item takes it away.
        r.toggle_remove();
        assert_eq!(r.items.len(), 3);
        // Esc: no change.
        r.start_comment();
        type_text(&mut r, "x");
        r.cancel_input();
        assert!(r.items.iter().all(|i| i.comment.is_none()));
    }

    #[test]
    fn the_feedback_for_the_agent() {
        let mut r = review();
        r.toggle_ok();
        r.move_by(1);
        r.start_comment();
        type_text(&mut r, "use a table");
        r.save_input();
        r.start_edit();
        type_text(&mut r, "TWO");
        r.save_input();
        r.start_add();
        type_text(&mut r, "a new step");
        r.save_input();
        r.move_by(1);
        r.toggle_remove();
        assert_eq!(r.decision(), Decision::Changes);
        let text = r.feedback(Decision::Changes);
        assert!(text.starts_with("Changes requested"), "{text}");
        assert!(
            text.contains(r#"Item 2 ("two"): comment: use a table"#),
            "{text}"
        );
        assert!(
            text.contains(r#"Item 2 ("two"): change the text to: "TWO""#),
            "{text}"
        );
        assert!(
            text.contains(r#"After item 2, add: "a new step""#),
            "{text}"
        );
        assert!(text.contains(r#"Item 3 ("three"): remove it"#), "{text}");
        assert!(!text.contains("Item 1"), "an Ok item is not listed: {text}");

        let json = r.result(Decision::Changes);
        assert_eq!(json["decision"], "changes");
        assert_eq!(json["feedback"], text);
        assert_eq!(json["items"][0]["mark"], "ok");
        assert_eq!(json["items"][1]["comment"], "use a table");
        assert_eq!(json["items"][1]["edited"], "TWO");
        assert_eq!(json["items"][2]["added"], true);
        assert_eq!(json["items"][3]["mark"], "remove");

        assert!(
            review()
                .feedback(Decision::Approved)
                .starts_with("Approved")
        );
        assert!(review().feedback(Decision::Cancelled).contains("closed"));
    }
}
