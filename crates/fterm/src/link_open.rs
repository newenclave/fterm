//! What fterm does with a link: open it at once, ask first, or not open it at all.

use fterm_term::links::Link;

/// What to do with a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    /// Open it at once.
    Now,
    /// Show the URL and ask first.
    Ask,
    /// Do not open it: a scheme that could start other programs.
    Refuse,
}

/// Schemes that fterm opens. An OSC 8 link can carry any URI (`ms-settings:`, a custom handler), and opening one
/// can start another program, so other schemes are refused.
const ALLOWED: [&str; 5] = ["https:", "http:", "ftp:", "mailto:", "file:"];

pub fn decide(link: &Link) -> Open {
    let url = link.url.trim().to_ascii_lowercase();
    if !ALLOWED.iter().any(|s| url.starts_with(s)) {
        return Open::Refuse;
    }
    // A file can be a program: always ask.
    if url.starts_with("file:") {
        return Open::Ask;
    }
    if link.osc8 && !text_is_url(&link.text, &link.url) {
        return Open::Ask;
    }
    Open::Now
}

/// The text shows the URL (maybe with no scheme, or with no `/` at the end).
fn text_is_url(text: &str, url: &str) -> bool {
    let norm = |s: &str| {
        let s = s.trim().to_ascii_lowercase();
        let s = s
            .split_once("://")
            .map_or(s.as_str(), |(_, rest)| rest)
            .to_owned();
        s.trim_end_matches('/').to_owned()
    };
    norm(text) == norm(url)
}

/// The letters of the labels, the easy keys first.
const LETTERS: &str = "asdfghjklqwertyuiopzxcvbnm";

/// `n` labels: one letter each when they fit, else two letters each (so no label is the start of another).
pub fn labels(n: usize) -> Vec<String> {
    let letters: Vec<char> = LETTERS.chars().collect();
    if n <= letters.len() {
        return letters.iter().take(n).map(|c| c.to_string()).collect();
    }
    letters
        .iter()
        .flat_map(|a| letters.iter().map(move |b| format!("{a}{b}")))
        .take(n)
        .collect()
}

/// What the typed letters choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Narrow {
    /// No label starts with them.
    None,
    /// Some labels start with them: wait for more.
    Many,
    /// This label.
    One(usize),
}

pub fn narrow(labels: &[String], typed: &str) -> Narrow {
    let typed = typed.to_ascii_lowercase();
    if let Some(i) = labels.iter().position(|l| *l == typed) {
        return Narrow::One(i);
    }
    if labels.iter().any(|l| l.starts_with(&typed)) {
        Narrow::Many
    } else {
        Narrow::None
    }
}

#[cfg(test)]
mod tests {
    use fterm_term::alacritty_terminal::index::{Column, Line, Point};

    use super::*;

    fn link(url: &str, text: &str, osc8: bool) -> Link {
        let p = Point::new(Line(0), Column(0));
        Link {
            start: p,
            end: p,
            url: url.into(),
            text: text.into(),
            osc8,
        }
    }

    #[test]
    fn a_web_link_opens_at_once() {
        let u = "https://example.com/a";
        assert_eq!(decide(&link(u, u, false)), Open::Now);
        assert_eq!(
            decide(&link("mailto:me@x.org", "mailto:me@x.org", false)),
            Open::Now
        );
        // OSC 8 that shows its own URL.
        assert_eq!(decide(&link(u, "example.com/a", true)), Open::Now);
        assert_eq!(
            decide(&link("https://x.org/", "https://x.org", true)),
            Open::Now
        );
    }

    #[test]
    fn a_hidden_url_or_a_file_asks_first() {
        assert_eq!(
            decide(&link("https://evil.example", "https://bank.example", true)),
            Open::Ask
        );
        assert_eq!(
            decide(&link("https://x.org/pr/42", "PR #42", true)),
            Open::Ask
        );
        assert_eq!(
            decide(&link(
                "file:///C:/tools/a.exe",
                "file:///C:/tools/a.exe",
                false
            )),
            Open::Ask
        );
    }

    #[test]
    fn labels_are_one_letter_when_they_fit_and_two_when_not() {
        assert_eq!(labels(3), ["a", "s", "d"]);
        assert_eq!(labels(0).len(), 0);
        let many = labels(30);
        assert_eq!(many.len(), 30);
        assert!(many.iter().all(|l| l.len() == 2), "{many:?}");
        // No label is the start of another, so a typed letter never waits for nothing.
        let unique: std::collections::HashSet<_> = many.iter().collect();
        assert_eq!(unique.len(), 30);
        assert_eq!(&many[..2], ["aa", "as"]);
    }

    #[test]
    fn typed_letters_narrow_the_labels() {
        let l = labels(30);
        assert_eq!(narrow(&l, ""), Narrow::Many);
        assert_eq!(narrow(&l, "a"), Narrow::Many);
        assert_eq!(narrow(&l, "as"), Narrow::One(1));
        assert_eq!(narrow(&l, "x"), Narrow::None);
        let few = labels(3);
        assert_eq!(narrow(&few, "d"), Narrow::One(2));
        assert_eq!(narrow(&few, "D"), Narrow::One(2), "any case");
    }

    #[test]
    fn other_schemes_do_not_open() {
        for url in [
            "ms-settings:privacy",
            "javascript:alert(1)",
            "vscode://x",
            "C:\\a.exe",
        ] {
            assert_eq!(decide(&link(url, "click", true)), Open::Refuse, "{url}");
        }
    }
}
