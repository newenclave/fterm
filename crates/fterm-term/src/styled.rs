//! Text with its colors and styles, for the API (`get_text` with `styled`): what the user sees,
//! so an agent can tell an error in red from a normal line.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{NamedColor, Rgb};

use crate::colors::{Palette, cell_colors};
use crate::input::total_lines;

/// The style of a run of text. The colors are final: bold, dim, inverse, and hidden are in them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub dim: bool,
}

/// Text with one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub style: Style,
}

/// The colors of text with no color of its own.
pub fn default_colors<T>(term: &Term<T>, palette: &Palette) -> (Rgb, Rgb) {
    let overrides = term.colors();
    (
        palette.get(NamedColor::Foreground as usize, overrides),
        palette.get(NamedColor::Background as usize, overrides),
    )
}

/// The lines `from..to` (lines from the top of the history) as runs of one style. Like
/// `lines_text`: wrapped lines are one line, and spaces at the ends (with no color) are gone.
pub fn styled_lines<T>(term: &Term<T>, from: usize, to: usize, palette: &Palette) -> Vec<Vec<Run>> {
    let grid = term.grid();
    let history = grid.history_size() as i32;
    let columns = grid.columns();
    let overrides = term.colors();
    let (_, default_bg) = default_colors(term, palette);
    let to = to.min(total_lines(term));
    let mut lines = Vec::new();
    // The cells of one line: (its text, its style).
    let mut cells: Vec<(String, Style)> = Vec::new();
    for abs in from..to {
        let row = &grid[Line(abs as i32 - history)];
        for c in 0..columns {
            let cell = &row[Column(c)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let (fg, bg) = cell_colors(cell.fg, cell.bg, cell.flags, palette, overrides);
            let style = Style {
                fg,
                bg,
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                strike: cell.flags.contains(Flags::STRIKEOUT),
                dim: cell.flags.contains(Flags::DIM),
            };
            let mut text = cell.c.to_string();
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra);
            }
            cells.push((text, style));
        }
        // A line that the screen wrapped goes on in the next row.
        let wrapped = row[Column(columns - 1)].flags.contains(Flags::WRAPLINE);
        if !wrapped || abs + 1 == to {
            // Spaces at the end that show nothing.
            while cells.last().is_some_and(|(text, style)| {
                text.trim().is_empty()
                    && style.bg == default_bg
                    && !style.underline
                    && !style.strike
            }) {
                cells.pop();
            }
            lines.push(runs(std::mem::take(&mut cells)));
        }
    }
    lines
}

/// Cells next to each other with the same style become one run.
fn runs(cells: Vec<(String, Style)>) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (text, style) in cells {
        match runs.last_mut() {
            Some(run) if run.style == style => run.text.push_str(&text),
            _ => runs.push(Run { text, style }),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::size::GridSize;

    fn term(columns: usize, rows: usize, bytes: &str) -> Term<VoidListener> {
        let mut term = Term::new(
            Config::default(),
            &GridSize::new(columns, rows),
            VoidListener,
        );
        Processor::<StdSyncHandler>::new().advance(&mut term, bytes.as_bytes());
        term
    }

    fn texts(lines: &[Vec<Run>]) -> Vec<Vec<&str>> {
        lines
            .iter()
            .map(|l| l.iter().map(|r| r.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn runs_of_one_style() {
        let p = Palette::default();
        let t = term(
            20,
            3,
            "ok \x1b[1;31merror\x1b[0m done\r\n\x1b[4;3mlink\x1b[0m",
        );
        let lines = styled_lines(&t, 0, total_lines(&t), &p);
        assert_eq!(
            texts(&lines),
            vec![vec!["ok ", "error", " done"], vec!["link"], vec![]]
        );
        let (fg, bg) = default_colors(&t, &p);
        let plain = lines[0][0].style;
        assert_eq!((plain.fg, plain.bg, plain.bold), (fg, bg, false));
        let error = lines[0][1].style;
        assert!(error.bold);
        // Bold red is drawn bright red.
        assert_eq!(error.fg, p.get(NamedColor::BrightRed as usize, t.colors()));
        assert_eq!(lines[0][2].style, plain);
        let link = lines[1][0].style;
        assert!(link.underline && link.italic && !link.bold);
    }

    #[test]
    fn colors_are_final() {
        let p = Palette::default();
        let t = term(
            20,
            2,
            "\x1b[7mrev\x1b[0m \x1b[38;2;1;2;3;48;5;196mrgb\x1b[0m",
        );
        let lines = styled_lines(&t, 0, 1, &p);
        let (fg, bg) = default_colors(&t, &p);
        let rev = lines[0][0].style;
        assert_eq!((rev.fg, rev.bg), (bg, fg), "inverse is in the colors");
        let rgb = lines[0][2].style;
        assert_eq!(rgb.fg, Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(rgb.bg, Rgb { r: 255, g: 0, b: 0 });
    }

    #[test]
    fn a_wrapped_line_and_colored_spaces() {
        let p = Palette::default();
        // A bar with a background at the end stays; plain spaces at the end go.
        let t = term(10, 3, "abcdefghijkl\r\nx \x1b[44m  \x1b[0m   ");
        let lines = styled_lines(&t, 0, total_lines(&t), &p);
        assert_eq!(
            texts(&lines)[0],
            vec!["abcdefghijkl"],
            "a wrapped line is one line"
        );
        assert_eq!(texts(&lines)[1], vec!["x ", "  "]);
        assert_eq!(
            lines[1][1].style.bg,
            p.get(NamedColor::Blue as usize, t.colors())
        );
    }
}
