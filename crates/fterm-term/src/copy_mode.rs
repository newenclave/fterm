//! Copy mode: select and copy with the keyboard. It uses the vi mode of alacritty:
//! a cursor that can go up into the history, and the view follows it.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vi_mode::ViMotion;

use crate::select::StickySelection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyAction {
    Move(ViMotion),
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    /// The first line of the history.
    Top,
    /// The last line of the screen.
    Bottom,
    /// Start (or stop) a selection of this type at the cursor.
    Select(SelectionType),
    /// Copy the selection and leave copy mode.
    Copy,
    /// Leave copy mode without copying.
    Exit,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CopyResult {
    /// Still in copy mode.
    Continue,
    /// The text to put into the clipboard. Copy mode is closed.
    Copied(String),
    /// Copy mode is closed.
    Exited,
}

pub fn is_active<T: EventListener>(term: &Term<T>) -> bool {
    term.mode().contains(TermMode::VI)
}

/// Starts copy mode. The cursor starts at the shell cursor.
pub fn enter<T: EventListener>(term: &mut Term<T>, selection: &mut StickySelection) {
    if !is_active(term) {
        term.toggle_vi_mode();
    }
    selection.set(term, None);
}

pub fn apply<T: EventListener>(
    term: &mut Term<T>,
    selection: &mut StickySelection,
    action: CopyAction,
) -> CopyResult {
    let lines = term.screen_lines() as i32;
    match action {
        CopyAction::Move(motion) => {
            term.vi_motion(motion);
            let point = term.vi_mode_cursor.point;
            term.scroll_to_point(point);
        }
        CopyAction::PageUp => move_lines(term, -lines),
        CopyAction::PageDown => move_lines(term, lines),
        CopyAction::HalfPageUp => move_lines(term, -lines / 2),
        CopyAction::HalfPageDown => move_lines(term, lines / 2),
        CopyAction::Top => goto_line(term, term.topmost_line()),
        CopyAction::Bottom => goto_line(term, term.bottommost_line()),
        CopyAction::Select(ty) => {
            if selection.is_active() {
                selection.set(term, None);
            } else {
                let start = Selection::new(ty, term.vi_mode_cursor.point, Side::Left);
                selection.set(term, Some(start));
            }
            return CopyResult::Continue;
        }
        CopyAction::Copy => {
            let text = term.selection_to_string();
            leave(term, selection);
            return text.map_or(CopyResult::Exited, CopyResult::Copied);
        }
        CopyAction::Exit => {
            leave(term, selection);
            return CopyResult::Exited;
        }
    }
    // vi motions change `term.selection` directly. Keep the sticky copy the same.
    if selection.is_active() {
        let current = term.selection.clone();
        selection.set(term, current);
    }
    CopyResult::Continue
}

fn move_lines<T: EventListener>(term: &mut Term<T>, delta: i32) {
    goto_line(term, term.vi_mode_cursor.point.line + delta);
}

/// Moves the cursor to `line` (kept inside the history and the screen). The view follows.
fn goto_line<T: EventListener>(term: &mut Term<T>, line: Line) {
    let line = line.max(term.topmost_line()).min(term.bottommost_line());
    let point = Point::new(line, term.vi_mode_cursor.point.column);
    term.vi_goto_point(point);
}

fn leave<T: EventListener>(term: &mut Term<T>, selection: &mut StickySelection) {
    if is_active(term) {
        term.toggle_vi_mode();
    }
    selection.set(term, None);
    term.scroll_display(Scroll::Bottom);
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::size::GridSize;

    /// A 20x5 terminal with `lines` lines of output: "line 1", "line 2", ...
    fn term_with_lines(lines: usize) -> (Term<VoidListener>, StickySelection) {
        let mut term = Term::new(Config::default(), &GridSize::new(20, 5), VoidListener);
        let text: Vec<String> = (1..=lines).map(|i| format!("line {i}")).collect();
        Processor::<StdSyncHandler>::new().advance(&mut term, text.join("\r\n").as_bytes());
        (term, StickySelection::default())
    }

    fn run(
        term: &mut Term<VoidListener>,
        sel: &mut StickySelection,
        actions: &[CopyAction],
    ) -> CopyResult {
        let mut last = CopyResult::Continue;
        for &action in actions {
            last = apply(term, sel, action);
        }
        last
    }

    #[test]
    fn enter_and_exit() {
        let (mut term, mut sel) = term_with_lines(3);
        assert!(!is_active(&term));
        enter(&mut term, &mut sel);
        assert!(is_active(&term));
        assert_eq!(
            apply(&mut term, &mut sel, CopyAction::Exit),
            CopyResult::Exited
        );
        assert!(!is_active(&term));
    }

    #[test]
    fn moving_up_scrolls_into_the_history() {
        let (mut term, mut sel) = term_with_lines(200);
        enter(&mut term, &mut sel);
        assert_eq!(term.grid().display_offset(), 0);
        for _ in 0..10 {
            apply(&mut term, &mut sel, CopyAction::Move(ViMotion::Up));
        }
        // The screen has 5 lines, so 10 lines up must scroll the view.
        assert!(
            term.grid().display_offset() >= 5,
            "{}",
            term.grid().display_offset()
        );
    }

    #[test]
    fn top_goes_to_the_first_line_of_the_history() {
        let (mut term, mut sel) = term_with_lines(200);
        enter(&mut term, &mut sel);
        apply(&mut term, &mut sel, CopyAction::Top);
        assert_eq!(term.grid().display_offset(), term.history_size());
        assert_eq!(term.vi_mode_cursor.point.line, term.topmost_line());
    }

    #[test]
    fn copy_the_whole_history_with_the_keyboard() {
        // The user's case: select text that went off the screen long ago.
        let (mut term, mut sel) = term_with_lines(200);
        enter(&mut term, &mut sel);
        let result = run(
            &mut term,
            &mut sel,
            &[
                CopyAction::Top,
                CopyAction::Select(SelectionType::Lines),
                CopyAction::Bottom,
                CopyAction::Copy,
            ],
        );
        let CopyResult::Copied(text) = result else {
            panic!("nothing copied: {result:?}");
        };
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.first(), Some(&"line 1"));
        assert_eq!(lines.last(), Some(&"line 200"));
        assert_eq!(lines.len(), 200);
        // After copy: copy mode is closed, the view is at the bottom, the selection is gone.
        assert!(!is_active(&term));
        assert_eq!(term.grid().display_offset(), 0);
        assert!(term.selection.is_none() && !sel.is_active());
    }

    #[test]
    fn page_up_moves_a_screen_up() {
        let (mut term, mut sel) = term_with_lines(200);
        enter(&mut term, &mut sel);
        let start = term.vi_mode_cursor.point.line;
        apply(&mut term, &mut sel, CopyAction::PageUp);
        assert_eq!(term.vi_mode_cursor.point.line, start - 5);
        apply(&mut term, &mut sel, CopyAction::HalfPageDown);
        assert_eq!(term.vi_mode_cursor.point.line, start - 3);
    }

    #[test]
    fn select_twice_stops_the_selection() {
        let (mut term, mut sel) = term_with_lines(10);
        enter(&mut term, &mut sel);
        apply(
            &mut term,
            &mut sel,
            CopyAction::Select(SelectionType::Simple),
        );
        assert!(sel.is_active());
        apply(
            &mut term,
            &mut sel,
            CopyAction::Select(SelectionType::Simple),
        );
        assert!(!sel.is_active() && term.selection.is_none());
    }

    #[test]
    fn selection_in_copy_mode_is_sticky() {
        // The selection goes through StickySelection, so new output does not remove it.
        let (mut term, mut sel) = term_with_lines(10);
        enter(&mut term, &mut sel);
        run(
            &mut term,
            &mut sel,
            &[
                CopyAction::Select(SelectionType::Lines),
                CopyAction::Move(ViMotion::Up),
            ],
        );
        Processor::<StdSyncHandler>::new().advance(&mut term, b"\x1b[2J");
        sel.sync(&mut term);
        assert!(term.selection.is_some());
    }

    #[test]
    fn exit_clears_the_selection_and_goes_to_the_bottom() {
        let (mut term, mut sel) = term_with_lines(200);
        enter(&mut term, &mut sel);
        run(
            &mut term,
            &mut sel,
            &[CopyAction::Top, CopyAction::Select(SelectionType::Simple)],
        );
        apply(&mut term, &mut sel, CopyAction::Exit);
        assert_eq!(term.grid().display_offset(), 0);
        assert!(term.selection.is_none() && !sel.is_active());
    }
}
