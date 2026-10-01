//! Links: find a URL under a cell, also when the URL is wrapped to the next line.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags;

/// URL schemes that we open.
const SCHEMES: [&str; 5] = ["https://", "http://", "file://", "ftp://", "mailto:"];

/// The URL at `point`, or `None` when there is no URL there.
pub fn url_at<T: EventListener>(term: &Term<T>, point: Point) -> Option<String> {
    let grid = term.grid();
    let last = Column(term.columns() - 1);
    let wraps = |line: Line| grid[line][last].flags.contains(Flags::WRAPLINE);

    // The whole logical line: lines that were wrapped by the screen belong together.
    let mut start = point.line;
    while start > term.topmost_line() && wraps(start - 1) {
        start -= 1;
    }
    let mut end = point.line;
    while end < term.bottommost_line() && wraps(end) {
        end += 1;
    }

    let mut chars = Vec::new();
    let mut target = None;
    let mut line = start;
    while line <= end {
        for col in 0..term.columns() {
            let cell = &grid[line][Column(col)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            if line == point.line && col == point.column.0 {
                target = Some(chars.len());
            }
            chars.push(cell.c);
        }
        line += 1;
    }
    let target = target?;

    // The run of URL chars around the target.
    let is_url_char = |c: char| !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | '>' | '`');
    if !is_url_char(chars[target]) {
        return None;
    }
    let mut a = target;
    while a > 0 && is_url_char(chars[a - 1]) {
        a -= 1;
    }
    let mut b = target + 1;
    while b < chars.len() && is_url_char(chars[b]) {
        b += 1;
    }
    let run: String = chars[a..b].iter().collect();

    // The URL starts at a scheme. The target must be inside it.
    let (scheme_at, _) = SCHEMES
        .iter()
        .filter_map(|scheme| run.find(scheme).map(|at| (at, scheme)))
        .min_by_key(|(at, _)| *at)?;
    let mut url: Vec<char> = run[scheme_at..].chars().collect();
    let first = a + run[..scheme_at].chars().count();
    // Punctuation at the end and a `)` that was not opened are not part of the URL.
    while let Some(&c) = url.last() {
        let open = url.iter().filter(|&&x| x == '(').count();
        let close = url.iter().filter(|&&x| x == ')').count();
        if matches!(c, '.' | ',' | ';' | ':' | '!' | '?') || (c == ')' && close > open) {
            url.pop();
        } else {
            break;
        }
    }
    (target >= first && target < first + url.len()).then(|| url.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::size::GridSize;

    fn term_with(text: &str, columns: usize) -> Term<VoidListener> {
        let mut term = Term::new(Config::default(), &GridSize::new(columns, 4), VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut term, text.as_bytes());
        term
    }

    fn at(term: &Term<VoidListener>, line: i32, col: usize) -> Option<String> {
        url_at(term, Point::new(Line(line), Column(col)))
    }

    #[test]
    fn url_in_the_middle_of_a_line() {
        let term = term_with("open https://example.com/a?b=1 now", 60);
        assert_eq!(
            at(&term, 0, 10).as_deref(),
            Some("https://example.com/a?b=1")
        );
        // The first and the last char of the URL work too.
        assert_eq!(
            at(&term, 0, 5).as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(
            at(&term, 0, 29).as_deref(),
            Some("https://example.com/a?b=1")
        );
    }

    #[test]
    fn no_url_outside() {
        let term = term_with("open https://example.com now", 60);
        assert_eq!(at(&term, 0, 1), None);
        assert_eq!(at(&term, 0, 26), None);
        let term = term_with("just text, no links here", 60);
        assert_eq!(at(&term, 0, 5), None);
    }

    #[test]
    fn punctuation_at_the_end_is_not_part_of_the_url() {
        let term = term_with("see https://example.com/page.", 60);
        assert_eq!(at(&term, 0, 8).as_deref(), Some("https://example.com/page"));
        let term = term_with("(see https://a.org/x)", 60);
        assert_eq!(at(&term, 0, 8).as_deref(), Some("https://a.org/x"));
    }

    #[test]
    fn wrapped_url_is_one_url() {
        // 20 columns: the URL goes on to the next line.
        let term = term_with("go https://example.com/long/path ok", 20);
        let url = Some("https://example.com/long/path");
        assert_eq!(at(&term, 0, 5).as_deref(), url);
        assert_eq!(at(&term, 1, 2).as_deref(), url);
    }

    #[test]
    fn other_schemes() {
        let term = term_with("file:///C:/work/a.txt and mailto:me@x.org", 60);
        assert_eq!(at(&term, 0, 3).as_deref(), Some("file:///C:/work/a.txt"));
        assert_eq!(at(&term, 0, 30).as_deref(), Some("mailto:me@x.org"));
    }
}
