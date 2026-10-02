//! The AI panel model (Phase 8): the turns of the chat, the input box, and the lines to draw.
//! Pure: no drawing and no network.

use fterm_render::dock::{ChatLine, ChatStyle};
use unicode_width::UnicodeWidthChar;

/// The text that the user types (many lines). The cursor is a char index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputBox {
    pub text: String,
    cursor: usize,
}

impl InputBox {
    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn insert(&mut self, text: &str) {
        let at = self.byte(self.cursor);
        self.text.insert_str(at, text);
        self.cursor += text.chars().count();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let at = self.byte(self.cursor);
        self.text.remove(at);
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.chars().count() {
            let at = self.byte(self.cursor);
            self.text.remove(at);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.text.chars().count());
    }

    /// The start / the end of the current line.
    pub fn home(&mut self) {
        let before: Vec<char> = self.text.chars().take(self.cursor).collect();
        self.cursor = before.iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
    }

    pub fn end(&mut self) {
        let after = self.text.chars().skip(self.cursor).position(|c| c == '\n');
        self.cursor = match after {
            Some(n) => self.cursor + n,
            None => self.text.chars().count(),
        };
    }

    /// Takes the text out (to send it) and empties the box.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_owned();
        self.cursor = self.text.chars().count();
    }

    /// The byte place of a char index.
    fn byte(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(i, _)| i)
    }

    /// The lines to draw (each line cut at `width` cells) and the place of the cursor (line, cell).
    pub fn layout(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(1);
        let mut lines = Vec::new();
        let mut cursor = (0, 0);
        let mut seen = 0;
        for logical in self.text.split('\n') {
            let count = logical.chars().count();
            let first_row = lines.len();
            lines.extend(cut(logical, width));
            if self.cursor >= seen && self.cursor <= seen + count {
                let used: usize = logical
                    .chars()
                    .take(self.cursor - seen)
                    .map(|c| c.width().unwrap_or(0))
                    .sum();
                cursor = (first_row + used / width, used % width);
            }
            seen += count + 1;
        }
        (lines, cursor)
    }
}

/// A part of an answer: text, or a code block (```lang ... ```).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Text(String),
    Code { lang: String, code: String },
}

fn text_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l))
}

/// Splits an answer into text and code blocks. A code block that is not closed yet (the answer still
/// streams) is a code block too.
pub fn blocks(text: &str) -> Vec<Block> {
    fn flush(text: &mut Vec<&str>, out: &mut Vec<Block>) {
        let joined = text.join("\n");
        let trimmed = joined.trim_matches('\n');
        if !trimmed.trim().is_empty() {
            out.push(Block::Text(trimmed.to_owned()));
        }
        text.clear();
    }
    let mut out = Vec::new();
    let mut plain: Vec<&str> = Vec::new();
    let mut code: Option<(String, Vec<&str>)> = None;
    for line in text_lines(text) {
        let fence = line.trim_start().strip_prefix("```");
        match (&mut code, fence) {
            (None, Some(lang)) => {
                flush(&mut plain, &mut out);
                code = Some((lang.trim().to_owned(), Vec::new()));
            }
            (Some(_), Some(_)) => {
                if let Some((lang, lines)) = code.take() {
                    out.push(Block::Code {
                        lang,
                        code: lines.join("\n"),
                    });
                }
            }
            (Some((_, lines)), None) => lines.push(line),
            (None, None) => plain.push(line),
        }
    }
    match code {
        Some((lang, lines)) => out.push(Block::Code {
            lang,
            code: lines.join("\n"),
        }),
        None => flush(&mut plain, &mut out),
    }
    out
}

/// Cuts a line into pieces of at most `width` cells (no word wrap).
fn cut(line: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for c in line.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(c);
        used += w;
    }
    out.push(current);
    out
}

fn cells(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// One turn of the chat for the layout.
pub struct TurnView<'a> {
    pub user: bool,
    pub text: &'a str,
    /// The error of a failed answer.
    pub error: Option<&'a str>,
    /// The answer streams now.
    pub streaming: bool,
}

/// All lines of the chat, wrapped at `width` cells.
pub fn layout(turns: &[TurnView], width: usize) -> Vec<ChatLine> {
    fn push(lines: &mut Vec<ChatLine>, text: String, style: ChatStyle) {
        lines.push(ChatLine { text, style });
    }
    let mut lines = Vec::new();
    for (i, turn) in turns.iter().enumerate() {
        if i > 0 {
            push(&mut lines, String::new(), ChatStyle::Note);
        }
        let start = lines.len();
        if turn.user {
            for line in text_lines(turn.text) {
                for piece in wrap(line, width) {
                    push(&mut lines, piece, ChatStyle::User);
                }
            }
            continue;
        }
        for block in blocks(turn.text) {
            match block {
                Block::Text(text) => {
                    for line in text_lines(&text) {
                        for piece in wrap(line, width) {
                            push(&mut lines, piece, ChatStyle::Answer);
                        }
                    }
                }
                Block::Code { lang, code } => {
                    let header = if lang.is_empty() {
                        "code".to_owned()
                    } else {
                        lang
                    };
                    push(&mut lines, header, ChatStyle::CodeHeader);
                    for line in text_lines(&code) {
                        for piece in cut(line, width.max(1)) {
                            push(&mut lines, piece, ChatStyle::Code);
                        }
                    }
                }
            }
        }
        if turn.streaming {
            match lines[start..].last_mut() {
                Some(last) => last.text.push('▍'),
                None => push(&mut lines, "Thinking…".to_owned(), ChatStyle::Note),
            }
        }
        if let Some(error) = turn.error {
            for line in text_lines(error) {
                for piece in wrap(line, width) {
                    push(&mut lines, piece, ChatStyle::Error);
                }
            }
        }
    }
    lines
}

/// Wraps one line at word ends; a word that is too long is cut.
pub fn wrap(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut current = String::new();
    for word in line.split(' ') {
        if current.is_empty() {
            current = word.to_owned();
        } else if cells(&current) + 1 + cells(word) <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            out.push(std::mem::take(&mut current));
            current = word.to_owned();
        }
        // A word that is longer than the line: cut it.
        if cells(&current) > width {
            let mut pieces = cut(&current, width);
            current = pieces.pop().unwrap_or_default();
            out.extend(pieces);
        }
    }
    out.push(current);
    out
}

/// The most lines of an output or a selection that go with a question (the end of it).
pub const MAX_CONTEXT_LINES: usize = 200;

/// Something from the terminal that goes with the next question. The user sees it as a chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextItem {
    /// The last command and its exit code.
    Command { command: String, exit: Option<i32> },
    /// The output of the last command.
    Output(String),
    /// The selected text.
    Selection(String),
}

impl ContextItem {
    /// The text of the chip: "output (42 lines)".
    pub fn label(&self) -> String {
        let lines = |text: &str| {
            let n = text.lines().count().max(1);
            if n == 1 {
                "1 line".to_owned()
            } else {
                format!("{n} lines")
            }
        };
        match self {
            Self::Command {
                exit: Some(code), ..
            } => format!("last command (exit {code})"),
            Self::Command { exit: None, .. } => "last command".to_owned(),
            Self::Output(text) => format!("output ({})", lines(text)),
            Self::Selection(text) => format!("selection ({})", lines(text)),
        }
    }
}

/// The text that goes to the AI: the context (in tags, so the model knows what is what), then the question.
pub fn build_message(question: &str, context: &[ContextItem]) -> String {
    if context.is_empty() {
        return question.to_owned();
    }
    // The end of a long text (the end of an output says most).
    let tail = |text: &str| {
        let lines: Vec<&str> = text.lines().collect();
        let cut = lines.len().saturating_sub(MAX_CONTEXT_LINES);
        let mut out = String::new();
        if cut > 0 {
            out.push_str(&format!("(the first {cut} lines are left out)\n"));
        }
        out.push_str(&lines[cut..].join("\n"));
        out
    };
    let mut text = String::from("<terminal>\n");
    for item in context {
        match item {
            ContextItem::Command { command, exit } => {
                text.push_str(&format!("The last command: {command}"));
                if let Some(code) = exit {
                    text.push_str(&format!(" (exit code {code})"));
                }
                text.push('\n');
            }
            ContextItem::Output(output) => {
                text.push_str(&format!("Its output:\n```text\n{}\n```\n", tail(output)));
            }
            ContextItem::Selection(selection) => {
                text.push_str(&format!(
                    "The selected text:\n```text\n{}\n```\n",
                    tail(selection)
                ));
            }
        }
    }
    text.push_str("</terminal>\n\n");
    text.push_str(question);
    text
}

/// The last code block of the last answer (for "put into the terminal" and "copy").
pub fn last_code_block(turns: &[Turn]) -> Option<String> {
    let answer = turns.iter().rev().find(|t| !t.user)?;
    blocks(&answer.text)
        .into_iter()
        .rev()
        .find_map(|b| match b {
            Block::Code { code, .. } if !code.trim().is_empty() => Some(code),
            _ => None,
        })
}

/// One turn of the chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub user: bool,
    /// What the chat shows.
    pub text: String,
    pub error: Option<String>,
    pub streaming: bool,
    /// What goes to the AI when it is not `text` (a question with its context).
    pub sent: Option<String>,
}

/// The chat of the AI panel: its turns, the input, the scroll, and the running request.
#[derive(Default)]
pub struct Session {
    pub turns: Vec<Turn>,
    pub input: InputBox,
    /// Lines up from the end of the chat (0 = the newest).
    pub scroll: usize,
    /// The id of the running request (answers of older ids are not for us).
    pub running: Option<u64>,
    next_id: u64,
    /// The last question (Up in an empty input brings it back).
    pub last_question: Option<String>,
    /// What goes with the next question (the chips over the input).
    pub context: Vec<ContextItem>,
}

impl Session {
    /// Starts a question: a user turn and an empty answer that streams. Returns the request id,
    /// or `None` when the text is empty or a request runs.
    #[cfg(test)]
    pub fn ask(&mut self, text: &str) -> Option<u64> {
        self.start(text, text, None)
    }

    /// Like `ask`, but the chat shows `display` and the AI gets `sent` (the question with its context).
    /// Up in the input brings back only the first line of `display` (the question).
    pub fn ask_with(&mut self, display: &str, sent: String) -> Option<u64> {
        let question = display.lines().next().unwrap_or_default().to_owned();
        self.start(display, &question, Some(sent))
    }

    fn start(&mut self, display: &str, question: &str, sent: Option<String>) -> Option<u64> {
        let text = display.trim();
        if text.is_empty() || self.running.is_some() {
            return None;
        }
        self.next_id += 1;
        self.turns.push(Turn {
            user: true,
            text: text.to_owned(),
            error: None,
            streaming: false,
            sent,
        });
        self.turns.push(Turn {
            user: false,
            text: String::new(),
            error: None,
            streaming: true,
            sent: None,
        });
        self.running = Some(self.next_id);
        self.last_question = Some(question.trim().to_owned());
        self.scroll = 0;
        Some(self.next_id)
    }

    /// The messages to send: the turns before the streaming answer, without the failed pairs.
    pub fn messages(&self) -> Vec<fterm_ai::Message> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.turns.len() {
            let turn = &self.turns[i];
            let answer = self.turns.get(i + 1).filter(|t| !t.user);
            match answer {
                // The question that waits for its answer now: send it.
                Some(a) if a.streaming => {
                    out.push(fterm_ai::Message {
                        role: fterm_ai::Role::User,
                        content: turn.sent.clone().unwrap_or_else(|| turn.text.clone()),
                    });
                }
                // A failed answer: the pair is not sent.
                Some(a) if a.error.is_some() => {}
                Some(a) => {
                    out.push(fterm_ai::Message {
                        role: fterm_ai::Role::User,
                        content: turn.sent.clone().unwrap_or_else(|| turn.text.clone()),
                    });
                    out.push(fterm_ai::Message {
                        role: fterm_ai::Role::Assistant,
                        content: a.text.clone(),
                    });
                }
                None => {}
            }
            i += 2;
        }
        out
    }

    /// An event of request `id`. Returns `true` when the chat changed.
    pub fn event(&mut self, id: u64, event: fterm_ai::Event) -> bool {
        if self.running != Some(id) {
            return false;
        }
        let Some(turn) = self.turns.last_mut().filter(|t| !t.user) else {
            return false;
        };
        match event {
            fterm_ai::Event::Delta(text) => turn.text.push_str(&text),
            fterm_ai::Event::Done { .. } => {
                turn.streaming = false;
                self.running = None;
            }
            fterm_ai::Event::Failed(err) => {
                turn.streaming = false;
                turn.error = Some(err.to_string());
                self.running = None;
            }
        }
        true
    }

    /// Forgets all turns (a new chat).
    pub fn clear(&mut self) {
        self.turns.clear();
        self.scroll = 0;
    }

    pub fn views(&self) -> Vec<TurnView<'_>> {
        self.turns
            .iter()
            .map(|t| TurnView {
                user: t.user,
                text: &t.text,
                error: t.error.as_deref(),
                streaming: t.streaming,
            })
            .collect()
    }
}

/// The system text: who the assistant is, where it runs, and the user's own instructions.
pub fn system_prompt(os: &str, shell: &str, cwd: Option<&str>, extra: &str) -> String {
    // A chat first: small models gave a command even for "are you here?" when the prompt spoke
    // only of commands.
    let mut text = format!(
        "You are a helpful assistant in the AI panel of fterm, a terminal. Talk with the user as in a \
         normal chat, in the language of the user. Answer greetings, questions, and requests for an \
         explanation in plain text, with no command. Only when the user wants to do something in the \
         terminal, give the command: the user works on {os}, and the shell of the active pane is {shell}."
    );
    if let Some(cwd) = cwd {
        text.push_str(&format!(" The current folder is {cwd}."));
    }
    text.push_str(
        " Put each command in a fenced code block with the shell name (for example ```powershell), \
         one command per block when you can, so the user can put it into the terminal. Give short and \
         clear answers.",
    );
    if !extra.trim().is_empty() {
        text.push_str("\n\n");
        text.push_str(extra.trim());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_moving() {
        let mut b = InputBox::default();
        b.insert("helo");
        b.left();
        b.insert("l");
        assert_eq!((b.text.as_str(), b.cursor()), ("hello", 4));
        b.end();
        b.insert("\nwörld");
        b.home();
        assert_eq!(b.cursor(), 6, "the start of the second line");
        b.delete();
        assert_eq!(b.text, "hello\nörld");
        b.right();
        b.backspace();
        assert_eq!(b.text, "hello\nrld");
        b.home();
        b.backspace();
        assert_eq!(
            b.text, "hellorld",
            "backspace at a line start joins the lines"
        );
        assert_eq!(b.take(), "hellorld");
        assert_eq!((b.text.as_str(), b.cursor()), ("", 0));
        b.backspace();
        b.left();
        b.delete();
        assert_eq!(b.text, "", "nothing breaks at the ends");
    }

    #[test]
    fn the_input_layout() {
        let mut b = InputBox::default();
        b.insert("abcdefgh\nxy");
        let (lines, cursor) = b.layout(5);
        assert_eq!(lines, ["abcde", "fgh", "xy"]);
        assert_eq!(cursor, (2, 2));
        b.set("");
        assert_eq!(b.layout(5), (vec![String::new()], (0, 0)));
        b.set("abcde");
        assert_eq!(
            b.layout(5).1,
            (1, 0),
            "a full line: the cursor goes to the next line"
        );
    }

    #[test]
    fn code_blocks() {
        let text = "Run this:\n```powershell\nGet-ChildItem | sort Length\n```\nThat is all.";
        assert_eq!(
            blocks(text),
            [
                Block::Text("Run this:".into()),
                Block::Code {
                    lang: "powershell".into(),
                    code: "Get-ChildItem | sort Length".into()
                },
                Block::Text("That is all.".into()),
            ]
        );
        // Still streaming: the block is not closed yet.
        assert_eq!(
            blocks("```bash\nls -la"),
            [Block::Code {
                lang: "bash".into(),
                code: "ls -la".into()
            }]
        );
        assert_eq!(blocks("just text"), [Block::Text("just text".into())]);
    }

    #[test]
    fn word_wrap() {
        assert_eq!(wrap("the quick brown fox", 10), ["the quick", "brown fox"]);
        assert_eq!(
            wrap("abcdefghijkl", 5),
            ["abcde", "fghij", "kl"],
            "a long word is cut"
        );
        assert_eq!(wrap("", 5), [""]);
        assert_eq!(
            wrap("日本語の文", 4),
            ["日本", "語の", "文"],
            "wide chars take two cells"
        );
    }

    #[test]
    fn the_chat_lines() {
        let turns = [
            TurnView {
                user: true,
                text: "list big files",
                error: None,
                streaming: false,
            },
            TurnView {
                user: false,
                text: "Use this:\n```powershell\ngci | sort Length\n```",
                error: None,
                streaming: false,
            },
            TurnView {
                user: true,
                text: "thanks",
                error: None,
                streaming: false,
            },
            TurnView {
                user: false,
                text: "",
                error: Some("no API key"),
                streaming: false,
            },
        ];
        let lines = layout(&turns, 40);
        let styled: Vec<(ChatStyle, &str)> =
            lines.iter().map(|l| (l.style, l.text.as_str())).collect();
        assert_eq!(
            styled,
            [
                (ChatStyle::User, "list big files"),
                (ChatStyle::Note, ""),
                (ChatStyle::Answer, "Use this:"),
                (ChatStyle::CodeHeader, "powershell"),
                (ChatStyle::Code, "gci | sort Length"),
                (ChatStyle::Note, ""),
                (ChatStyle::User, "thanks"),
                (ChatStyle::Note, ""),
                (ChatStyle::Error, "no API key"),
            ]
        );
    }

    #[test]
    fn a_streaming_answer_shows_that_it_works() {
        let turns = [TurnView {
            user: false,
            text: "",
            error: None,
            streaming: true,
        }];
        let lines = layout(&turns, 40);
        assert_eq!(
            lines.last().map(|l| (l.style, l.text.as_str())),
            Some((ChatStyle::Note, "Thinking…"))
        );
        let turns = [TurnView {
            user: false,
            text: "Hel",
            error: None,
            streaming: true,
        }];
        assert_eq!(
            layout(&turns, 40)[0].text,
            "Hel▍",
            "a block cursor at the end while it streams"
        );
    }

    fn answer(s: &mut Session, id: u64, text: &str) {
        s.event(id, fterm_ai::Event::Delta(text.into()));
        s.event(
            id,
            fterm_ai::Event::Done {
                stop_reason: Some("end_turn".into()),
                input_tokens: None,
                output_tokens: None,
            },
        );
    }

    #[test]
    fn a_question_and_its_streamed_answer() {
        let mut s = Session::default();
        assert_eq!(s.ask("   "), None, "nothing to ask");
        let id = s.ask("why?").unwrap();
        assert_eq!(s.ask("again"), None, "one request at a time");
        assert_eq!(s.turns.len(), 2);
        assert!(s.turns[1].streaming);
        assert!(s.event(id, fterm_ai::Event::Delta("Be".into())));
        assert!(s.event(id, fterm_ai::Event::Delta("cause".into())));
        assert!(
            !s.event(id + 7, fterm_ai::Event::Delta("old".into())),
            "an old request is not ours"
        );
        answer(&mut s, id, "");
        assert_eq!(s.turns[1].text, "Because");
        assert!(!s.turns[1].streaming);
        assert_eq!(s.running, None);
        assert_eq!(s.last_question.as_deref(), Some("why?"));
    }

    #[test]
    fn messages_alternate_and_skip_failed_pairs() {
        use fterm_ai::Role;
        let mut s = Session::default();
        let a = s.ask("one").unwrap();
        answer(&mut s, a, "1");
        let b = s.ask("two").unwrap();
        s.event(b, fterm_ai::Event::Failed(fterm_ai::AiError::Stopped));
        assert_eq!(s.turns[3].error.as_deref(), Some("stopped"));
        s.ask("three").unwrap();
        let sent: Vec<(Role, String)> = s
            .messages()
            .into_iter()
            .map(|m| (m.role, m.content))
            .collect();
        assert_eq!(
            sent,
            [
                (Role::User, "one".to_owned()),
                (Role::Assistant, "1".to_owned()),
                (Role::User, "three".to_owned()),
            ],
            "the failed pair is not sent, and the streaming answer is not sent"
        );
    }

    #[test]
    fn the_system_prompt() {
        let text = system_prompt(
            "Windows",
            "PowerShell",
            Some("C:/work"),
            "Answer in Russian.",
        );
        assert!(
            text.contains("Windows") && text.contains("PowerShell") && text.contains("C:/work")
        );
        assert!(text.contains("```"), "it asks for commands in code blocks");
        assert!(text.ends_with("Answer in Russian."));
        assert!(!system_prompt("Linux", "bash", None, "").contains("folder"));
    }

    #[test]
    fn the_panel_is_a_chat_not_only_commands() {
        // A small model gave a PowerShell block even for "are you here?".
        let text = system_prompt("Windows", "PowerShell", None, "");
        assert!(text.contains("plain text"), "{text}");
        assert!(text.contains("language of the user"), "{text}");
        assert!(
            text.contains("Only when the user wants to do something in the terminal"),
            "{text}"
        );
    }

    #[test]
    fn chip_labels() {
        let cmd = ContextItem::Command {
            command: "cargo build".into(),
            exit: Some(101),
        };
        assert_eq!(cmd.label(), "last command (exit 101)");
        assert_eq!(
            ContextItem::Command {
                command: "x".into(),
                exit: None
            }
            .label(),
            "last command"
        );
        assert_eq!(
            ContextItem::Output("a\nb\nc".into()).label(),
            "output (3 lines)"
        );
        assert_eq!(
            ContextItem::Selection("one".into()).label(),
            "selection (1 line)"
        );
    }

    #[test]
    fn a_message_with_context() {
        let context = [
            ContextItem::Command {
                command: "cargo build".into(),
                exit: Some(101),
            },
            ContextItem::Output("error[E0425]: cannot find value `x`".into()),
        ];
        let text = build_message("Why?", &context);
        assert!(text.starts_with("<terminal>"), "{text}");
        assert!(text.contains("The last command: cargo build (exit code 101)"));
        assert!(text.contains("```text\nerror[E0425]: cannot find value `x`\n```"));
        assert!(text.ends_with("</terminal>\n\nWhy?"));
        assert_eq!(
            build_message("Why?", &[]),
            "Why?",
            "no context: only the question"
        );
    }

    #[test]
    fn a_long_output_keeps_its_end() {
        let long: Vec<String> = (0..500).map(|i| format!("line {i}")).collect();
        let text = build_message("?", &[ContextItem::Output(long.join("\n"))]);
        assert!(text.contains("line 499"));
        assert!(!text.contains("line 299\n"), "only the last 200 lines");
        assert!(text.contains("line 300"));
        assert!(text.contains("(the first 300 lines are left out)"));
    }

    #[test]
    fn the_last_code_block() {
        let turn = |user: bool, text: &str| Turn {
            user,
            text: text.into(),
            error: None,
            streaming: false,
            sent: None,
        };
        let turns = [
            turn(true, "q"),
            turn(
                false,
                "First:\n```bash\nls\n```\nThen:\n```bash\nls -la\n```",
            ),
        ];
        assert_eq!(last_code_block(&turns).as_deref(), Some("ls -la"));
        assert_eq!(last_code_block(&[turn(false, "no code")]), None);
        assert_eq!(last_code_block(&[]), None);
    }

    #[test]
    fn the_chat_shows_the_question_and_the_api_gets_the_context() {
        let mut s = Session::default();
        let id = s
            .ask_with(
                "why?\n+ output (2 lines)",
                "<terminal>...</terminal>\n\nwhy?".into(),
            )
            .unwrap();
        assert_eq!(s.turns[0].text, "why?\n+ output (2 lines)");
        assert_eq!(s.messages()[0].content, "<terminal>...</terminal>\n\nwhy?");
        answer(&mut s, id, "because");
        assert_eq!(
            s.last_question.as_deref(),
            Some("why?"),
            "Up brings back the question, not the chips"
        );
    }
}
