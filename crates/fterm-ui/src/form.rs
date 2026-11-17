//! A form: fields from a JSON request, the focus, the keys, the screen rows, and the answer. An agent sends the
//! request with the values that it read from a settings file; the user changes them; the answer has the new
//! values and the ids that changed. Untouched values come back exactly as they came.

use serde_json::{Map, Value, json};

use crate::field::{Field, Handled, Hint, Kind, Word, help_rows, tidy, tidy_lines};
use crate::key::Key;
use crate::look::{Look, Row, Span, span};
use crate::text::{cells, fit, wrap};

/// The most fields of a form.
pub const MAX_FIELDS: usize = 200;

/// The form is laid out for at least this many cells (a narrower pane cuts the rows).
const MIN_COLS: usize = 20;

/// What has the focus: a field, or one of the buttons at the bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Field(usize),
    Save,
    Cancel,
}

/// What a key did to the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    /// Draw it again.
    Changed,
    /// The user saves: all changed fields are valid.
    Save,
    /// The user cancels.
    Cancel,
}

/// The rows of the screen and the place of the cursor in them (for the input method box).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    pub rows: Vec<Row>,
    pub cursor: Option<(usize, usize)>,
}

#[derive(Clone, Debug)]
pub struct Form {
    pub title: String,
    pub text: String,
    /// Who asked (the app sets it, as for a Review).
    pub from: String,
    fields: Vec<Field>,
    focus: Focus,
    /// The first row of the fields that is seen.
    top: usize,
    /// A save was tried: problems show under all changed fields.
    tried: bool,
    /// Esc was pressed once with changes: the next Esc cancels.
    esc_armed: bool,
}

impl Form {
    /// Reads a request. An error names the place: `fields[3].options[1]: ...`.
    pub fn from_json(v: &Value) -> Result<Form, String> {
        let Value::Object(map) = v else {
            return Err("the form: expected an object".to_owned());
        };
        let text = |key: &str| -> Result<String, String> {
            match map.get(key) {
                None | Some(Value::Null) => Ok(String::new()),
                Some(Value::String(s)) => Ok(s.clone()),
                Some(_) => Err(format!("{key}: expected a text")),
            }
        };
        let Some(Value::Array(list)) = map.get("fields") else {
            return Err("fields: missing (a list of fields)".to_owned());
        };
        if list.len() > MAX_FIELDS {
            return Err(format!(
                "fields: {} fields, at most {MAX_FIELDS}",
                list.len()
            ));
        }
        let mut fields: Vec<Field> = Vec::new();
        for (i, item) in list.iter().enumerate() {
            let field = Field::from_json(item, &format!("fields[{i}]"))?;
            if !field.is_group() && fields.iter().any(|f| f.id == field.id) {
                return Err(format!("fields[{i}].id: `{}` is there already", field.id));
            }
            fields.push(field);
        }
        let first = fields
            .iter()
            .position(|f| !f.is_group())
            .ok_or("fields: no field with a value")?;
        Ok(Form {
            title: tidy(&text("title")?),
            text: tidy_lines(&text("text")?),
            from: String::new(),
            fields,
            focus: Focus::Field(first),
            top: 0,
            tried: false,
            esc_armed: false,
        })
    }

    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// The user edited something (Esc then asks again).
    pub fn changed(&self) -> bool {
        self.fields.iter().any(Field::edited)
    }

    /// The answer of a saved form: all values (untouched ones exactly as they came) and the ids that changed.
    pub fn answer(&self) -> Value {
        let mut values = Map::new();
        let mut changed = Vec::new();
        for f in self.fields.iter().filter(|f| !f.is_group()) {
            values.insert(f.id.clone(), f.value());
            if f.changed() {
                changed.push(Value::String(f.id.clone()));
            }
        }
        json!({ "status": "saved", "values": values, "changed": changed })
    }

    /// The answer of a form that was not saved: no values, so an agent can not write them by mistake.
    pub fn cancelled() -> Value {
        json!({ "status": "cancelled", "changed": [] })
    }

    /// The focus targets in order: the fields with a value, then Save and Cancel.
    fn targets(&self) -> Vec<Focus> {
        self.fields
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.is_group())
            .map(|(i, _)| Focus::Field(i))
            .chain([Focus::Save, Focus::Cancel])
            .collect()
    }

    fn step(&mut self, by: isize, wrap_around: bool) {
        let targets = self.targets();
        let at = targets.iter().position(|t| *t == self.focus).unwrap_or(0) as isize;
        let n = targets.len() as isize;
        let next = if wrap_around {
            (at + by).rem_euclid(n)
        } else {
            (at + by).clamp(0, n - 1)
        };
        self.focus = targets[next as usize];
    }

    /// The cells of the label column and of the values for `cols` cells.
    fn widths(&self, cols: usize) -> (usize, usize) {
        let cols = cols.max(MIN_COLS);
        let longest = self
            .fields
            .iter()
            .filter(|f| !f.is_group())
            .map(|f| cells(&f.label))
            .max()
            .unwrap_or(4);
        let label_w = longest.clamp(4, (cols / 3).max(4));
        let value_w = cols.saturating_sub(label_w + 4).max(4);
        (label_w, value_w)
    }

    /// Tries to save: the first changed field that is not valid gets the focus.
    fn save(&mut self) -> Outcome {
        self.tried = true;
        match self.fields.iter().position(|f| f.problem().is_some()) {
            Some(bad) => {
                self.focus = Focus::Field(bad);
                Outcome::Changed
            }
            None => Outcome::Save,
        }
    }

    /// One key; `cols` = the width of the form in cells.
    pub fn key(&mut self, key: Key, cols: usize) -> Outcome {
        let was_armed = self.esc_armed;
        let out = self.key_inner(key, cols);
        // The "Esc again" hint went: draw again even when the key did nothing else.
        if was_armed && !self.esc_armed && out == Outcome::Nothing {
            return Outcome::Changed;
        }
        out
    }

    fn key_inner(&mut self, key: Key, cols: usize) -> Outcome {
        let armed = std::mem::replace(&mut self.esc_armed, false);
        let (_, value_w) = self.widths(cols);
        // An open select list keeps its keys (Esc closes it).
        if let Focus::Field(i) = self.focus
            && self.fields[i].list_open()
        {
            self.fields[i].key(key, value_w);
            return Outcome::Changed;
        }
        match key {
            Key::Save | Key::Enter { ctrl: true, .. } => return self.save(),
            Key::Escape => {
                if self.changed() && !armed {
                    self.esc_armed = true;
                    return Outcome::Changed;
                }
                return Outcome::Cancel;
            }
            Key::Tab => {
                self.step(1, true);
                return Outcome::Changed;
            }
            Key::BackTab => {
                self.step(-1, true);
                return Outcome::Changed;
            }
            _ => {}
        }
        match self.focus {
            Focus::Save | Focus::Cancel => match key {
                Key::Enter { .. } | Key::Text(" ") => {
                    return if self.focus == Focus::Save {
                        self.save()
                    } else {
                        Outcome::Cancel
                    };
                }
                Key::Left | Key::Right => {
                    self.focus = if self.focus == Focus::Save {
                        Focus::Cancel
                    } else {
                        Focus::Save
                    };
                }
                Key::Up => self.step(-1, false),
                Key::Down => self.step(1, false),
                Key::Home => self.focus = self.targets()[0],
                Key::PageUp => self.step(-5, false),
                _ => return Outcome::Nothing,
            },
            Focus::Field(i) => {
                if self.fields[i].key(key, value_w) == Handled::Used {
                    return Outcome::Changed;
                }
                match key {
                    Key::Up => self.step(-1, false),
                    Key::Down | Key::Enter { .. } => self.step(1, false),
                    Key::PageUp => self.step(-5, false),
                    Key::PageDown => self.step(5, false),
                    Key::Home => self.focus = self.targets()[0],
                    Key::End => self.focus = Focus::Save,
                    _ => return Outcome::Nothing,
                }
            }
        }
        Outcome::Changed
    }

    /// The screen of the form for `cols` x `lines` cells: the header, the fields (scrolled so the cursor is seen),
    /// the buttons, and the keys.
    pub fn screen(&mut self, pane: usize, lines: usize, word: &dyn Fn(Word) -> String) -> Screen {
        let cols = pane.max(MIN_COLS);
        let (label_w, value_w) = self.widths(cols);
        let full = |spans: Vec<Span>, selected: bool| Row { spans, selected };

        // The header: the title and who asked, then the text.
        let from = word(Word::From(&self.from));
        let title = fit(&self.title, cols.saturating_sub(cells(&from) + 3));
        let gap = cols.saturating_sub(cells(&title) + cells(&from) + 2);
        let mut header = vec![full(
            vec![
                span(" ", Look::Dim),
                span(title, Look::Title),
                span(" ".repeat(gap), Look::Dim),
                span(from, Look::Dim),
            ],
            false,
        )];
        for line in self
            .text
            .lines()
            .flat_map(|l| wrap(l, cols.saturating_sub(2)))
        {
            header.push(full(
                vec![span(" ", Look::Dim), span(line, Look::Dim)],
                false,
            ));
        }
        header.push(full(vec![], false));
        // A long text must leave room for the fields and the buttons; a cut shows a mark.
        let most = lines.saturating_sub(4).max(1);
        if header.len() > most {
            header.truncate(most);
            if most > 1
                && let Some(last) = header.last_mut()
            {
                *last = full(vec![span(" …", Look::Dim)], false);
            }
        }

        // The fields.
        let mut body: Vec<Row> = Vec::new();
        // The rows of the focused field (with its help and problem), and its row that must be seen.
        let mut whole = (0, 0);
        let mut target = 0;
        let mut cursor_at = None;
        let focus = self.focus;
        let tried = self.tried;
        for i in 0..self.fields.len() {
            let focused = focus == Focus::Field(i);
            if self.fields[i].is_group() && !body.is_empty() {
                body.push(full(vec![], false));
            }
            let first = body.len();
            let (rows, (cr, cc), keep) = self.fields[i].rows(label_w, value_w, focused, word);
            body.extend(rows);
            let f = &self.fields[i];
            if focused {
                target = first + keep;
                cursor_at = Some((first + cr, cc));
                if let Some(help) = &f.help {
                    body.extend(help_rows(help, label_w + 3, value_w));
                }
            }
            // A problem shows under an edited field when the user left it, and under any field after a try to
            // save (a required field with no value is no error before that).
            if let Some(p) = f.problem()
                && (tried || (!focused && f.edited()))
            {
                body.push(full(
                    vec![
                        span(" ".repeat(label_w + 3), Look::Dim),
                        span(fit(&word(Word::Problem(p)), value_w), Look::Error),
                    ],
                    false,
                ));
            }
            if focused {
                whole = (first, body.len().saturating_sub(1));
            }
        }

        // The buttons and the keys.
        let button = |text: String, on: bool| {
            let look = if on { Look::Cursor } else { Look::Text };
            span(format!(" {text} "), look)
        };
        let buttons = full(
            vec![
                span(" ".repeat(label_w + 3), Look::Dim),
                button(word(Word::Save), focus == Focus::Save),
                span("  ", Look::Dim),
                button(word(Word::Cancel), focus == Focus::Cancel),
            ],
            false,
        );
        let hint = if self.esc_armed {
            Hint::EscAgain
        } else {
            match focus {
                Focus::Field(i) => match &self.fields[i].kind {
                    Kind::Text { .. } | Kind::Number { .. } => Hint::Text,
                    Kind::Multiline { .. } => Hint::Multiline,
                    Kind::Select { list: Some(_), .. } => Hint::List,
                    _ => Hint::Form,
                },
                _ => Hint::Form,
            }
        };
        let hint_look = if self.esc_armed {
            Look::Error
        } else {
            Look::Dim
        };
        let footer = vec![
            full(vec![], false),
            buttons,
            full(
                vec![
                    span(" ", Look::Dim),
                    span(fit(&word(Word::Hint(hint)), cols - 1), hint_look),
                ],
                false,
            ),
        ];

        // The fields scroll so the focused field is seen when it fits, and its cursor row always.
        let room = lines.saturating_sub(header.len() + footer.len()).max(1);
        if !matches!(focus, Focus::Field(_)) {
            target = body.len().saturating_sub(1);
            whole = (target, target);
        }
        if whole.1 >= self.top + room {
            self.top = whole.1 + 1 - room;
        }
        if whole.0 < self.top {
            self.top = whole.0;
        }
        if target < self.top {
            self.top = target;
        } else if target >= self.top + room {
            self.top = target + 1 - room;
        }
        self.top = self.top.min(body.len().saturating_sub(room));
        let seen: Vec<Row> = body.into_iter().skip(self.top).take(room).collect();
        let fill = room - seen.len();
        let cursor = cursor_at
            .filter(|(r, _)| *r >= self.top && *r < self.top + room)
            .map(|(r, c)| (header.len() + r - self.top, c));
        let mut rows = header;
        rows.extend(seen);
        rows.extend((0..fill).map(|_| full(vec![], false)));
        rows.extend(footer);
        rows.truncate(lines.max(1));
        // No row is wider than the pane (it would wrap and move the whole screen).
        for row in &mut rows {
            clip(row, pane);
        }
        let cursor = cursor.filter(|(r, c)| *r < rows.len() && *c < pane);
        Screen { rows, cursor }
    }
}

/// Cuts a row to `cols` cells.
fn clip(row: &mut Row, cols: usize) {
    let mut left = cols;
    let mut keep = Vec::new();
    for s in row.spans.drain(..) {
        if left == 0 {
            break;
        }
        let text = fit(&s.text, left);
        left -= cells(&text);
        keep.push(span(text, s.look));
    }
    row.spans = keep;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::Problem;

    const W: usize = 60;

    fn words(w: Word) -> String {
        match w {
            Word::From(who) => format!("from {who}"),
            Word::Save => "Save".into(),
            Word::Cancel => "Cancel".into(),
            Word::NotSet => "(not set)".into(),
            Word::Current => "(current)".into(),
            Word::Problem(p) => format!("problem: {p:?}"),
            Word::Hint(h) => format!("keys: {h:?}"),
        }
    }

    fn form(v: Value) -> Form {
        Form::from_json(&v).unwrap_or_else(|e| panic!("{e}"))
    }

    fn error(v: Value) -> String {
        Form::from_json(&v).expect_err("an error")
    }

    fn git() -> Form {
        form(json!({
            "title": "Git settings",
            "text": "~/.gitconfig",
            "fields": [
                { "type": "group", "label": "User" },
                { "id": "user.name", "type": "text", "label": "Name", "value": "Ann", "required": true },
                { "id": "user.email", "type": "text", "label": "Email", "pattern": "[^@ ]+@[^@ ]+" },
                { "id": "commit.gpgsign", "type": "checkbox", "label": "Sign commits", "value": true },
                { "id": "commit.template", "type": "multiline", "value": "a\tb\r\nc", "rows": 3 },
                { "id": "core.editor", "type": "radio", "label": "Editor",
                  "options": ["vim", { "value": "code --wait", "label": "VS Code" }], "value": "nano" },
                { "id": "init.defaultBranch", "type": "select", "options": ["main", "master", "trunk"],
                  "value": "main" },
                { "id": "font_size", "type": "number", "value": 13, "min": 6, "max": 72 },
                { "id": "scale", "type": "number", "value": 1.5 },
            ]
        }))
    }

    fn typing(f: &mut Form, text: &str) {
        for c in text.chars() {
            f.key(Key::Text(&c.to_string()), W);
        }
    }

    fn go(f: &mut Form, id: &str) {
        let i = f.fields().iter().position(|x| x.id == id).unwrap();
        f.focus = Focus::Field(i);
    }

    fn field<'a>(f: &'a Form, id: &str) -> &'a Field {
        f.fields().iter().find(|x| x.id == id).unwrap()
    }

    fn values(f: &Form) -> Value {
        f.answer()["values"].clone()
    }

    #[test]
    fn untouched_values_come_back_exactly() {
        let f = git();
        let a = f.answer();
        assert_eq!(a["status"], "saved");
        assert_eq!(a["changed"], json!([]));
        let v = &a["values"];
        assert_eq!(v["user.name"], "Ann");
        assert_eq!(v["user.email"], Value::Null, "a missing value is null");
        assert_eq!(v["commit.template"], "a\tb\r\nc", "tabs and CR LF are kept");
        assert_eq!(
            v["core.editor"], "nano",
            "a value that is not an option is kept"
        );
        assert_eq!(v["font_size"], json!(13), "an integer stays an integer");
        assert_eq!(v["scale"], json!(1.5));
        let ids: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(ids[0], "user.name", "in the order of the fields");
    }

    #[test]
    fn the_focus_starts_on_the_first_field_and_tab_goes_round() {
        let mut f = git();
        assert_eq!(f.focus(), Focus::Field(1), "not the group");
        for _ in 0..8 {
            f.key(Key::Tab, W);
        }
        assert_eq!(f.focus(), Focus::Save);
        f.key(Key::Tab, W);
        assert_eq!(f.focus(), Focus::Cancel);
        f.key(Key::Tab, W);
        assert_eq!(
            f.focus(),
            Focus::Field(1),
            "after Cancel comes the first field"
        );
        f.key(Key::BackTab, W);
        assert_eq!(f.focus(), Focus::Cancel);
    }

    #[test]
    fn typing_changes_a_text() {
        let mut f = git();
        f.key(Key::End, W);
        go(&mut f, "user.name");
        typing(&mut f, "a");
        assert_eq!(values(&f)["user.name"], "Anna");
        assert_eq!(f.answer()["changed"], json!(["user.name"]));
        f.key(Key::Backspace, W);
        assert_eq!(
            f.answer()["changed"],
            json!([]),
            "back to the old text: not changed"
        );
    }

    #[test]
    fn a_one_line_text_gets_one_line_from_a_paste() {
        let mut f = git();
        go(&mut f, "user.email");
        f.key(Key::Paste("a@b.c\r\n"), W);
        assert_eq!(
            values(&f)["user.email"],
            "a@b.c",
            "one line, with no spaces at the ends (F6)"
        );
        go(&mut f, "commit.template");
        f.key(Key::Paste("\nline 2\u{1b}[31m"), W);
        assert_eq!(
            values(&f)["commit.template"],
            "a\tb\r\nc\r\nline 2[31m",
            "the tab and the CR LF that were there stay; the new line is CR LF too; no control chars come in"
        );
    }

    #[test]
    fn touching_a_text_keeps_the_parts_that_the_user_did_not_edit() {
        // F1: the edit copy was cleaned, and the clean text became the value.
        let mut f = form(json!({ "fields": [
            { "id": "m", "type": "multiline", "value": "a\tb\r\nc\r\n" },
            { "id": "t", "type": "text", "value": "x\ty" },
        ]}));
        f.key(Key::Delete, W);
        assert_eq!(
            f.answer()["changed"],
            json!([]),
            "a Delete that does nothing changes nothing"
        );
        f.key(Key::Text("z"), W);
        assert_eq!(values(&f)["m"], "a\tb\r\nc\r\nz");
        f.key(
            Key::Enter {
                ctrl: false,
                shift: false,
            },
            W,
        );
        assert_eq!(
            values(&f)["m"],
            "a\tb\r\nc\r\nz\r\n",
            "a new line in a CR LF text is CR LF"
        );
        f.key(Key::Backspace, W);
        assert_eq!(
            values(&f)["m"],
            "a\tb\r\nc\r\nz",
            "Backspace takes the whole CR LF"
        );
        f.key(Key::Tab, W);
        f.key(Key::End, W);
        f.key(Key::Text("!"), W);
        assert_eq!(values(&f)["t"], "x\ty!", "the tab stays");
    }

    #[test]
    fn a_long_value_is_fast_to_draw() {
        // F2: the view of a one-line input was O(n^2).
        let long = "x".repeat(64 * 1024);
        let mut f = form(json!({ "fields": [
            { "id": "a", "type": "text", "value": long.clone() },
            { "id": "b", "type": "password", "value": long },
        ]}));
        let started = std::time::Instant::now();
        for _ in 0..5 {
            f.screen(W, 20, &words);
            f.key(Key::Tab, W);
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn an_open_list_at_the_bottom_is_seen() {
        // F3: the list opened under the last seen row, so its options were off the screen.
        let mut list: Vec<Value> = (0..20)
            .map(|i| json!({ "id": format!("t{i}"), "type": "text" }))
            .collect();
        list.push(
            json!({ "id": "s", "type": "select", "options": ["one", "two", "three", "four"] }),
        );
        let mut f = form(json!({ "fields": list }));
        go(&mut f, "s");
        f.screen(W, 15, &words);
        f.key(
            Key::Enter {
                ctrl: false,
                shift: false,
            },
            W,
        );
        f.key(Key::Down, W);
        f.key(Key::Down, W);
        let screen = f.screen(W, 15, &words);
        let seen: Vec<String> = screen.rows.iter().map(|r| r.plain()).collect();
        assert!(
            screen
                .rows
                .iter()
                .any(|r| r.selected && r.plain().contains("three")),
            "the highlighted option is seen: {seen:#?}"
        );
    }

    #[test]
    fn no_row_is_wider_than_the_pane() {
        // F4: a radio with "(not set)", a long option, or a tab in a label made rows too wide.
        let mut f = form(json!({ "title": "t\tt", "fields": [
            { "id": "r", "type": "radio", "label": "a\tb", "options": ["alpha", "bravo", "charlie", "delta", "echo"] },
            { "id": "q", "type": "radio", "options": ["x".repeat(80)] },
            { "id": "s", "type": "select", "options": ["y".repeat(80)], "value": "y".repeat(80) },
        ]}));
        for cols in [20, 33, 47, 60] {
            let screen = f.screen(cols, 30, &words);
            for row in &screen.rows {
                assert!(cells(&row.plain()) <= cols, "{cols}: {:?}", row.plain());
                assert!(!row.plain().contains('\t'), "{:?}", row.plain());
            }
        }
    }

    #[test]
    fn a_field_that_is_back_to_its_value_is_not_checked() {
        // F5: touched but not changed: an old bad value blocked saving.
        let mut f = form(json!({ "fields": [
            { "id": "n", "type": "number", "value": 100, "max": 10 },
        ]}));
        f.key(Key::Delete, W);
        f.key(Key::Text("0"), W);
        f.key(Key::Backspace, W);
        assert_eq!(f.key(Key::Save, W), Outcome::Save);
    }

    #[test]
    fn a_long_text_does_not_push_out_the_buttons() {
        // F7: a long `text` pushed the buttons off, and the cursor was out of the rows.
        let mut f = form(json!({ "text": "word ".repeat(400), "fields": [
            { "id": "a", "type": "text", "value": "v" },
        ]}));
        let screen = f.screen(40, 12, &words);
        assert_eq!(screen.rows.len(), 12);
        assert!(
            screen.rows.iter().any(|r| r.plain().contains("Save")),
            "the buttons are seen"
        );
        if let Some((row, _)) = screen.cursor {
            assert!(row < screen.rows.len());
            assert!(
                screen.rows[row].plain().contains('v'),
                "the cursor is on its field"
            );
        }
    }

    #[test]
    fn the_esc_hint_goes_with_the_next_key() {
        // F8: after Esc, another key cleared it but did not redraw.
        let mut f = git();
        go(&mut f, "commit.gpgsign");
        f.key(Key::Text(" "), W);
        assert_eq!(f.key(Key::Escape, W), Outcome::Changed);
        assert_eq!(
            f.key(Key::Left, W),
            Outcome::Changed,
            "a key that does nothing to a checkbox: the hint must still go away"
        );
    }

    #[test]
    fn a_multi_line_input_takes_a_placeholder() {
        // F9.
        let mut f = form(json!({ "fields": [
            { "id": "a", "type": "text" },
            { "id": "m", "type": "multiline", "value": "", "placeholder": "notes here" },
        ]}));
        let screen = f.screen(W, 20, &words);
        assert!(screen.rows.iter().any(|r| r.plain().contains("notes here")));
    }

    #[test]
    fn an_empty_text_stays_empty_when_it_comes_back() {
        // F10: a clearable "" became null (remove the setting) after a touch.
        let mut f = form(json!({ "fields": [
            { "id": "hooks", "type": "text", "value": "", "clearable": true },
        ]}));
        f.key(Key::Text("x"), W);
        f.key(Key::Backspace, W);
        assert_eq!(values(&f)["hooks"], "");
        assert_eq!(f.answer()["changed"], json!([]));
    }

    #[test]
    fn a_pattern_can_not_get_out_of_its_wrapper() {
        // F11: "a)|(b" made "^(?:a)|(b)$": any value that starts with a passed.
        assert!(
            error(json!({ "fields": [{ "id": "p", "type": "text", "pattern": "a)|(b" }] }))
                .contains("fields[0].pattern")
        );
    }

    #[test]
    fn a_password_is_not_in_the_debug_text() {
        let f =
            form(json!({ "fields": [{ "id": "token", "type": "password", "value": "s3cret" }] }));
        let all = format!("{f:?}");
        assert!(!all.contains("s3cret"), "{all}");
        let kind = format!("{:?}", f.fields()[0].kind);
        assert!(!kind.contains("s3cret"), "{kind}");
    }

    #[test]
    fn a_pattern_must_match_the_whole_value() {
        let mut f = form(json!({ "fields": [
            { "id": "n", "type": "text", "pattern": "[0-9]+" },
        ]}));
        typing(&mut f, "12abc");
        assert_eq!(f.fields()[0].problem(), Some(Problem::NoMatch));
        for _ in 0..3 {
            f.key(Key::Backspace, W);
        }
        assert_eq!(f.fields()[0].problem(), None);
        assert_eq!(f.key(Key::Save, W), Outcome::Save);
    }

    #[test]
    fn an_old_value_that_breaks_a_rule_does_not_block_saving() {
        let mut f = form(json!({ "fields": [
            { "id": "email", "type": "text", "value": "not an email", "pattern": "[^@ ]+@[^@ ]+", "max": 5 },
            { "id": "n", "type": "number", "value": 100, "max": 10 },
            { "id": "ok", "type": "checkbox", "value": false },
        ]}));
        f.key(Key::Tab, W);
        f.key(Key::Tab, W);
        f.key(Key::Text(" "), W);
        assert_eq!(f.key(Key::Save, W), Outcome::Save);
        assert_eq!(f.answer()["changed"], json!(["ok"]));
        assert_eq!(values(&f)["n"], json!(100));
    }

    #[test]
    fn a_required_field_with_no_value_blocks_saving() {
        // `required` means: the answer has a value, also when the user did not touch the field.
        let options = json!({ "options": ["a", "b"] });
        for (kind, extra) in [
            ("text", json!({})),
            ("multiline", json!({})),
            ("number", json!({})),
            ("radio", options.clone()),
            ("select", options),
        ] {
            let mut spec = json!({ "id": "need", "type": kind, "required": true });
            spec.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let mut f = form(json!({ "fields": [
                { "id": "x", "type": "checkbox", "value": true },
                spec,
            ]}));
            assert_eq!(f.key(Key::Save, W), Outcome::Changed, "{kind}: no save");
            assert_eq!(f.focus(), Focus::Field(1), "{kind}: the focus goes to it");
            assert_eq!(
                field(&f, "need").problem(),
                Some(Problem::Required),
                "{kind}"
            );
        }
        let mut f = form(
            json!({ "fields": [{ "id": "t", "type": "text", "value": "", "required": true }] }),
        );
        assert_eq!(
            f.key(Key::Save, W),
            Outcome::Changed,
            "an empty text is no value too"
        );
    }

    #[test]
    fn a_required_problem_shows_after_a_try_to_save() {
        let mut f = form(json!({ "fields": [
            { "id": "x", "type": "checkbox", "value": true },
            { "id": "need", "type": "text", "required": true },
        ]}));
        let seen = |f: &mut Form| {
            f.screen(W, 20, &words)
                .rows
                .iter()
                .any(|r| r.plain().contains("problem: Required"))
        };
        assert!(!seen(&mut f), "no error before the user did anything");
        f.key(Key::Save, W);
        assert!(seen(&mut f), "the error shows after a try to save");
    }

    #[test]
    fn a_bad_changed_field_blocks_saving_and_gets_the_focus() {
        let mut f = git();
        go(&mut f, "user.name");
        for _ in 0..3 {
            f.key(Key::Backspace, W);
        }
        f.key(Key::Tab, W);
        assert_eq!(f.key(Key::Save, W), Outcome::Changed);
        assert_eq!(
            f.focus(),
            Focus::Field(1),
            "the focus goes to the bad field"
        );
        let screen = f.screen(W, 40, &words);
        assert!(
            screen
                .rows
                .iter()
                .any(|r| r.plain().contains("problem: Required")),
            "its problem shows"
        );
    }

    #[test]
    fn numbers_are_numbers() {
        let mut f = git();
        go(&mut f, "font_size");
        f.key(Key::End, W);
        typing(&mut f, "5");
        assert_eq!(values(&f)["font_size"], json!(135));
        assert_eq!(field(&f, "font_size").problem(), Some(Problem::Max(72.0)));
        f.key(Key::Backspace, W);
        f.key(Key::Backspace, W);
        typing(&mut f, "4");
        assert_eq!(values(&f)["font_size"], json!(14));
        typing(&mut f, ".5");
        assert_eq!(values(&f)["font_size"], json!(14.5));
        typing(&mut f, "x");
        assert_eq!(field(&f, "font_size").problem(), Some(Problem::NotNumber));
        let mut g = form(json!({ "fields": [{ "id": "n", "type": "number", "integer": true }] }));
        typing(&mut g, "1.5");
        assert_eq!(g.fields()[0].problem(), Some(Problem::NotInteger));
    }

    #[test]
    fn a_value_that_is_not_an_option_is_kept_as_current() {
        let mut f = git();
        go(&mut f, "core.editor");
        let screen = f.screen(W, 40, &words);
        assert!(
            screen
                .rows
                .iter()
                .any(|r| r.plain().contains("nano (current)"))
        );
        f.key(Key::Left, W);
        assert_eq!(values(&f)["core.editor"], "code --wait");
        assert_eq!(f.answer()["changed"], json!(["core.editor"]));
        f.key(Key::Right, W);
        assert_eq!(
            values(&f)["core.editor"],
            "nano",
            "back to the current value"
        );
        assert_eq!(f.answer()["changed"], json!([]));
    }

    #[test]
    fn a_select_opens_filters_and_picks() {
        let mut f = git();
        go(&mut f, "init.defaultBranch");
        f.key(
            Key::Enter {
                ctrl: false,
                shift: false,
            },
            W,
        );
        assert!(field(&f, "init.defaultBranch").list_open());
        typing(&mut f, "TR");
        f.key(
            Key::Enter {
                ctrl: false,
                shift: false,
            },
            W,
        );
        assert!(!field(&f, "init.defaultBranch").list_open());
        assert_eq!(
            values(&f)["init.defaultBranch"],
            "trunk",
            "the filter has no case"
        );
        f.key(Key::Text("m"), W);
        f.key(Key::Down, W);
        f.key(Key::Escape, W);
        assert!(
            !field(&f, "init.defaultBranch").list_open(),
            "Esc closes the list"
        );
        assert_eq!(
            values(&f)["init.defaultBranch"],
            "trunk",
            "and picks nothing"
        );
    }

    #[test]
    fn a_checkbox_and_a_cleared_value() {
        let mut f = form(json!({ "fields": [
            { "id": "a", "type": "checkbox" },
            { "id": "b", "type": "checkbox", "value": true, "clearable": true },
            { "id": "c", "type": "text", "value": "x", "clearable": true },
        ]}));
        f.key(Key::Text(" "), W);
        assert_eq!(values(&f)["a"], json!(true), "not set, then on");
        f.key(Key::Tab, W);
        f.key(Key::Delete, W);
        assert_eq!(values(&f)["b"], Value::Null, "clearable: Delete = not set");
        f.key(Key::Tab, W);
        f.key(Key::Backspace, W);
        assert_eq!(
            values(&f)["c"],
            Value::Null,
            "an empty clearable text is not set"
        );
    }

    #[test]
    fn esc_twice_when_something_changed() {
        let mut f = git();
        assert_eq!(
            f.key(Key::Escape, W),
            Outcome::Cancel,
            "nothing changed: one Esc"
        );
        go(&mut f, "commit.gpgsign");
        f.key(Key::Text(" "), W);
        assert_eq!(f.key(Key::Escape, W), Outcome::Changed);
        let screen = f.screen(W, 40, &words);
        assert!(screen.rows.last().unwrap().plain().contains("EscAgain"));
        assert_eq!(f.key(Key::Escape, W), Outcome::Cancel);
        assert_eq!(
            Form::cancelled(),
            json!({ "status": "cancelled", "changed": [] }),
            "no values"
        );
    }

    #[test]
    fn the_buttons_work_with_no_ctrl() {
        let mut f = git();
        f.key(Key::BackTab, W);
        f.key(Key::BackTab, W);
        assert_eq!(f.focus(), Focus::Save);
        assert_eq!(
            f.key(
                Key::Enter {
                    ctrl: false,
                    shift: false
                },
                W
            ),
            Outcome::Save
        );
        f.key(Key::Right, W);
        assert_eq!(f.focus(), Focus::Cancel);
        assert_eq!(
            f.key(
                Key::Enter {
                    ctrl: false,
                    shift: false
                },
                W
            ),
            Outcome::Cancel
        );
    }

    #[test]
    fn a_multi_line_input_keeps_up_and_down_while_it_can() {
        let mut f = git();
        go(&mut f, "commit.template");
        f.key(
            Key::Enter {
                ctrl: false,
                shift: false,
            },
            W,
        );
        assert_eq!(
            values(&f)["commit.template"],
            "a\tb\r\nc\r\n",
            "Enter is a new line here (CR LF, as the text has)"
        );
        f.key(Key::Up, W);
        assert_eq!(f.focus(), Focus::Field(4), "Up moves in the text");
        f.key(Key::Up, W);
        f.key(Key::Up, W);
        assert_eq!(
            f.focus(),
            Focus::Field(3),
            "from the first line: the field above"
        );
    }

    #[test]
    fn a_password_is_drawn_as_dots() {
        let mut f = form(json!({ "fields": [
            { "id": "token", "type": "password", "value": "s3cret" },
        ]}));
        let screen = f.screen(W, 20, &words);
        let all: String = screen.rows.iter().map(|r| r.plain()).collect();
        assert!(!all.contains("s3cret"), "{all}");
        assert!(all.contains("••••••"));
        assert_eq!(values(&f)["token"], "s3cret");
    }

    #[test]
    fn the_screen_fits_and_follows_the_cursor() {
        let mut f = form(json!({ "title": "Many", "fields": (0..40)
            .map(|i| json!({ "id": format!("f{i}"), "type": "text", "value": format!("v{i}") }))
            .collect::<Vec<_>>() }));
        for _ in 0..30 {
            f.key(Key::Tab, W);
        }
        let screen = f.screen(W, 15, &words);
        assert_eq!(screen.rows.len(), 15);
        let (row, _) = screen.cursor.expect("the cursor is seen");
        assert!(
            screen.rows[row].plain().contains("v30"),
            "{:?}",
            screen.rows[row].plain()
        );
        assert!(
            screen.rows[row].selected,
            "the focused row has the selection background"
        );
        assert!(screen.rows[0].plain().contains("Many"));
        assert!(
            screen.rows[13].plain().contains("Save"),
            "the buttons stay at the bottom"
        );
    }

    #[test]
    fn a_long_multi_line_value_scrolls_with_its_cursor() {
        let text: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
        let mut f = form(json!({ "fields": [
            { "id": "t", "type": "multiline", "value": text.join("\n"), "rows": 4 },
        ]}));
        let screen = f.screen(W, 20, &words);
        let (row, _) = screen.cursor.expect("seen");
        assert!(
            screen.rows[row].plain().contains("line 30"),
            "the cursor at the end is seen"
        );
    }

    #[test]
    fn bad_requests_say_where() {
        let cases = [
            (json!([]), "expected an object"),
            (json!({}), "fields: missing"),
            (
                json!({ "fields": [{ "type": "group", "label": "x" }] }),
                "no field with a value",
            ),
            (
                json!({ "fields": [{ "type": "slider", "id": "a" }] }),
                "fields[0].type",
            ),
            (
                json!({ "fields": [{ "type": "text" }] }),
                "fields[0].id: missing",
            ),
            (
                json!({ "fields": [{ "type": "text", "id": "a" }, { "type": "text", "id": "a" }] }),
                "fields[1].id",
            ),
            (
                json!({ "fields": [{ "type": "text", "id": "a", "requried": true }] }),
                "`requried`",
            ),
            (
                json!({ "fields": [{ "type": "text", "id": "a", "value": 13 }] }),
                "use a field of type number",
            ),
            (
                json!({ "fields": [{ "type": "checkbox", "id": "a", "value": "yes" }] }),
                "fields[0].value",
            ),
            (
                json!({ "fields": [{ "type": "radio", "id": "a", "options": (0..9).collect::<Vec<_>>() }] }),
                "use a select",
            ),
            (
                json!({ "fields": [{ "type": "select", "id": "a", "options": [] }] }),
                "fields[0].options: empty",
            ),
            (
                json!({ "fields": [{ "type": "select", "id": "a", "options": ["x", "x"] }] }),
                "fields[0].options[1]",
            ),
            (
                json!({ "fields": [{ "type": "select", "id": "a", "options": [{ "value": [1] }] }] }),
                "fields[0].options[0].value",
            ),
            (
                json!({ "fields": [{ "type": "text", "id": "a", "pattern": "(" }] }),
                "fields[0].pattern",
            ),
            (
                json!({ "fields": [{ "type": "text", "id": "a", "pattern": "x".repeat(2000) }] }),
                "longer than 1024",
            ),
            (
                json!({ "fields": [{ "type": "text", "id": "a", "pattern": "(?=x)" }] }),
                "fields[0].pattern",
            ),
            (
                json!({ "fields": [{ "type": "multiline", "id": "a", "rows": 50 }] }),
                "fields[0].rows",
            ),
        ];
        for (request, want) in cases {
            let e = error(request.clone());
            assert!(e.contains(want), "{request}: {e}");
        }
    }

    #[test]
    fn ids_are_any_text() {
        let f = form(json!({ "fields": [
            { "id": "url.\"https://x\".insteadOf", "type": "text", "value": "y" },
        ]}));
        assert_eq!(values(&f)["url.\"https://x\".insteadOf"], "y");
    }

    #[test]
    fn a_number_with_no_value_checks_what_the_user_typed() {
        // Second review, finding 1: "12px" gave null, the same as the request, so it was not checked and saved.
        let mut f = form(json!({ "fields": [
            { "id": "n", "type": "number" },
            { "id": "r", "type": "number", "required": true },
        ]}));
        typing(&mut f, "12px");
        assert_eq!(field(&f, "n").problem(), Some(Problem::NotNumber));
        f.key(Key::Tab, W);
        assert_eq!(
            f.key(Key::Save, W),
            Outcome::Changed,
            "no save with a text that is not a number"
        );
        assert_eq!(f.focus(), Focus::Field(0));
        assert_eq!(
            f.key(Key::Escape, W),
            Outcome::Changed,
            "Esc asks again: the user typed something"
        );
    }

    #[test]
    fn down_goes_past_a_crlf_line_that_fills_its_row() {
        // Second review, finding 2: the CR took the place of the last char, and Down came back to it.
        let cols = 20; // the value is 12 cells wide
        let mut f = form(json!({ "fields": [
            { "id": "m", "type": "multiline", "value": "abcdefghijkl\r\nmn\r\nop", "rows": 5 },
        ]}));
        for _ in 0..6 {
            f.key(Key::Up, cols);
        }
        f.key(Key::Home, cols);
        for _ in 0..3 {
            f.key(Key::Down, cols);
        }
        f.key(Key::Text("X"), cols);
        assert_eq!(values(&f)["m"], "abcdefghijkl\r\nmn\r\nXop");
        // One Down from the first line: the end of that line, before its CR LF.
        for _ in 0..6 {
            f.key(Key::Up, cols);
        }
        f.key(Key::Home, cols);
        f.key(Key::Down, cols);
        f.key(Key::Text("Y"), cols);
        assert_eq!(values(&f)["m"], "abcdefghijklY\r\nmn\r\nXop");
    }

    #[test]
    fn an_open_list_at_the_bottom_shows_its_options() {
        // Second review, finding 3: only the highlighted option was seen.
        let mut list: Vec<Value> = (0..20)
            .map(|i| json!({ "id": format!("t{i}"), "type": "text" }))
            .collect();
        list.push(
            json!({ "id": "s", "type": "select", "options": ["one", "two", "three", "four"] }),
        );
        let mut f = form(json!({ "fields": list }));
        go(&mut f, "s");
        f.screen(W, 15, &words);
        f.key(
            Key::Enter {
                ctrl: false,
                shift: false,
            },
            W,
        );
        let screen = f.screen(W, 15, &words);
        let seen: Vec<String> = screen.rows.iter().map(|r| r.plain()).collect();
        for option in ["one", "two", "three", "four"] {
            assert!(
                seen.iter().any(|r| r.contains(option)),
                "{option}: {seen:#?}"
            );
        }
    }

    #[test]
    fn a_placeholder_shows_for_a_field_with_no_value() {
        // Second review, finding 4: a field with no value showed "(not set)" or an empty input.
        let mut f = form(json!({ "fields": [
            { "id": "a", "type": "text", "placeholder": "your name" },
            { "id": "m", "type": "multiline", "placeholder": "notes here" },
        ]}));
        let all: Vec<String> = f
            .screen(W, 20, &words)
            .rows
            .iter()
            .map(|r| r.plain())
            .collect();
        assert!(
            all.iter().any(|r| r.contains("your name")),
            "focused: {all:#?}"
        );
        assert!(
            all.iter().any(|r| r.contains("notes here")),
            "not focused: {all:#?}"
        );
    }

    #[test]
    fn a_narrow_pane_gets_no_wide_rows() {
        // Second review, finding 5: rows were cut at 20 cells, not at the pane.
        let mut f = git();
        for cols in [8, 12, 19] {
            let screen = f.screen(cols, 30, &words);
            for row in &screen.rows {
                assert!(cells(&row.plain()) <= cols, "{cols}: {:?}", row.plain());
            }
            if let Some((_, c)) = screen.cursor {
                assert!(c < cols, "{cols}: the cursor at {c}");
            }
        }
    }

    #[test]
    fn keys_and_drawing_use_the_same_widths() {
        // Second review, finding 5: in a narrow pane, Up moved by rows that were not drawn.
        let mut f = form(json!({ "fields": [
            { "id": "a", "type": "text" },
            { "id": "m", "type": "multiline", "value": "abcdefghij" },
        ]}));
        go(&mut f, "m");
        f.screen(12, 20, &words);
        f.key(Key::Up, 12);
        assert_eq!(
            f.focus(),
            Focus::Field(0),
            "the value is drawn on one row, so Up leaves it"
        );
    }

    #[test]
    fn a_cut_text_shows_a_mark() {
        // Second review, finding 6: a long text was cut with no sign.
        let mut f = form(json!({ "text": "word ".repeat(400), "fields": [
            { "id": "a", "type": "text", "value": "v" },
        ]}));
        let screen = f.screen(40, 12, &words);
        let all: Vec<String> = screen.rows.iter().map(|r| r.plain()).collect();
        assert!(all.iter().any(|r| r.contains('…')), "{all:#?}");
    }
}
