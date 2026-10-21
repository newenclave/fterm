//! Links: URLs in the text, also when a URL is wrapped to the next line, and OSC 8 links (text with a hidden
//! URL). Each link knows its cells, so the window can underline it or put a label on it.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags;

/// URL schemes that we find in the text.
const SCHEMES: [&str; 5] = ["https://", "http://", "file://", "ftp://", "mailto:"];

/// A link on the screen or in the history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The first and the last cell (in history coordinates, like `Point`s of the grid).
    pub start: Point,
    pub end: Point,
    pub url: String,
    /// The text that the user sees (for a text URL the same as `url`).
    pub text: String,
    /// An OSC 8 link: the URL is hidden behind the text.
    pub osc8: bool,
}

impl Link {
    pub fn contains(&self, point: Point) -> bool {
        self.start <= point && point <= self.end
    }
}

/// The URL at `point`, or `None` when there is no URL there.
pub fn url_at<T: EventListener>(term: &Term<T>, point: Point) -> Option<String> {
    link_at(term, point).map(|link| link.url)
}

/// The link at `point` (an OSC 8 link, or a URL in the text).
pub fn link_at<T: EventListener>(term: &Term<T>, point: Point) -> Option<Link> {
    let (start, end) = logical_line(term, point.line);
    line_links(term, start, end)
        .into_iter()
        .find(|link| link.contains(point))
}

/// All links on the lines `top..=bottom` (a link that starts above `top` but goes on into it is there too).
pub fn links_in<T: EventListener>(term: &Term<T>, top: Line, bottom: Line) -> Vec<Link> {
    let mut out = Vec::new();
    let mut line = top.max(term.topmost_line());
    let bottom = bottom.min(term.bottommost_line());
    while line <= bottom {
        let (start, end) = logical_line(term, line);
        out.extend(line_links(term, start, end));
        line = end + 1;
    }
    out
}

/// The first and the last row of the logical line at `line`: rows that the screen wrapped belong together.
fn logical_line<T: EventListener>(term: &Term<T>, line: Line) -> (Line, Line) {
    let grid = term.grid();
    let last = Column(term.columns() - 1);
    let wraps = |line: Line| grid[line][last].flags.contains(Flags::WRAPLINE);
    let mut start = line;
    while start > term.topmost_line() && wraps(start - 1) {
        start -= 1;
    }
    let mut end = line;
    while end < term.bottommost_line() && wraps(end) {
        end += 1;
    }
    (start, end)
}

/// The id and the URL of an OSC 8 link of a cell.
type Osc8 = (String, String);

/// The links of one logical line: the OSC 8 links first, then the URLs in the text that are not in one.
fn line_links<T: EventListener>(term: &Term<T>, start: Line, end: Line) -> Vec<Link> {
    let grid = term.grid();
    // The chars of the line, with their cells and their OSC 8 link.
    let mut cells: Vec<(char, Point, Option<Osc8>)> = Vec::new();
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
            // The mark of a picture is a hyperlink too, but not a link to open.
            let link = cell
                .hyperlink()
                .filter(|h| crate::osc::picture_id(h.uri()).is_none())
                .map(|h| (h.id().to_owned(), h.uri().to_owned()));
            cells.push((cell.c, Point::new(line, Column(col)), link));
        }
        line += 1;
    }

    let mut links = Vec::new();
    // OSC 8: runs of cells with the same link.
    let mut i = 0;
    while i < cells.len() {
        let Some(link) = cells[i].2.clone() else {
            i += 1;
            continue;
        };
        let mut j = i + 1;
        while j < cells.len() && cells[j].2.as_ref() == Some(&link) {
            j += 1;
        }
        let text: String = cells[i..j].iter().map(|c| c.0).collect();
        links.push(Link {
            start: cells[i].1,
            end: cells[j - 1].1,
            url: link.1,
            text: text.trim().to_owned(),
            osc8: true,
        });
        i = j;
    }

    // URLs in the text: runs of URL chars that hold a scheme.
    let is_url_char = |c: char| !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | '>' | '`');
    let mut a = 0;
    while a < cells.len() {
        if !is_url_char(cells[a].0) {
            a += 1;
            continue;
        }
        let mut b = a;
        while b < cells.len() && is_url_char(cells[b].0) {
            b += 1;
        }
        let run: String = cells[a..b].iter().map(|c| c.0).collect();
        let mut rest_from = 0;
        // A run can hold more than one URL only in odd text; take each scheme start once.
        while let Some((at, _)) = SCHEMES
            .iter()
            .filter_map(|scheme| {
                run[rest_from..]
                    .find(scheme)
                    .map(|at| (rest_from + at, scheme))
            })
            .min_by_key(|(at, _)| *at)
        {
            let mut url: Vec<char> = run[at..].chars().collect();
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
            let first = a + run[..at].chars().count();
            let last = first + url.len().saturating_sub(1);
            let inside_osc8 = cells[first..=last.min(cells.len() - 1)]
                .iter()
                .any(|c| c.2.is_some());
            if !url.is_empty() && !inside_osc8 {
                let url: String = url.into_iter().collect();
                links.push(Link {
                    start: cells[first].1,
                    end: cells[last].1,
                    text: url.clone(),
                    url,
                    osc8: false,
                });
            }
            rest_from = run.len();
        }
        a = b;
    }
    links.sort_by_key(|l| l.start);
    links
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
    fn the_mark_of_a_picture_is_not_a_link() {
        let term = term_with(
            &format!(
                "\x1b]8;id=p;{}\x1b\\X\x1b]8;;\x1b\\ text",
                crate::osc::picture_uri(3)
            ),
            60,
        );
        assert_eq!(at(&term, 0, 0), None);
        assert!(links_in(&term, Line(0), Line(0)).is_empty());
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

    fn p(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    fn all(term: &Term<VoidListener>) -> Vec<Link> {
        links_in(term, Line(0), Line(term.screen_lines() as i32 - 1))
    }

    #[test]
    fn links_with_their_cells() {
        let term = term_with("a https://x.org/1 b https://y.org c", 60);
        let links = all(&term);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].url, "https://x.org/1");
        assert_eq!((links[0].start, links[0].end), (p(0, 2), p(0, 16)));
        assert_eq!(links[0].text, links[0].url);
        assert!(!links[0].osc8);
        assert_eq!(links[1].url, "https://y.org");
        assert_eq!((links[1].start, links[1].end), (p(0, 20), p(0, 32)));
    }

    #[test]
    fn a_wrapped_link_spans_two_rows() {
        let term = term_with("go https://example.com/long/path ok", 20);
        let links = all(&term);
        assert_eq!(links.len(), 1);
        assert_eq!((links[0].start, links[0].end), (p(0, 3), p(1, 11)));
    }

    #[test]
    fn osc8_links_have_their_text_and_url() {
        let term = term_with(
            "see \x1b]8;;https://example.com/pr/42\x1b\\PR #42\x1b]8;;\x1b\\ and https://z.org",
            60,
        );
        let links = all(&term);
        assert_eq!(links.len(), 2, "{links:?}");
        assert!(links[0].osc8);
        assert_eq!(links[0].url, "https://example.com/pr/42");
        assert_eq!(links[0].text, "PR #42");
        assert_eq!((links[0].start, links[0].end), (p(0, 4), p(0, 9)));
        assert!(!links[1].osc8);
        assert_eq!(links[1].url, "https://z.org");
        // The cell under the mouse.
        let at = link_at(&term, p(0, 6)).unwrap();
        assert_eq!(at.url, "https://example.com/pr/42");
        assert_eq!(link_at(&term, p(0, 2)), None);
        assert_eq!(link_at(&term, p(0, 18)).unwrap().url, "https://z.org");
    }

    #[test]
    fn other_schemes() {
        let term = term_with("file:///C:/work/a.txt and mailto:me@x.org", 60);
        assert_eq!(at(&term, 0, 3).as_deref(), Some("file:///C:/work/a.txt"));
        assert_eq!(at(&term, 0, 30).as_deref(), Some("mailto:me@x.org"));
    }
}
