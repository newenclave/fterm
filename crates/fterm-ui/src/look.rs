//! How text looks (a role, not a color), screen rows of looked text, and their drawing as ANSI text.
//! The app maps each look to the colors of its theme, so this crate has no colors of its own.

use std::fmt::Write;

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
    /// A problem (a value that is not valid).
    Error,
}

/// A part of a screen line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub look: Look,
}

/// A span with this text and look.
pub fn span(text: impl Into<String>, look: Look) -> Span {
    Span {
        text: text.into(),
        look,
    }
}

/// One screen line: its parts, and if it has the background of the selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub spans: Vec<Span>,
    pub selected: bool,
}

impl Row {
    /// The text with no looks.
    pub fn plain(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

/// A color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// The style of a look: colors (`None` = the color of the pane) and text attributes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub strike: bool,
}

fn fg(c: Rgb) -> String {
    format!("\x1b[38;2;{};{};{}m", c.r, c.g, c.b)
}

fn bg(c: Rgb) -> String {
    format!("\x1b[48;2;{};{};{}m", c.r, c.g, c.b)
}

/// The rows as ANSI text for a pane with no program, from its top left: each look in `style(look)`, a selected
/// row in the background `selected`, and the rest of each line in the background of its row. The cursor of the
/// pane is hidden (a text UI draws its own).
pub fn render_ansi(rows: &[Row], style: impl Fn(Look) -> Style, selected: Rgb) -> String {
    let mut out = String::from("\x1b[H\x1b[?25l");
    for (n, row) in rows.iter().enumerate() {
        let back = if row.selected {
            bg(selected)
        } else {
            "\x1b[49m".to_owned()
        };
        out.push_str("\x1b[0m");
        out.push_str(&back);
        for s in &row.spans {
            let st = style(s.look);
            let mut code = String::new();
            if st.bold {
                code.push_str("\x1b[1m");
            }
            if st.strike {
                code.push_str("\x1b[9m");
            }
            if let Some(c) = st.fg {
                code.push_str(&fg(c));
            }
            if let Some(c) = st.bg {
                code.push_str(&bg(c));
            }
            // The text may come from outside (an agent): no control char reaches the pane, so no escape sequence.
            let text: String = s
                .text
                .chars()
                .filter_map(|c| match c {
                    '\t' => Some(' '),
                    c if c.is_control() => None,
                    c => Some(c),
                })
                .collect();
            let _ = write!(out, "\x1b[0m{back}{code}{text}");
        }
        // The rest of the line in the row's background.
        let _ = write!(out, "\x1b[0m{back}\x1b[K\x1b[0m");
        if n + 1 < rows.len() {
            out.push_str("\r\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgb = Rgb { r: 200, g: 0, b: 0 };
    const SEL: Rgb = Rgb { r: 1, g: 2, b: 3 };

    fn style(look: Look) -> Style {
        match look {
            Look::Title => Style {
                fg: Some(RED),
                bold: true,
                ..Style::default()
            },
            Look::Remove => Style {
                fg: Some(RED),
                strike: true,
                ..Style::default()
            },
            Look::Input => Style {
                bg: Some(Rgb { r: 9, g: 9, b: 9 }),
                ..Style::default()
            },
            _ => Style::default(),
        }
    }

    #[test]
    fn looks_become_ansi_with_their_attributes() {
        let rows = [
            Row {
                spans: vec![span("Plan", Look::Title)],
                selected: false,
            },
            Row {
                spans: vec![span("old", Look::Remove), span("x", Look::Input)],
                selected: true,
            },
        ];
        let out = render_ansi(&rows, style, SEL);
        assert!(
            out.starts_with("\x1b[H\x1b[?25l"),
            "from the top left, no cursor"
        );
        assert!(
            out.contains("\x1b[1m\x1b[38;2;200;0;0mPlan"),
            "bold title: {out:?}"
        );
        assert!(
            out.contains("\x1b[9m\x1b[38;2;200;0;0mold"),
            "struck out: {out:?}"
        );
        assert!(out.contains("\x1b[48;2;9;9;9mx"), "the input background");
        assert!(
            out.contains("\x1b[48;2;1;2;3m"),
            "the selected row background"
        );
        assert_eq!(
            out.matches("\r\n").count(),
            1,
            "a new line between rows only"
        );
        assert!(
            out.ends_with("\x1b[K\x1b[0m"),
            "the rest of the line is cleared"
        );
    }

    #[test]
    fn control_chars_in_the_text_do_not_reach_the_pane() {
        // A label from an agent with an escape sequence (here: OSC 52, a write to the clipboard).
        let rows = [Row {
            spans: vec![span("a\u{1b}]52;c;eA==\u{7}b\u{9b}2Jc\td", Look::Text)],
            selected: false,
        }];
        let out = render_ansi(&rows, style, SEL);
        assert!(!out.contains("\u{1b}]"), "no OSC: {out:?}");
        assert!(!out.contains('\u{7}') && !out.contains('\u{9b}'), "{out:?}");
        assert!(
            out.contains("a]52;c;eA==b2Jc d"),
            "the rest is plain text, a tab is a space: {out:?}"
        );
    }

    #[test]
    fn a_row_as_plain_text() {
        let row = Row {
            spans: vec![span("a", Look::Dim), span("b", Look::Text)],
            selected: false,
        };
        assert_eq!(row.plain(), "ab");
    }
}
