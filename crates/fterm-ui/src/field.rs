//! One field of a form: its value as it came (kept exactly), its edit state, its keys, its rows, and its check.

use regex_automata::meta::Regex;
use serde_json::Value;

use crate::input::InputBox;
use crate::key::Key;
use crate::look::{Look, Row, Span, span};
use crate::text::{cells, fit, one_line, wrap};

/// The most options of a radio group (more: use a select).
pub const MAX_RADIO: usize = 8;
/// The most options of a select.
pub const MAX_OPTIONS: usize = 200;
/// The most chars of a text value.
pub const MAX_TEXT: usize = 64 * 1024;
/// The most chars of an id.
pub const MAX_ID: usize = 200;
/// The most bytes of a `pattern`.
pub const MAX_PATTERN: usize = 1024;
/// The rows of a select list that are seen at once.
const LIST_ROWS: usize = 8;

/// Why a value is not valid. The app turns it into words of the UI language.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Problem {
    Required,
    NoMatch,
    TooLong(usize),
    NotNumber,
    NotInteger,
    Min(f64),
    Max(f64),
}

/// A word or a line of the UI. The app gives the text (`tr!`), so a form follows the UI language.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Word<'a> {
    /// "from claude" in the header.
    From(&'a str),
    Save,
    Cancel,
    /// A value that is not set ("(not set)").
    NotSet,
    /// After a value that is not one of the options: "(current)".
    Current,
    Problem(Problem),
    /// The keys at the bottom, for what has the focus now.
    Hint(Hint),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hint {
    /// A field with no typing (a checkbox, a radio group, a closed select), or a button.
    Form,
    /// A text, a password, or a number.
    Text,
    Multiline,
    /// The open list of a select.
    List,
    /// Esc once more cancels: the form has changes.
    EscAgain,
}

/// One option of a radio group or a select.
#[derive(Clone, Debug, PartialEq)]
pub struct Opt {
    pub value: Value,
    pub label: String,
    /// Not an option of the request: the value that the field had ("(current)").
    pub current: bool,
}

/// The rules of a text.
#[derive(Clone, Debug, Default)]
pub struct TextRules {
    pub required: bool,
    pub pattern: Option<Regex>,
    pub max: Option<usize>,
}

/// The open list of a select: the typed filter and the highlighted row (in the filtered list).
#[derive(Clone, Debug, Default)]
pub struct List {
    pub filter: InputBox,
    pub at: usize,
}

#[derive(Clone)]
pub enum Kind {
    /// A heading, no value.
    Group,
    Checkbox {
        on: Option<bool>,
    },
    /// One line; `password` = drawn as dots.
    Text {
        input: InputBox,
        password: bool,
        placeholder: String,
        rules: TextRules,
    },
    Number {
        input: InputBox,
        min: Option<f64>,
        max: Option<f64>,
        integer: bool,
        required: bool,
    },
    Multiline {
        input: InputBox,
        rows: usize,
        /// The first line of the value that is seen.
        top: usize,
        rules: TextRules,
        placeholder: String,
        /// The value had CR LF: new lines that the user adds are CR LF too.
        crlf: bool,
    },
    Radio {
        options: Vec<Opt>,
        chosen: Option<usize>,
        required: bool,
    },
    Select {
        options: Vec<Opt>,
        chosen: Option<usize>,
        required: bool,
        list: Option<List>,
    },
}

#[derive(Clone)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub help: Option<String>,
    /// The user may set it to "not set" (`null`).
    pub clearable: bool,
    /// The value of the request, as it came. An untouched field gives it back exactly.
    pub original: Value,
    /// The user changed something in it.
    pub touched: bool,
    pub kind: Kind,
    /// The edit text at the start (a text, a number): when the text is this again, the value is the original.
    start: String,
}

impl std::fmt::Debug for Field {
    /// No values: a password must not get into a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Field")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("touched", &self.touched)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Kind {
    /// Only the kind, no values: a password must not get into a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Kind::Group => "Group",
            Kind::Checkbox { .. } => "Checkbox",
            Kind::Text { password: true, .. } => "Password",
            Kind::Text { .. } => "Text",
            Kind::Number { .. } => "Number",
            Kind::Multiline { .. } => "Multiline",
            Kind::Radio { .. } => "Radio",
            Kind::Select { .. } => "Select",
        })
    }
}

/// Did a field use a key, or is it for the form (to go to another field)?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    Used,
    Not,
}

/// Text for a one-line input: new lines and tabs are spaces, no control chars (the ends are kept).
fn line_text(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// The char that is drawn for a char of a value: one char for one char, so the cursor index stays the same.
/// A tab is a space, a CR alone has no width, another control char is a mark; in one line, a new line is a
/// space. The value keeps the real chars.
fn shown_char(c: char, one_line: bool) -> char {
    match c {
        '\n' if one_line => ' ',
        '\n' => '\n',
        '\t' => ' ',
        '\r' if one_line => ' ',
        '\r' => '\u{200b}',
        c if c.is_control() => '\u{fffd}',
        c => c,
    }
}

fn shown(text: &str, one_line: bool) -> String {
    text.chars().map(|c| shown_char(c, one_line)).collect()
}

/// A multi-line value as it is drawn: the CR of a CR LF is left out (the new line is one char), the other chars
/// are drawn one for one (`shown_char`). Also gives, for each place in the drawn text, the place in the value;
/// the end of a line is the place before its CR, so the cursor never gets between the CR and the LF.
fn drawn(text: &str) -> (String, Vec<usize>) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut places = Vec::with_capacity(chars.len() + 1);
    for (j, &c) in chars.iter().enumerate() {
        if c == '\r' && chars.get(j + 1) == Some(&'\n') {
            continue;
        }
        let after_cr = c == '\n' && j > 0 && chars[j - 1] == '\r';
        places.push(if after_cr { j - 1 } else { j });
        out.push(shown_char(c, false));
    }
    places.push(chars.len());
    (out, places)
}

/// The place in the drawn text (`drawn`) of a place in the value.
fn drawn_place(places: &[usize], at: usize) -> usize {
    places.partition_point(|&p| p < at)
}

/// Typed or pasted text for an input with many lines: one kind of new line (LF), tabs kept, no other control chars.
fn paste_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| matches!(c, '\n' | '\t') || !c.is_control())
        .collect()
}

/// A label, a title, or a placeholder from the request, for drawing: one line, no control chars.
pub(crate) fn tidy(text: &str) -> String {
    line_text(text)
}

/// A help text or the text of a form: lines kept, no other control chars, tabs are spaces.
pub(crate) fn tidy_lines(text: &str) -> String {
    text.split('\n')
        .map(line_text)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The char at a char index.
fn char_at(text: &str, i: usize) -> Option<char> {
    text.chars().nth(i)
}

/// The cursor index is between the CR and the LF of a CR LF.
fn inside_crlf(text: &str, i: usize) -> bool {
    i > 0 && char_at(text, i - 1) == Some('\r') && char_at(text, i) == Some('\n')
}

/// The words of an option for people: a text as it is, a number or a bool as JSON.
fn option_label(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

impl Field {
    /// Reads one field of the request. `path` names it in errors (`fields[3]`).
    pub fn from_json(v: &Value, path: &str) -> Result<Field, String> {
        let Value::Object(map) = v else {
            return Err(format!("{path}: expected an object"));
        };
        let kind = match map.get("type") {
            Some(Value::String(t)) => t.as_str(),
            Some(_) => return Err(format!("{path}.type: expected a text")),
            None => return Err(format!("{path}.type: missing")),
        };
        let allowed: &[&str] = match kind {
            "group" => &["type", "label"],
            "checkbox" => &["type", "id", "label", "help", "value", "clearable"],
            "text" | "password" => &[
                "type",
                "id",
                "label",
                "help",
                "value",
                "clearable",
                "required",
                "pattern",
                "placeholder",
                "max",
            ],
            "number" => &[
                "type",
                "id",
                "label",
                "help",
                "value",
                "clearable",
                "required",
                "min",
                "max",
                "integer",
            ],
            "multiline" => &[
                "type",
                "id",
                "label",
                "help",
                "value",
                "clearable",
                "required",
                "pattern",
                "max",
                "rows",
                "placeholder",
            ],
            "radio" | "select" => &[
                "type",
                "id",
                "label",
                "help",
                "value",
                "clearable",
                "required",
                "options",
            ],
            other => {
                return Err(format!(
                    "{path}.type: `{other}` is not a field type; the types are group, checkbox, text, \
                     password, number, multiline, radio, select"
                ));
            }
        };
        if let Some(key) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(format!(
                "{path}: `{key}` is not a key of a {kind} field; its keys are {}",
                allowed.join(", ")
            ));
        }
        let text = |key: &str| -> Result<Option<String>, String> {
            match map.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(format!("{path}.{key}: expected a text")),
            }
        };
        let flag = |key: &str| -> Result<bool, String> {
            match map.get(key) {
                None | Some(Value::Null) => Ok(false),
                Some(Value::Bool(b)) => Ok(*b),
                Some(_) => Err(format!("{path}.{key}: expected true or false")),
            }
        };
        let number = |key: &str| -> Result<Option<f64>, String> {
            match map.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Number(n)) => n
                    .as_f64()
                    .map(Some)
                    .ok_or_else(|| format!("{path}.{key}: a number too big")),
                Some(_) => Err(format!("{path}.{key}: expected a number")),
            }
        };
        let label = text("label")?;
        if kind == "group" {
            return Ok(Field {
                id: String::new(),
                label: tidy(&label.unwrap_or_default()),
                help: None,
                clearable: false,
                original: Value::Null,
                touched: false,
                kind: Kind::Group,
                start: String::new(),
            });
        }
        let id = match map.get("id") {
            Some(Value::String(id)) if id.is_empty() => {
                return Err(format!("{path}.id: empty"));
            }
            Some(Value::String(id)) if id.chars().count() > MAX_ID => {
                return Err(format!("{path}.id: longer than {MAX_ID} chars"));
            }
            Some(Value::String(id)) => id.clone(),
            Some(_) => return Err(format!("{path}.id: expected a text")),
            None => return Err(format!("{path}.id: missing")),
        };
        let original = map.get("value").cloned().unwrap_or(Value::Null);
        let value_path = format!("{path}.value");
        let text_value = |original: &Value| -> Result<String, String> {
            match original {
                Value::Null => Ok(String::new()),
                Value::String(s) if s.chars().count() > MAX_TEXT => {
                    Err(format!("{value_path}: longer than {MAX_TEXT} chars"))
                }
                Value::String(s) => Ok(s.clone()),
                Value::Number(_) => Err(format!(
                    "{value_path}: a number in a {kind} field; use a field of type number"
                )),
                other => Err(format!("{value_path}: expected a text, got {other}")),
            }
        };
        let rules = || -> Result<TextRules, String> {
            let pattern = match text("pattern")? {
                None => None,
                Some(p) if p.len() > MAX_PATTERN => {
                    return Err(format!("{path}.pattern: longer than {MAX_PATTERN} bytes"));
                }
                Some(p) => {
                    let build = |re: &str| {
                        Regex::builder()
                            .configure(Regex::config().nfa_size_limit(Some(1 << 20)))
                            .build(re)
                            .map_err(|err| format!("{path}.pattern: {err}"))
                    };
                    // On its own first: "a)|(b" would get out of the wrapper and match too much.
                    build(p.as_str())?;
                    // The whole value must match (as `pattern` of HTML), whatever the agent wrote.
                    Some(build(&format!("^(?:{p})$"))?)
                }
            };
            let max = match map.get("max") {
                None | Some(Value::Null) => None,
                Some(Value::Number(n)) => match n.as_u64() {
                    Some(n) if n > 0 => Some(n as usize),
                    _ => return Err(format!("{path}.max: expected a count of chars (1 or more)")),
                },
                Some(_) => return Err(format!("{path}.max: expected a count of chars")),
            };
            Ok(TextRules {
                required: flag("required")?,
                pattern,
                max,
            })
        };
        let input_with = |text: &str| {
            let mut input = InputBox::default();
            input.set(text);
            input
        };
        let options = |radio: bool| -> Result<(Vec<Opt>, Option<usize>), String> {
            let Some(Value::Array(list)) = map.get("options") else {
                return Err(format!("{path}.options: missing (a list)"));
            };
            let most = if radio { MAX_RADIO } else { MAX_OPTIONS };
            if list.is_empty() {
                return Err(format!("{path}.options: empty"));
            }
            if list.len() > most {
                return Err(format!(
                    "{path}.options: {} options, at most {most}{}",
                    list.len(),
                    if radio {
                        " in a radio group (use a select)"
                    } else {
                        ""
                    }
                ));
            }
            let mut out: Vec<Opt> = Vec::new();
            for (i, item) in list.iter().enumerate() {
                let opt_path = format!("{path}.options[{i}]");
                let (value, label) = match item {
                    Value::String(_) | Value::Number(_) | Value::Bool(_) => {
                        (item.clone(), option_label(item))
                    }
                    Value::Object(o) => {
                        if let Some(key) = o.keys().find(|k| *k != "value" && *k != "label") {
                            return Err(format!(
                                "{opt_path}: `{key}` is not a key of an option (value, label)"
                            ));
                        }
                        let value = match o.get("value") {
                            Some(v @ (Value::String(_) | Value::Number(_) | Value::Bool(_))) => {
                                v.clone()
                            }
                            _ => {
                                return Err(format!(
                                    "{opt_path}.value: expected a text, a number, or a bool"
                                ));
                            }
                        };
                        let label = match o.get("label") {
                            None => option_label(&value),
                            Some(Value::String(s)) => s.clone(),
                            Some(_) => return Err(format!("{opt_path}.label: expected a text")),
                        };
                        (value, label)
                    }
                    _ => {
                        return Err(format!(
                            "{opt_path}: expected a text, a number, a bool, or {{ value, label }}"
                        ));
                    }
                };
                if out.iter().any(|o| o.value == value) {
                    return Err(format!("{opt_path}: the same value as an option before"));
                }
                out.push(Opt {
                    value,
                    label: tidy(&label),
                    current: false,
                });
            }
            // The value now: one of the options, else kept as an extra option "(current)"; null = not set.
            let chosen = match &original {
                Value::Null => None,
                v @ (Value::String(_) | Value::Number(_) | Value::Bool(_)) => {
                    match out.iter().position(|o| &o.value == v) {
                        Some(i) => Some(i),
                        None => {
                            out.push(Opt {
                                value: v.clone(),
                                label: tidy(&option_label(v)),
                                current: true,
                            });
                            Some(out.len() - 1)
                        }
                    }
                }
                other => {
                    return Err(format!(
                        "{value_path}: expected a text, a number, or a bool, got {other}"
                    ));
                }
            };
            Ok((out, chosen))
        };
        let kind = match kind {
            "checkbox" => Kind::Checkbox {
                on: match &original {
                    Value::Null => None,
                    Value::Bool(b) => Some(*b),
                    other => {
                        return Err(format!("{value_path}: expected true or false, got {other}"));
                    }
                },
            },
            "text" | "password" => Kind::Text {
                input: input_with(&text_value(&original)?),
                password: kind == "password",
                placeholder: tidy(&text("placeholder")?.unwrap_or_default()),
                rules: rules()?,
            },
            "number" => {
                let shown = match &original {
                    Value::Null => String::new(),
                    Value::Number(n) => n.to_string(),
                    other => return Err(format!("{value_path}: expected a number, got {other}")),
                };
                Kind::Number {
                    input: input_with(&shown),
                    min: number("min")?,
                    max: number("max")?,
                    integer: flag("integer")?,
                    required: flag("required")?,
                }
            }
            "multiline" => {
                let rows = match map.get("rows") {
                    None | Some(Value::Null) => 5,
                    Some(Value::Number(n)) => match n.as_u64() {
                        Some(n @ 1..=20) => n as usize,
                        _ => return Err(format!("{path}.rows: expected 1 to 20")),
                    },
                    Some(_) => return Err(format!("{path}.rows: expected a number")),
                };
                let start = text_value(&original)?;
                Kind::Multiline {
                    crlf: start.contains("\r\n"),
                    input: input_with(&start),
                    rows,
                    top: 0,
                    rules: rules()?,
                    placeholder: tidy(&text("placeholder")?.unwrap_or_default()),
                }
            }
            "radio" => {
                let (options, chosen) = options(true)?;
                Kind::Radio {
                    options,
                    chosen,
                    required: flag("required")?,
                }
            }
            _ => {
                let (options, chosen) = options(false)?;
                Kind::Select {
                    options,
                    chosen,
                    required: flag("required")?,
                    list: None,
                }
            }
        };
        let start = match &kind {
            Kind::Text { input, .. }
            | Kind::Number { input, .. }
            | Kind::Multiline { input, .. } => input.text.clone(),
            _ => String::new(),
        };
        Ok(Field {
            label: tidy(&label.unwrap_or_else(|| id.clone())),
            id,
            help: text("help")?.map(|h| tidy_lines(&h)),
            clearable: flag("clearable")?,
            original,
            touched: false,
            kind,
            start,
        })
    }

    pub fn is_group(&self) -> bool {
        matches!(self.kind, Kind::Group)
    }

    /// The value now: the original until the user changes something.
    pub fn value(&self) -> Value {
        if !self.touched {
            return self.original.clone();
        }
        let empty_is_null = self.clearable || self.original.is_null();
        match &self.kind {
            Kind::Group => Value::Null,
            Kind::Checkbox { on } => on.map_or(Value::Null, Value::Bool),
            Kind::Text { input, .. }
            | Kind::Multiline { input, .. }
            | Kind::Number { input, .. }
                if input.text == self.start =>
            {
                self.original.clone()
            }
            Kind::Text { input, .. } | Kind::Multiline { input, .. } => {
                if input.text.is_empty() && empty_is_null {
                    Value::Null
                } else {
                    Value::String(input.text.clone())
                }
            }
            Kind::Number { input, .. } => match parse_number(&input.text) {
                Some(n) => n,
                None => Value::Null,
            },
            Kind::Radio {
                options, chosen, ..
            }
            | Kind::Select {
                options, chosen, ..
            } => chosen.map_or(Value::Null, |i| options[i].value.clone()),
        }
    }

    /// The value is not the one of the request.
    pub fn changed(&self) -> bool {
        !self.is_group() && self.value() != self.original
    }

    /// The user edited it: its text is not the one of the start (a number "12px" gives no value, the same
    /// as a request with no value, but it is edited), or its value changed.
    pub fn edited(&self) -> bool {
        match &self.kind {
            Kind::Text { input, .. }
            | Kind::Number { input, .. }
            | Kind::Multiline { input, .. } => input.text != self.start,
            _ => self.changed(),
        }
    }

    /// A `required` field with no value (no text, no option): the answer must have a value.
    pub fn missing(&self) -> bool {
        match &self.kind {
            Kind::Group | Kind::Checkbox { .. } => false,
            Kind::Text { input, rules, .. } | Kind::Multiline { input, rules, .. } => {
                rules.required && input.text.is_empty()
            }
            Kind::Number {
                input, required, ..
            } => *required && input.text.trim().is_empty(),
            Kind::Radio {
                chosen, required, ..
            }
            | Kind::Select {
                chosen, required, ..
            } => *required && chosen.is_none(),
        }
    }

    /// Checks a field. `required` holds always; the other rules only for a field that the user edited (an old
    /// value that breaks them never blocks saving).
    pub fn problem(&self) -> Option<Problem> {
        if self.missing() {
            return Some(Problem::Required);
        }
        if !self.edited() {
            return None;
        }
        // (An empty required field is `missing`, above.)
        let text_problem = |text: &str, rules: &TextRules| {
            if text.is_empty() {
                return None;
            }
            if let Some(max) = rules.max
                && text.chars().count() > max
            {
                return Some(Problem::TooLong(max));
            }
            if let Some(re) = &rules.pattern
                && !re.is_match(text)
            {
                return Some(Problem::NoMatch);
            }
            None
        };
        match &self.kind {
            Kind::Group | Kind::Checkbox { .. } => None,
            Kind::Text { input, rules, .. } | Kind::Multiline { input, rules, .. } => {
                text_problem(&input.text, rules)
            }
            Kind::Number {
                input,
                min,
                max,
                integer,
                ..
            } => {
                let text = input.text.trim();
                if text.is_empty() {
                    // No number is "not set": only for a clearable field or one that had no value.
                    let ok = self.clearable || self.original.is_null();
                    return (!ok).then_some(Problem::Required);
                }
                let Some(n) = parse_number(text) else {
                    return Some(Problem::NotNumber);
                };
                let x = n.as_f64().unwrap_or(f64::NAN);
                if *integer && !n.is_i64() && !n.is_u64() {
                    return Some(Problem::NotInteger);
                }
                if let Some(m) = min
                    && x < *m
                {
                    return Some(Problem::Min(*m));
                }
                if let Some(m) = max
                    && x > *m
                {
                    return Some(Problem::Max(*m));
                }
                None
            }
            Kind::Radio { .. } | Kind::Select { .. } => None,
        }
    }

    /// The open list of a select, if it is open.
    pub fn list_open(&self) -> bool {
        matches!(self.kind, Kind::Select { list: Some(_), .. })
    }

    /// One key. `width` = the cells of the value (for Up and Down in a multi-line input).
    pub fn key(&mut self, key: Key, width: usize) -> Handled {
        let clearable = self.clearable;
        let used = match &mut self.kind {
            Kind::Group => return Handled::Not,
            Kind::Checkbox { on } => match key {
                Key::Text(" ") => {
                    *on = Some(!on.unwrap_or(false));
                    true
                }
                Key::Delete if clearable => {
                    *on = None;
                    true
                }
                _ => return Handled::Not,
            },
            Kind::Text { input, .. } | Kind::Number { input, .. } => match key {
                Key::Text(t) | Key::Paste(t) => {
                    let t = if matches!(key, Key::Paste(_)) {
                        one_line(t)
                    } else {
                        line_text(t)
                    };
                    if t.is_empty() {
                        return Handled::Used;
                    }
                    input.insert(&t);
                    true
                }
                Key::Backspace => {
                    input.backspace();
                    true
                }
                Key::Delete => {
                    input.delete();
                    true
                }
                Key::Left => {
                    input.left();
                    return Handled::Used;
                }
                Key::Right => {
                    input.right();
                    return Handled::Used;
                }
                Key::Home => {
                    input.home();
                    return Handled::Used;
                }
                Key::End => {
                    input.end();
                    return Handled::Used;
                }
                _ => return Handled::Not,
            },
            Kind::Multiline { input, crlf, .. } => {
                let nl = if *crlf { "\r\n" } else { "\n" };
                match key {
                    Key::Text(t) | Key::Paste(t) => {
                        let t = paste_text(t);
                        if t.is_empty() {
                            return Handled::Used;
                        }
                        input.insert(&if *crlf { t.replace('\n', nl) } else { t });
                        true
                    }
                    Key::Enter { ctrl: false, .. } => {
                        input.insert(nl);
                        true
                    }
                    Key::Backspace => {
                        // A CR LF goes as one new line.
                        let at = input.cursor();
                        if at >= 2
                            && char_at(&input.text, at - 1) == Some('\n')
                            && char_at(&input.text, at - 2) == Some('\r')
                        {
                            input.backspace();
                        }
                        input.backspace();
                        true
                    }
                    Key::Delete => {
                        let at = input.cursor();
                        if char_at(&input.text, at) == Some('\r')
                            && char_at(&input.text, at + 1) == Some('\n')
                        {
                            input.delete();
                        }
                        input.delete();
                        true
                    }
                    Key::Left => {
                        input.left();
                        if inside_crlf(&input.text, input.cursor()) {
                            input.left();
                        }
                        return Handled::Used;
                    }
                    Key::Right => {
                        input.right();
                        if inside_crlf(&input.text, input.cursor()) {
                            input.right();
                        }
                        return Handled::Used;
                    }
                    Key::Home => {
                        input.home();
                        return Handled::Used;
                    }
                    Key::End => {
                        input.end();
                        if inside_crlf(&input.text, input.cursor()) {
                            input.left();
                        }
                        return Handled::Used;
                    }
                    Key::Up | Key::Down => {
                        // By the rows that are drawn.
                        let (text, places) = drawn(&input.text);
                        let mut d = InputBox::default();
                        d.set(&text);
                        d.set_cursor(drawn_place(&places, input.cursor()));
                        let moved = if key == Key::Up {
                            d.up(width)
                        } else {
                            d.down(width)
                        };
                        if !moved {
                            return Handled::Not;
                        }
                        input.set_cursor(places[d.cursor()]);
                        return Handled::Used;
                    }
                    _ => return Handled::Not,
                }
            }
            Kind::Radio {
                options, chosen, ..
            } => {
                let last = options.len() - 1;
                match key {
                    Key::Left => *chosen = Some(chosen.map_or(0, |c| c.saturating_sub(1))),
                    Key::Right => *chosen = Some(chosen.map_or(0, |c| (c + 1).min(last))),
                    Key::Text(" ") => {
                        *chosen = Some(chosen.map_or(0, |c| if c >= last { 0 } else { c + 1 }))
                    }
                    Key::Delete if clearable => *chosen = None,
                    _ => return Handled::Not,
                }
                true
            }
            Kind::Select {
                options,
                chosen,
                list,
                ..
            } => match list {
                Some(open) => {
                    let shown = filtered(options, &open.filter.text);
                    let mut picked = false;
                    match key {
                        Key::Up => open.at = open.at.saturating_sub(1),
                        Key::Down => open.at = (open.at + 1).min(shown.len().saturating_sub(1)),
                        Key::PageUp => open.at = open.at.saturating_sub(LIST_ROWS),
                        Key::PageDown => {
                            open.at = (open.at + LIST_ROWS).min(shown.len().saturating_sub(1))
                        }
                        Key::Escape => *list = None,
                        Key::Enter { .. } | Key::Tab => {
                            let pick = shown.get(open.at).copied();
                            *list = None;
                            if let Some(i) = pick
                                && *chosen != Some(i)
                            {
                                *chosen = Some(i);
                                picked = true;
                            }
                        }
                        Key::Text(t) | Key::Paste(t) => {
                            open.filter.insert(&line_text(t));
                            open.at = 0;
                        }
                        Key::Backspace => {
                            open.filter.backspace();
                            open.at = 0;
                        }
                        _ => {}
                    }
                    // The list keeps all keys while it is open.
                    if !picked {
                        return Handled::Used;
                    }
                    true
                }
                None => match key {
                    Key::Enter { ctrl: false, .. } | Key::Text(" ") => {
                        let at = chosen.unwrap_or(0);
                        *list = Some(List {
                            filter: InputBox::default(),
                            at,
                        });
                        return Handled::Used;
                    }
                    Key::Text(t) => {
                        let mut filter = InputBox::default();
                        filter.insert(&line_text(t));
                        *list = Some(List { filter, at: 0 });
                        return Handled::Used;
                    }
                    Key::Left => {
                        *chosen = Some(chosen.map_or(0, |c| c.saturating_sub(1)));
                        true
                    }
                    Key::Right => {
                        *chosen = Some(chosen.map_or(0, |c| (c + 1).min(options.len() - 1)));
                        true
                    }
                    Key::Delete if clearable => {
                        *chosen = None;
                        true
                    }
                    _ => return Handled::Not,
                },
            },
        };
        if used {
            self.touched = true;
        }
        Handled::Used
    }

    /// The rows of the field: `label_w` cells for the label, `value_w` for the value. Gives the rows and, when
    /// focused, the row and cell of its cursor (or of its focus) in them.
    pub fn rows(
        &mut self,
        label_w: usize,
        value_w: usize,
        focused: bool,
        word: &dyn Fn(Word) -> String,
    ) -> (Vec<Row>, (usize, usize), usize) {
        let value_w = value_w.max(4);
        let label = fit(&self.label, label_w);
        let pad = " ".repeat(label_w - cells(&label) + 2);
        let lead = |first: bool| -> Vec<Span> {
            if first {
                vec![
                    span(" ", Look::Dim),
                    span(label.clone(), Look::Text),
                    span(pad.clone(), Look::Dim),
                ]
            } else {
                vec![span(" ".repeat(label_w + 3), Look::Dim)]
            }
        };
        let start = label_w + 3;
        let not_set = || span(word(Word::NotSet), Look::Dim);
        let unset = self.original.is_null() && !self.touched;
        let mut rows: Vec<Row> = Vec::new();
        let mut cursor = (0, start);
        let mut keep = None;
        match &mut self.kind {
            Kind::Group => {
                rows.push(Row {
                    spans: vec![
                        span(" ", Look::Dim),
                        span(self.label.clone(), Look::Heading),
                    ],
                    selected: false,
                });
                return (rows, (0, 0), 0);
            }
            Kind::Checkbox { on } => {
                let mut spans = lead(true);
                spans.push(span(
                    match on {
                        Some(true) => "[x]",
                        Some(false) => "[ ]",
                        None => "[ ]",
                    },
                    Look::Text,
                ));
                if on.is_none() {
                    spans.push(span(" ", Look::Dim));
                    spans.push(not_set());
                }
                rows.push(Row {
                    spans,
                    selected: focused,
                });
                cursor = (0, start + 1);
            }
            Kind::Text {
                input,
                password,
                placeholder,
                ..
            } => {
                let mut spans = lead(true);
                if input.text.is_empty() && !placeholder.is_empty() {
                    spans.extend(placeholder_spans(placeholder, value_w, focused));
                } else if unset && !focused {
                    spans.push(not_set());
                } else {
                    let text: String = if *password {
                        "•".repeat(input.text.chars().count())
                    } else {
                        shown(&input.text, true)
                    };
                    let (parts, at) = one_line_view(&text, input.cursor(), value_w, focused);
                    spans.extend(parts);
                    cursor = (0, start + at);
                }
                rows.push(Row {
                    spans,
                    selected: focused,
                });
            }
            Kind::Number { input, .. } => {
                let mut spans = lead(true);
                if unset && !focused {
                    spans.push(not_set());
                } else {
                    let text = shown(&input.text, true);
                    let (parts, at) = one_line_view(&text, input.cursor(), value_w, focused);
                    spans.extend(parts);
                    cursor = (0, start + at);
                }
                rows.push(Row {
                    spans,
                    selected: focused,
                });
            }
            Kind::Multiline {
                input,
                rows: view,
                top,
                placeholder,
                ..
            } => {
                let (text, places) = drawn(&input.text);
                let mut d = InputBox::default();
                d.set(&text);
                d.set_cursor(drawn_place(&places, input.cursor()));
                let (lines, (cl, cc)) = d.layout(value_w);
                // The view follows the cursor (a long value never hides it).
                if focused {
                    if cl < *top {
                        *top = cl;
                    } else if cl >= *top + *view {
                        *top = cl + 1 - *view;
                    }
                }
                *top = (*top).min(lines.len().saturating_sub(*view));
                for n in 0..*view {
                    let mut spans = lead(n == 0);
                    let line = lines.get(*top + n).cloned().unwrap_or_default();
                    if n == 0 && input.text.is_empty() && !placeholder.is_empty() {
                        spans.extend(placeholder_spans(placeholder, value_w, focused));
                    } else if n == 0 && unset && !focused {
                        spans.push(not_set());
                    } else if focused && *top + n == cl {
                        let before = fit(&line, cc);
                        let rest: String = line.chars().skip(before.chars().count()).collect();
                        let mut chars = rest.chars();
                        let under = chars.next().map_or(" ".to_owned(), String::from);
                        let after: String = chars.collect();
                        let fill =
                            value_w.saturating_sub(cells(&before) + cells(&under) + cells(&after));
                        spans.push(span(before, Look::Input));
                        spans.push(span(under, Look::Cursor));
                        spans.push(span(format!("{after}{}", " ".repeat(fill)), Look::Input));
                        cursor = (n, start + cc);
                    } else {
                        let fill = value_w.saturating_sub(cells(&line));
                        spans.push(span(format!("{line}{}", " ".repeat(fill)), Look::Input));
                    }
                    rows.push(Row {
                        spans,
                        selected: focused && n == 0,
                    });
                }
            }
            Kind::Radio {
                options, chosen, ..
            } => {
                // The options wrap onto more rows.
                let mut line: Vec<Span> = lead(true);
                let mut used = 0;
                let mut row_n = 0;
                for (i, opt) in options.iter().enumerate() {
                    let mark = if *chosen == Some(i) { "(•) " } else { "( ) " };
                    let mut text = opt.label.clone();
                    if opt.current {
                        text = format!("{text} {}", word(Word::Current));
                    }
                    let item = format!("{mark}{}", fit(&text, value_w.saturating_sub(4)));
                    let w = cells(&item) + 2;
                    if used > 0 && used + w > value_w {
                        rows.push(Row {
                            spans: std::mem::replace(&mut line, lead(false)),
                            selected: focused && row_n == 0,
                        });
                        row_n += 1;
                        used = 0;
                    }
                    if *chosen == Some(i) {
                        cursor = (row_n, start + used + 1);
                    }
                    let look = if *chosen == Some(i) {
                        Look::Text
                    } else {
                        Look::Dim
                    };
                    line.push(span(item, look));
                    line.push(span("  ", Look::Dim));
                    used += w;
                }
                if chosen.is_none() {
                    let w = cells(&word(Word::NotSet));
                    if used > 0 && used + w > value_w {
                        rows.push(Row {
                            spans: std::mem::replace(&mut line, lead(false)),
                            selected: focused && row_n == 0,
                        });
                        row_n += 1;
                    }
                    line.push(not_set());
                }
                rows.push(Row {
                    spans: line,
                    selected: focused && row_n == 0,
                });
            }
            Kind::Select {
                options,
                chosen,
                list,
                ..
            } => {
                let mut spans = lead(true);
                match chosen {
                    Some(i) => {
                        let opt = &options[*i];
                        let mut text = format!("▾ {}", opt.label);
                        if opt.current {
                            text = format!("{text} {}", word(Word::Current));
                        }
                        spans.push(span(fit(&text, value_w), Look::Text));
                    }
                    None => {
                        spans.push(span("▾ ", Look::Text));
                        spans.push(not_set());
                    }
                }
                rows.push(Row {
                    spans,
                    selected: focused && list.is_none(),
                });
                cursor = (0, start);
                if let Some(open) = list {
                    let shown = filtered(options, &open.filter.text);
                    open.at = open.at.min(shown.len().saturating_sub(1));
                    // The filter, then the options around the highlighted one.
                    let mut spans = lead(false);
                    let (parts, at) =
                        one_line_view(&open.filter.text, open.filter.cursor(), value_w, true);
                    spans.extend(parts);
                    rows.push(Row {
                        spans,
                        selected: false,
                    });
                    cursor = (1, start + at);
                    let first = open.at.saturating_sub(LIST_ROWS - 1);
                    for (k, &i) in shown.iter().enumerate().skip(first).take(LIST_ROWS) {
                        let mut spans = lead(false);
                        let mut text = options[i].label.clone();
                        if options[i].current {
                            text = format!("{text} {}", word(Word::Current));
                        }
                        spans.push(span(fit(&text, value_w), Look::Text));
                        if k == open.at {
                            keep = Some(rows.len());
                        }
                        rows.push(Row {
                            spans,
                            selected: k == open.at,
                        });
                    }
                }
            }
        }
        (rows, cursor, keep.unwrap_or(cursor.0))
    }
}

/// The options whose label has the filter (no case).
fn filtered(options: &[Opt], filter: &str) -> Vec<usize> {
    let filter = filter.to_lowercase();
    options
        .iter()
        .enumerate()
        .filter(|(_, o)| o.label.to_lowercase().contains(&filter))
        .map(|(i, _)| i)
        .collect()
}

/// A number from the text of a number field: an integer stays an integer (`13`, not `13.0`).
pub fn parse_number(text: &str) -> Option<Value> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if !text.contains(['.', 'e', 'E'])
        && let Ok(n) = text.parse::<i64>()
    {
        return Some(Value::from(n));
    }
    let x: f64 = text.parse().ok()?;
    serde_json::Number::from_f64(x).map(Value::Number)
}

/// One line of an input in `width` cells: the part around the cursor, and the cell of the cursor. It looks only
/// at the chars near the cursor, so a long value is cheap.
fn one_line_view(text: &str, cursor: usize, width: usize, focused: bool) -> (Vec<Span>, usize) {
    use unicode_width::UnicodeWidthChar;
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    // Back from the cursor while it fits (one cell is kept for the cursor).
    let room = width.saturating_sub(1);
    let mut from = cursor;
    let mut used = 0;
    while from > 0 {
        let w = chars[from - 1].width().unwrap_or(0);
        if used + w > room {
            break;
        }
        used += w;
        from -= 1;
    }
    let before: String = chars[from..cursor].iter().collect();
    let at = used;
    let mut out = vec![span(before, Look::Input)];
    // After the cursor: only as many chars as can be seen.
    let rest_room = width.saturating_sub(at);
    let mut rest = String::new();
    let mut rest_used = 0;
    for &c in &chars[cursor..] {
        let w = c.width().unwrap_or(0);
        if rest_used + w > rest_room {
            break;
        }
        rest_used += w;
        rest.push(c);
    }
    if focused {
        let mut it = rest.chars();
        let under = it.next().map_or(" ".to_owned(), String::from);
        let after = fit(it.as_str(), width.saturating_sub(at + cells(&under)));
        let fill = width.saturating_sub(at + cells(&under) + cells(&after));
        out.push(span(under, Look::Cursor));
        out.push(span(format!("{after}{}", " ".repeat(fill)), Look::Input));
    } else {
        let fill = width.saturating_sub(at + cells(&rest));
        out.push(span(format!("{rest}{}", " ".repeat(fill)), Look::Input));
    }
    (out, at)
}

/// The placeholder of an empty input (after the cursor when it is focused).
fn placeholder_spans(placeholder: &str, width: usize, focused: bool) -> Vec<Span> {
    if focused {
        vec![
            span(" ", Look::Cursor),
            span(fit(placeholder, width.saturating_sub(1)), Look::Dim),
        ]
    } else {
        vec![span(fit(placeholder, width), Look::Dim)]
    }
}

/// Wraps a help text to `width` cells (for the form).
pub fn help_rows(text: &str, indent: usize, width: usize) -> Vec<Row> {
    text.lines()
        .flat_map(|l| wrap(l, width.max(8)))
        .map(|l| Row {
            spans: vec![span(" ".repeat(indent), Look::Dim), span(l, Look::Dim)],
            selected: false,
        })
        .collect()
}
