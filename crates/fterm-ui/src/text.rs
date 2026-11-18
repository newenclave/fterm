//! Text helpers: widths in cells, cutting, wrapping, and cleaning text from outside.

use unicode_width::UnicodeWidthChar;

/// The width of a text in cells.
pub fn cells(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// The first `width` cells of a text.
pub fn fit(text: &str, width: usize) -> String {
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

/// Cuts a line into pieces of at most `width` cells (no word wrap).
pub fn cut(line: &str, width: usize) -> Vec<String> {
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

/// Text from the clipboard for an input with many lines: one kind of new line, no control chars (a tab is a
/// space), so no escape sequence reaches the screen.
pub fn clean(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| *c == '\n' || !c.is_control())
        .collect()
}

/// Text for an input with one line: new lines and tabs are spaces, other control chars go, no spaces at the ends.
pub fn one_line(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let line: String = text
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    line.trim().to_owned()
}

/// A name from outside (a client, a process) for a box that the user trusts: one line, no chars that hide or turn
/// text (zero-width, bidi, line and paragraph separators), not too many chars, at most `cells` cells.
pub fn safe_name(text: &str, cells: usize) -> String {
    let hidden = |c: char| {
        matches!(c,
            '\u{00ad}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
    };
    // A limit of chars before the cells: zero-width chars (combining marks) take no cell.
    let line: String = one_line(text)
        .chars()
        .filter(|c| !hidden(*c))
        .take(4 * cells)
        .collect();
    fit(line.trim(), cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_safe_name_hides_nothing_and_is_short() {
        assert_eq!(safe_name("claude", 32), "claude");
        assert_eq!(
            safe_name("a\u{202e}exe.cod", 32),
            "aexe.cod",
            "no bidi override"
        );
        assert_eq!(
            safe_name("cl\u{200b}aude\u{2028}x", 32),
            "claudex",
            "no zero-width, no separator"
        );
        assert_eq!(safe_name("\u{feff}a\u{2066}b\u{2069}", 32), "ab");
        let long = format!("a{}", "\u{0301}".repeat(1_000_000));
        assert!(
            safe_name(&long, 32).chars().count() <= 4 * 32,
            "combining marks: a limit of chars too"
        );
        assert_eq!(cells(&safe_name(&"x".repeat(100), 32)), 32);
    }

    #[test]
    fn widths_and_fitting() {
        assert_eq!(cells("ab中"), 4);
        assert_eq!(
            fit("ab中d", 3),
            "ab",
            "a wide char that does not fit stays out"
        );
        assert_eq!(fit("ab中d", 4), "ab中");
        assert_eq!(cut("abcdefg", 3), ["abc", "def", "g"]);
        assert_eq!(cut("", 3), [""]);
    }

    #[test]
    fn wrapping_at_word_ends() {
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(
            wrap("abcdefghij", 4),
            ["abcd", "efgh", "ij"],
            "a long word is cut"
        );
    }

    #[test]
    fn cleaning_text_from_outside() {
        assert_eq!(clean("a\r\nb\rc\td\u{1b}[31m\u{9b}e"), "a\nb\nc d[31me");
        assert_eq!(one_line("  a@b.c\r\n"), "a@b.c");
        assert_eq!(one_line("x\ny\tz\u{7}"), "x y z");
    }
}
