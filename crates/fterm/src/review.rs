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

/// How a part of a line looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Title,
    Dim,
    Text,
    Heading,
    Ok,
    Remove,
    Changed,
    Comment,
    Code,
    Input,
    Cursor,
}

/// A part of a screen line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub look: Look,
}

/// One screen line: its parts, the item it belongs to, and the selection background.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub spans: Vec<Span>,
    pub selected: bool,
}

impl Row {
    /// The text with no looks (for tests).
    #[cfg(test)]
    pub fn plain(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

/// A key for a review, from the window (the app turns winit keys into these).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key<'a> {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Left,
    Right,
    Enter {
        ctrl: bool,
        shift: bool,
    },
    Escape,
    Backspace,
    Delete,
    /// Typed text (Space too).
    Text(&'a str),
}

/// What a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The key is not for the review.
    Nothing,
    /// The review changed: draw it again.
    Changed,
    /// The user sends the review.
    Send,
}

impl Review {
    /// One key: `page` = how many items a PageUp or PageDown moves.
    pub fn key(&mut self, key: Key, page: usize) -> Outcome {
        let page = page.max(1) as isize;
        if let Some(input) = self.input_mut() {
            match key {
                Key::Enter {
                    shift: true,
                    ctrl: false,
                } => input.insert("\n"),
                Key::Enter { .. } => self.save_input(),
                Key::Escape => self.cancel_input(),
                Key::Backspace => input.backspace(),
                Key::Delete => input.delete(),
                Key::Left => input.left(),
                Key::Right => input.right(),
                Key::Home => input.home(),
                Key::End => input.end(),
                Key::Text(text) => input.insert(text),
                Key::Up | Key::Down | Key::PageUp | Key::PageDown => return Outcome::Nothing,
            }
            return Outcome::Changed;
        }
        match key {
            Key::Up => self.move_by(-1),
            Key::Down => self.move_by(1),
            Key::PageUp => self.move_by(-page),
            Key::PageDown => self.move_by(page),
            Key::Home => self.selected = 0,
            Key::End => self.move_by(isize::MAX / 2),
            Key::Delete => self.toggle_remove(),
            Key::Enter { ctrl: true, .. } => return Outcome::Send,
            Key::Text(text) => match text {
                " " | "o" => self.toggle_ok(),
                "O" => self.rest_ok(),
                "c" | "C" => self.start_comment(),
                "e" | "E" => self.start_edit(),
                "a" | "A" => self.start_add(),
                "d" | "D" => self.toggle_remove(),
                "s" | "S" => return Outcome::Send,
                _ => return Outcome::Nothing,
            },
            _ => return Outcome::Nothing,
        }
        Outcome::Changed
    }
}

/// The width of a text in cells.
fn cells(text: &str) -> usize {
    use unicode_width::UnicodeWidthChar;
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// The first `width` cells of a text.
fn fit(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out
}

fn span(text: impl Into<String>, look: Look) -> Span {
    Span {
        text: text.into(),
        look,
    }
}

/// The lines of the open input under a label, with the cursor cell. `pad` = the cells on the left.
fn input_rows(label: &str, input: &InputBox, pad: usize, width: usize) -> Vec<Vec<Span>> {
    let mut out = vec![vec![
        span(" ".repeat(pad), Look::Dim),
        span(label, Look::Dim),
    ]];
    let (lines, (cursor_line, cursor_cell)) = input.layout(width.max(1));
    for (n, line) in lines.iter().enumerate() {
        let mut row = vec![span(" ".repeat(pad), Look::Dim)];
        if n == cursor_line {
            let before = fit(line, cursor_cell);
            let rest: String = line.chars().skip(before.chars().count()).collect();
            let mut chars = rest.chars();
            let under = chars.next().map_or(" ".to_owned(), String::from);
            row.push(span(before, Look::Input));
            row.push(span(under, Look::Cursor));
            row.push(span(chars.as_str(), Look::Input));
        } else {
            row.push(span(line.clone(), Look::Input));
        }
        out.push(row);
    }
    out
}

/// The screen lines of the review for `cols` x `rows` cells. It moves `scroll` so the selection is seen.
pub fn rows(review: &mut Review, cols: usize, rows: usize) -> Vec<Row> {
    let cols = cols.max(20);
    let row = |spans: Vec<Span>, selected: bool| Row { spans, selected };
    // The top: the title and who asked, and a short count.
    let from = format!("from {}", review.from);
    let title_room = cols.saturating_sub(cells(&from) + 2);
    let title = fit(&review.title, title_room);
    let gap = cols.saturating_sub(cells(&title) + cells(&from));
    let ok = review.items.iter().filter(|i| i.mark == Mark::Ok).count();
    let changed = review
        .items
        .iter()
        .filter(|i| i.added || i.mark == Mark::Remove || i.comment.is_some() || i.edited.is_some())
        .count();
    let summary = format!(
        "{} items · {ok} ok · {changed} with changes or comments",
        review.items.len()
    );
    let header = vec![
        row(
            vec![
                span(title, Look::Title),
                span(" ".repeat(gap), Look::Dim),
                span(from, Look::Dim),
            ],
            false,
        ),
        row(vec![span(fit(&summary, cols), Look::Dim)], false),
        row(Vec::new(), false),
    ];

    // The items, with their comments and the open input.
    let mut body: Vec<(Row, usize)> = Vec::new();
    let mut number = 0;
    for (i, item) in review.items.iter().enumerate() {
        let selected = i == review.selected;
        let num = if item.added {
            String::new()
        } else {
            number += 1;
            format!("{number}. ")
        };
        let (mark, mark_look) = if item.added {
            ('+', Look::Changed)
        } else if item.mark == Mark::Remove {
            ('✗', Look::Remove)
        } else if item.edited.is_some() {
            ('~', Look::Changed)
        } else if item.comment.is_some() {
            ('»', Look::Comment)
        } else if item.mark == Mark::Ok {
            ('✓', Look::Ok)
        } else {
            (' ', Look::Dim)
        };
        let look = match (item.kind, item.mark) {
            (_, Mark::Remove) => Look::Remove,
            _ if item.edited.is_some() => Look::Changed,
            (Kind::Heading, _) => Look::Heading,
            (Kind::Code, _) => Look::Code,
            _ => Look::Text,
        };
        let indent = (item.depth * 2).min(cols / 3);
        let pad = 3 + indent + cells(&num);
        let width = cols.saturating_sub(pad).max(8);
        let text = item.edited.as_deref().unwrap_or(&item.text);
        let mut first = true;
        for source in text.lines() {
            for line in crate::ai_chat::wrap(source, width) {
                let lead = if first {
                    vec![
                        span(if selected { "▶" } else { " " }, Look::Title),
                        span(mark.to_string(), mark_look),
                        span(" ".repeat(1 + indent), Look::Dim),
                        span(num.clone(), Look::Dim),
                    ]
                } else {
                    vec![span(" ".repeat(pad), Look::Dim)]
                };
                first = false;
                let mut spans = lead;
                spans.push(span(line, look));
                body.push((row(spans, selected), i));
            }
        }
        if item.edited.is_some() {
            let was = fit(
                &format!("was: {}", item.text.lines().next().unwrap_or("")),
                width,
            );
            body.push((
                row(
                    vec![span(" ".repeat(pad), Look::Dim), span(was, Look::Dim)],
                    selected,
                ),
                i,
            ));
        }
        if let Some(comment) = &item.comment {
            for (n, line) in comment
                .lines()
                .flat_map(|l| crate::ai_chat::wrap(l, width.saturating_sub(2)))
                .enumerate()
            {
                let lead = if n == 0 { "└ " } else { "  " };
                body.push((
                    row(
                        vec![
                            span(" ".repeat(pad), Look::Dim),
                            span(format!("{lead}{line}"), Look::Comment),
                        ],
                        selected,
                    ),
                    i,
                ));
            }
        }
        if selected {
            let open = match &review.mode {
                Mode::Browse => None,
                Mode::Comment(input) => Some(("Comment:", input)),
                Mode::Edit(input) => Some(("New text:", input)),
                Mode::Add(input) => Some(("New item after this one:", input)),
            };
            if let Some((label, input)) = open {
                for spans in input_rows(label, input, pad, width) {
                    body.push((row(spans, false), i));
                }
            }
        }
    }

    // Keep the selection on the screen.
    let height = rows.saturating_sub(header.len() + 1).max(1);
    let first = body.iter().position(|(_, i)| *i == review.selected);
    let last = body.iter().rposition(|(_, i)| *i == review.selected);
    if let (Some(first), Some(last)) = (first, last) {
        if first < review.scroll {
            review.scroll = first;
        } else if last >= review.scroll + height {
            review.scroll = (last + 1 - height).min(first);
        }
    }
    review.scroll = review.scroll.min(body.len().saturating_sub(height));

    let hints = match review.mode {
        Mode::Browse => {
            "S send · Space ok · C comment · E edit · A add · D remove · Shift+O rest ok · ↑↓"
        }
        _ => "Enter save · Shift+Enter new line · Esc cancel",
    };
    let mut out = header;
    out.extend(
        body.into_iter()
            .skip(review.scroll)
            .take(height)
            .map(|(r, _)| r),
    );
    while out.len() + 1 < rows.max(2) {
        out.push(row(Vec::new(), false));
    }
    out.push(row(vec![span(fit(hints, cols), Look::Dim)], false));
    // Every row fits the width.
    for r in &mut out {
        let mut used = 0;
        for s in &mut r.spans {
            let room = cols.saturating_sub(used);
            if cells(&s.text) > room {
                s.text = fit(&s.text, room);
            }
            used += cells(&s.text);
        }
    }
    out
}

/// The review as ANSI text for the grid: from the top left, colors from the theme.
pub fn render(
    review: &mut Review,
    cols: usize,
    rows_n: usize,
    ui: &fterm_render::theme::UiColors,
) -> String {
    use std::fmt::Write;
    let fg = |c: fterm_term::alacritty_terminal::vte::ansi::Rgb| {
        format!("\x1b[38;2;{};{};{}m", c.r, c.g, c.b)
    };
    let bg = |c: fterm_term::alacritty_terminal::vte::ansi::Rgb| {
        format!("\x1b[48;2;{};{};{}m", c.r, c.g, c.b)
    };
    let mut out = String::from("\x1b[H\x1b[?25l");
    let lines = rows(review, cols, rows_n);
    let count = lines.len();
    for (n, row) in lines.into_iter().enumerate() {
        let back = if row.selected {
            bg(ui.selected)
        } else {
            "\x1b[49m".to_owned()
        };
        out.push_str("\x1b[0m");
        out.push_str(&back);
        for s in &row.spans {
            let style = match s.look {
                Look::Title | Look::Heading => format!("\x1b[1m{}", fg(ui.accent)),
                Look::Dim | Look::Code => fg(ui.text_dim),
                Look::Text => fg(ui.text),
                Look::Ok => fg(ui.success),
                Look::Remove => format!("\x1b[9m{}", fg(ui.error)),
                Look::Changed => fg(ui.warning),
                Look::Comment => fg(ui.info),
                Look::Input => format!("{}{}", fg(ui.text), bg(ui.input_bg)),
                Look::Cursor => format!("{}{}", fg(ui.surface_active), bg(ui.text)),
            };
            let _ = write!(out, "\x1b[0m{back}{style}{}", s.text);
        }
        // The rest of the line in the row's background.
        let _ = write!(out, "\x1b[0m{back}\x1b[K\x1b[0m");
        if n + 1 < count {
            out.push_str("\r\n");
        }
    }
    out
}

/// What a `review` call does now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// Open the Review tab.
    Open,
    /// Ask the user first.
    Ask,
    /// No review: the caller (the plan mode hook) lets Claude Code show its own dialog.
    Skip,
}

/// A plan of the plan mode hook follows `plan_review`; a review that an agent or a script asked for opens.
pub fn start(plan_mode: bool, mode: fterm_config::load::PlanReview) -> Start {
    use fterm_config::load::PlanReview as P;
    match (plan_mode, mode) {
        (false, _) | (true, P::Always) => Start::Open,
        (true, P::Ask) => Start::Ask,
        (true, P::Never) => Start::Skip,
    }
}

/// The answer when there is no review.
pub fn skipped() -> Value {
    json!({ "decision": "skipped", "feedback": "", "items": [] })
}

/// The question before a plan of plan mode opens in a Review tab.
pub fn question_lines(from: &str, title: &str) -> Vec<String> {
    vec![
        format!("{from} has a plan: {title}"),
        String::new(),
        "R = review it here, item by item".to_owned(),
        format!("Esc = the dialog of {from}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_of_plan_mode_follows_plan_review() {
        use fterm_config::load::PlanReview as P;
        assert_eq!(start(true, P::Always), Start::Open);
        assert_eq!(start(true, P::Ask), Start::Ask);
        assert_eq!(start(true, P::Never), Start::Skip);
        // An agent or a script asked for the review itself: it always opens.
        for mode in [P::Always, P::Ask, P::Never] {
            assert_eq!(start(false, mode), Start::Open);
        }
    }

    #[test]
    fn a_skipped_review() {
        let out = skipped();
        assert_eq!(out["decision"], "skipped");
        assert_eq!(out["items"], json!([]));
    }

    #[test]
    fn the_question_before_a_plan_review() {
        let lines = question_lines("claude", "Themes");
        assert_eq!(lines[0], "claude has a plan: Themes");
        assert!(lines.iter().any(|l| l.contains("R = review it here")));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("Esc = the dialog of claude"))
        );
    }

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

    fn plain(rows: &[Row]) -> Vec<String> {
        rows.iter().map(Row::plain).collect()
    }

    #[test]
    fn the_screen_of_a_review() {
        let mut r = Review::new("Themes plan", "claude", parse_markdown(PLAN));
        r.move_by(3);
        r.toggle_ok();
        r.move_by(1);
        r.start_comment();
        type_text(&mut r, "and light/dark");
        r.save_input();
        let screen = rows(&mut r, 60, 30);
        let text = plain(&screen);
        assert_eq!(screen.len(), 30, "one row per line of the grid");
        assert!(
            text[0].contains("Themes plan") && text[0].contains("claude"),
            "{}",
            text[0]
        );
        assert!(text.iter().any(|l| l.contains("Steps")));
        // The number the agent sees in the feedback, and the mark.
        let model = text.iter().find(|l| l.contains("Theme model")).unwrap();
        // Headings and text count too: the agent gets the same numbers in the feedback.
        assert!(model.contains('✓') && model.contains("4."), "{model}");
        let config = text.iter().position(|l| l.contains("Config")).unwrap();
        assert!(
            screen[config].selected,
            "the selected item has the selection"
        );
        assert!(
            text[config + 1].contains("and light/dark"),
            "the comment is under it"
        );
        // A nested item is further right.
        let top = text.iter().find(|l| l.contains("Config")).unwrap();
        let nested = text.iter().find(|l| l.contains("a name")).unwrap();
        let column = |line: &str, word: &str| line[..line.find(word).unwrap()].chars().count();
        assert!(column(nested, "a name") > column(top, "Config"));
        // Key hints at the bottom.
        assert!(
            text[29].contains("send") || text[28].contains("send"),
            "{:?}",
            &text[27..]
        );
        for line in &text {
            assert!(line.chars().count() <= 60, "too wide: {line}");
        }
    }

    #[test]
    fn the_selection_is_always_on_the_screen() {
        let items: Vec<String> = (1..=100).map(|n| format!("step {n}")).collect();
        let mut r = Review::new("long", "claude", from_items(&items));
        r.move_by(80);
        let screen = rows(&mut r, 40, 20);
        let text = plain(&screen);
        let i = text
            .iter()
            .position(|l| l.contains("step 81"))
            .expect("on the screen");
        assert!(screen[i].selected);
        r.move_by(-80);
        let text = plain(&rows(&mut r, 40, 20));
        assert!(
            text.iter()
                .any(|l| l.contains("step 1 ") || l.ends_with("step 1"))
        );
    }

    #[test]
    fn an_open_input_shows_its_text_and_a_cursor() {
        let mut r = review();
        r.start_edit();
        type_text(&mut r, "the new text");
        let screen = rows(&mut r, 50, 15);
        let text = plain(&screen);
        assert!(text.iter().any(|l| l.contains("the new text")));
        assert!(
            screen
                .iter()
                .any(|row| row.spans.iter().any(|s| s.look == Look::Cursor)),
            "a cursor cell"
        );
        assert!(
            text.iter().any(|l| l.contains("Enter save")),
            "the hints of the input"
        );
        let ansi = render(&mut r, 50, 15, &fterm_render::theme::UiColors::default());
        assert!(ansi.starts_with("\x1b[H"), "from the top left");
        assert!(ansi.contains("38;2;"), "colors of the theme");
        assert!(ansi.contains("the new text"));
    }

    #[test]
    fn keys_in_the_list() {
        let mut r = review();
        assert_eq!(r.key(Key::Down, 10), Outcome::Changed);
        assert_eq!(r.selected, 1);
        assert_eq!(r.key(Key::Text(" "), 10), Outcome::Changed);
        assert_eq!(r.items[1].mark, Mark::Ok);
        r.key(Key::Text("d"), 10);
        assert_eq!(r.items[1].mark, Mark::Remove);
        r.key(Key::Text("o"), 10);
        assert_eq!(r.items[1].mark, Mark::Ok);
        r.key(Key::End, 10);
        assert_eq!(r.selected, 2);
        r.key(Key::Home, 10);
        assert_eq!(r.selected, 0);
        r.key(Key::Text("O"), 10);
        assert!(
            r.items.iter().all(|i| i.mark == Mark::Ok),
            "Shift+O: the rest Ok"
        );
        assert_eq!(r.key(Key::Text("s"), 10), Outcome::Send);
        assert_eq!(
            r.key(
                Key::Enter {
                    ctrl: true,
                    shift: false
                },
                10
            ),
            Outcome::Send
        );
        assert_eq!(r.key(Key::Text("z"), 10), Outcome::Nothing);
        assert_eq!(
            r.key(Key::Escape, 10),
            Outcome::Nothing,
            "Esc in the list is not ours"
        );
    }

    #[test]
    fn keys_in_an_input() {
        let mut r = review();
        r.key(Key::Text("c"), 10);
        assert!(matches!(r.mode, Mode::Comment(_)));
        // Letters are text now, not commands.
        for t in ["s", "o", " ", "k"] {
            assert_eq!(r.key(Key::Text(t), 10), Outcome::Changed);
        }
        r.key(
            Key::Enter {
                ctrl: false,
                shift: true,
            },
            10,
        );
        r.key(Key::Text("x"), 10);
        r.key(Key::Backspace, 10);
        r.key(Key::Text("y"), 10);
        r.key(
            Key::Enter {
                ctrl: false,
                shift: false,
            },
            10,
        );
        assert!(matches!(r.mode, Mode::Browse));
        assert_eq!(r.items[0].comment.as_deref(), Some("so k\ny"));
        r.key(Key::Text("e"), 10);
        r.key(Key::Text("!"), 10);
        r.key(Key::Escape, 10);
        assert!(matches!(r.mode, Mode::Browse));
        assert_eq!(r.items[0].edited, None, "Esc: no change");
        r.key(Key::Text("a"), 10);
        r.key(Key::Text("new"), 10);
        r.key(
            Key::Enter {
                ctrl: true,
                shift: false,
            },
            10,
        );
        assert_eq!(r.items[1].text, "new", "Ctrl+Enter in an input saves it");
    }
}
