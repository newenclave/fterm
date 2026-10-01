//! "Sticky" selection: only the user removes it, not the terminal.
//!
//! alacritty removes `term.selection` when an app erases or rewrites the selected lines.
//! Apps like Claude Code redraw their area all the time, so the selection drops while
//! you select. Here we keep our own copy and put it back.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Line;
use alacritty_terminal::selection::Selection;
use alacritty_terminal::term::Term;

#[derive(Default)]
pub struct StickySelection {
    /// The user's selection, as it was at the last `sync`.
    saved: Option<Selection>,
    /// `history_size()` at the last `sync`.
    history: usize,
    /// Width of the grid at the last `sync`.
    columns: usize,
}

impl StickySelection {
    /// The user starts or changes a selection (`None` = the user removes it).
    pub fn set<T: EventListener>(&mut self, term: &mut Term<T>, selection: Option<Selection>) {
        term.selection = selection.clone();
        self.saved = selection;
        self.history = term.history_size();
        self.columns = term.columns();
    }

    /// Call after the terminal got new output, before drawing or copying.
    /// It puts the selection back when the terminal removed it.
    pub fn sync<T: EventListener>(&mut self, term: &mut Term<T>) {
        let history = term.history_size();
        if term.columns() != self.columns {
            // The lines were reflowed, so the old selection is not right any more.
            term.selection = None;
            self.saved = None;
        } else if let Some(current) = &term.selection {
            // alacritty keeps it up to date (it moves with scrolling): save it.
            self.saved = Some(current.clone());
        } else if let Some(saved) = self.saved.take() {
            // The terminal removed it. Put it back, moved by the lines that went into the history.
            let moved = history.saturating_sub(self.history) as i32;
            let restored = if moved > 0 {
                let screen = Line(0)..Line(term.screen_lines() as i32);
                saved.rotate(term, &screen, moved)
            } else {
                Some(saved)
            };
            term.selection = restored.clone();
            self.saved = restored;
        }
        self.history = history;
        self.columns = term.columns();
    }

    pub fn is_active(&self) -> bool {
        self.saved.is_some()
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::{Column, Line, Point, Side};
    use alacritty_terminal::selection::SelectionType;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::size::GridSize;

    struct Fixture {
        term: Term<VoidListener>,
        parser: Processor<StdSyncHandler>,
        sticky: StickySelection,
    }

    impl Fixture {
        fn new(columns: usize, rows: usize) -> Self {
            Self {
                term: Term::new(
                    Config::default(),
                    &GridSize::new(columns, rows),
                    VoidListener,
                ),
                parser: Processor::new(),
                sticky: StickySelection::default(),
            }
        }

        /// New output from the app, then `sync` (like the app does before drawing).
        fn output(&mut self, bytes: &str) {
            self.parser.advance(&mut self.term, bytes.as_bytes());
            self.sticky.sync(&mut self.term);
        }

        /// The user selects a whole screen line (from column 0 to `last_column`).
        fn select_line(&mut self, line: i32, last_column: usize) {
            let mut selection = Selection::new(
                SelectionType::Simple,
                Point::new(Line(line), Column(0)),
                Side::Left,
            );
            selection.update(Point::new(Line(line), Column(last_column)), Side::Right);
            self.sticky.set(&mut self.term, Some(selection));
        }

        fn text(&self) -> Option<String> {
            self.term.selection_to_string()
        }

        fn selected_lines(&self) -> Option<(i32, i32)> {
            let range = self.term.selection.as_ref()?.to_range(&self.term)?;
            Some((range.start.line.0, range.end.line.0))
        }
    }

    #[test]
    fn set_shows_the_selection() {
        let mut f = Fixture::new(10, 3);
        f.output("aaa\r\nbbb");
        f.select_line(1, 2);
        assert_eq!(f.text().as_deref(), Some("bbb"));
    }

    #[test]
    fn selection_survives_when_the_app_rewrites_the_line() {
        let mut f = Fixture::new(10, 3);
        f.output("aaa\r\nbbb\r\nccc");
        f.select_line(1, 2);
        // The app moves to line 2, erases it, and writes new text. alacritty removes the selection here.
        f.output("\x1b[2;1H\x1b[2Kxyz");
        assert_eq!(f.selected_lines(), Some((1, 1)));
        assert_eq!(f.text().as_deref(), Some("xyz"));
    }

    #[test]
    fn selection_survives_a_full_screen_clear() {
        let mut f = Fixture::new(10, 3);
        f.output("aaa\r\nbbb");
        f.select_line(0, 2);
        f.output("\x1b[2J");
        assert!(f.term.selection.is_some());
    }

    #[test]
    fn selection_moves_up_with_new_lines() {
        let mut f = Fixture::new(10, 3);
        f.output("1\r\n2\r\n3");
        f.select_line(1, 0);
        // Two new lines: "2" goes up into the history. alacritty moves the selection itself.
        f.output("\r\n4\r\n5");
        assert_eq!(f.text().as_deref(), Some("2"));
    }

    #[test]
    fn restored_selection_follows_lines_that_went_into_the_history() {
        let mut f = Fixture::new(10, 3);
        f.output("1\r\n2\r\n3");
        f.select_line(2, 0); // "3"
        // In one batch: one new line ("3" moves to line 1), then the app erases line 1.
        f.output("\r\n4\x1b[2;1H\x1b[2K");
        assert_eq!(f.selected_lines(), Some((1, 1)));
    }

    #[test]
    fn user_can_remove_the_selection() {
        let mut f = Fixture::new(10, 3);
        f.output("aaa");
        f.select_line(0, 2);
        f.sticky.set(&mut f.term, None);
        assert!(!f.sticky.is_active());
        f.output("\x1b[2K");
        assert!(f.term.selection.is_none());
    }

    #[test]
    fn selection_drops_when_the_width_changes() {
        // Lines reflow when the width changes, so the old selection is not right any more.
        let mut f = Fixture::new(10, 3);
        f.output("aaa");
        f.select_line(0, 2);
        f.term.resize(GridSize::new(20, 3));
        f.sticky.sync(&mut f.term);
        assert!(f.term.selection.is_none());
        assert!(!f.sticky.is_active());
    }
}
