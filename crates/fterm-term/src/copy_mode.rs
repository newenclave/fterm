//! Copy mode: select and copy with the keyboard. It uses the vi mode of alacritty:
//! a cursor that can go up into the history, and the view follows it.
//! `CopyState` keeps what vi mode does not: the start of the selection, the search, and the last jump.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vi_mode::ViMotion;

use crate::select::StickySelection;

/// The most matches that are counted (the status shows "1000+").
pub const MAX_COUNT: usize = 1000;

/// How a jump goes on the line: `f` `F` `t` `T`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Jump {
    Forward,
    Backward,
    TillForward,
    TillBackward,
}

impl Jump {
    fn reversed(self) -> Self {
        match self {
            Self::Forward => Self::Backward,
            Self::Backward => Self::Forward,
            Self::TillForward => Self::TillBackward,
            Self::TillBackward => Self::TillForward,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Select the word at the cursor: `big` = up to spaces; `around` = with the spaces after it.
    SelectWord {
        big: bool,
        around: bool,
    },
    /// The cursor goes to the other end of the selection.
    SwapEnds,
    /// Copy the selection; leave copy mode, or stay (the selection goes away).
    Copy {
        stay: bool,
    },
    /// Copy the cursor line (or the lines of the selection).
    CopyLine {
        stay: bool,
    },
    /// Leave copy mode without copying.
    Exit,
    /// Open the search prompt (the app shows it); `forward` = `/`, else `?`.
    SearchStart {
        forward: bool,
    },
    /// The text of the prompt changed: go to the first match from the place where the search started.
    SearchSet(String),
    /// Enter in the prompt: keep the search and the place.
    SearchAccept,
    /// Esc in the prompt: back to the place and the search before.
    SearchCancel,
    /// `n`: the next match in the way of the search; `N` (`false`): the other way.
    NextMatch {
        same_way: bool,
    },
    /// Go to a char on the line.
    JumpTo(Jump, char),
    /// `;` (`same_way`) and `,`: the last jump again.
    RepeatJump {
        same_way: bool,
    },
    /// The mouse: the cursor goes to this cell.
    MoveTo(Point),
}

#[derive(Debug, PartialEq, Eq)]
pub enum CopyResult {
    /// Still in copy mode.
    Continue,
    /// The text to put into the clipboard. Copy mode is closed, unless the action said "stay".
    Copied(String),
    /// Copy mode is closed.
    Exited,
}

/// A line counted from the top of the history, so new output does not move it (when the history is not full).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Abs {
    line: i64,
    column: usize,
}

impl Abs {
    fn of<T>(term: &Term<T>, point: Point) -> Self {
        Self {
            line: i64::from(point.line.0) + term.history_size() as i64,
            column: point.column.0,
        }
    }

    fn point<T>(self, term: &Term<T>) -> Point {
        let line = (self.line - term.history_size() as i64) as i32;
        let line = Line(line)
            .max(term.topmost_line())
            .min(term.bottommost_line());
        Point::new(line, Column(self.column.min(term.columns() - 1)))
    }
}

/// A search: its pattern and the current match.
#[derive(Clone, Debug)]
struct Search {
    pattern: String,
    regex: RegexSearch,
    forward: bool,
    current: Option<(Abs, Abs)>,
}

/// The search prompt while the user types.
#[derive(Clone, Debug)]
struct Prompt {
    origin: Abs,
    forward: bool,
    before: Option<Search>,
}

/// What copy mode keeps for a pane.
#[derive(Clone, Debug, Default)]
pub struct CopyState {
    /// The start of the selection and its type.
    anchor: Option<(Abs, SelectionType)>,
    search: Option<Search>,
    prompt: Option<Prompt>,
    last_jump: Option<(Jump, char)>,
    /// The keys typed so far that wait for more (the app sets it; the status bar shows it).
    pub pending: String,
}

impl CopyState {
    /// `normal`, `visual`, `visual_line`, or `visual_block`.
    pub fn mode(&self) -> &'static str {
        match self.anchor.map(|(_, ty)| ty) {
            None => "normal",
            Some(SelectionType::Lines) => "visual_line",
            Some(SelectionType::Block) => "visual_block",
            Some(_) => "visual",
        }
    }

    /// The search prompt is open.
    pub fn prompting(&self) -> bool {
        self.prompt.is_some()
    }

    /// `/` or `?` while the prompt is open.
    pub fn prompt_forward(&self) -> bool {
        self.prompt.as_ref().is_some_and(|p| p.forward)
    }

    pub fn search_pattern(&self) -> Option<&str> {
        self.search.as_ref().map(|s| s.pattern.as_str())
    }

    /// The matches in the visible rows, for drawing, and which of them is the current one.
    pub fn visible_matches<T>(&mut self, term: &Term<T>) -> (Vec<Match>, Option<usize>) {
        let Some(search) = self.search.as_mut() else {
            return (Vec::new(), None);
        };
        let offset = term.grid().display_offset() as i32;
        let start = Point::new(Line(-offset), Column(0));
        let end = Point::new(
            Line(term.screen_lines() as i32 - 1 - offset),
            Column(term.columns() - 1),
        );
        let current = search.current.map(|(a, _)| a.point(term));
        let list: Vec<Match> =
            RegexIter::new(start, end, Direction::Right, term, &mut search.regex).collect();
        let at = current.and_then(|c| list.iter().position(|m| *m.start() == c));
        (list, at)
    }

    /// The number of the current match and how many there are (at most `MAX_COUNT`).
    pub fn match_count<T>(&mut self, term: &Term<T>) -> Option<(Option<usize>, usize)> {
        let search = self.search.as_mut()?;
        let current = search.current.map(|(a, _)| a.point(term));
        let start = Point::new(term.topmost_line(), Column(0));
        let end = Point::new(term.bottommost_line(), Column(term.columns() - 1));
        let mut index = None;
        let mut count = 0;
        for m in RegexIter::new(start, end, Direction::Right, term, &mut search.regex) {
            if Some(*m.start()) == current {
                index = Some(count + 1);
            }
            count += 1;
            if count >= MAX_COUNT {
                break;
            }
        }
        Some((index, count))
    }
}

pub fn is_active<T: EventListener>(term: &Term<T>) -> bool {
    term.mode().contains(TermMode::VI)
}

/// Starts copy mode. The cursor starts at the shell cursor. The last search stays (for `n`).
pub fn enter<T: EventListener>(
    term: &mut Term<T>,
    selection: &mut StickySelection,
    state: &mut CopyState,
) {
    if !is_active(term) {
        term.toggle_vi_mode();
    }
    state.anchor = None;
    state.prompt = None;
    state.pending.clear();
    selection.set(term, None);
}

/// The line of the cursor (from 1 at the top of the history) and the number of lines.
pub fn cursor_row<T>(term: &Term<T>) -> (i64, usize) {
    let abs = Abs::of(term, term.vi_mode_cursor.point);
    (abs.line + 1, term.total_lines())
}

/// The text of the line at `point` (a long line that wraps is one line).
pub fn line_text<T>(term: &Term<T>, point: Point) -> String {
    let start = term.line_search_left(point);
    let end = term.line_search_right(point);
    let text = term.bounds_to_string(
        Point::new(start.line, Column(0)),
        Point::new(end.line, Column(term.columns() - 1)),
    );
    text.trim_end().to_owned()
}

/// The word at `point` (words end at spaces and at the semantic chars, as `w` does).
pub fn word_at<T>(term: &Term<T>, point: Point) -> Option<String> {
    let (start, end) = word_bounds(term, point, false)?;
    Some(term.bounds_to_string(start, end))
}

/// The start and end of the word at `point`; `None` on a space.
fn word_bounds<T>(term: &Term<T>, point: Point, big: bool) -> Option<(Point, Point)> {
    let row = &term.grid()[point.line];
    let last = term.columns() - 1;
    let c = row[point.column].c;
    if c == ' ' || c == '\t' {
        return None;
    }
    if !big {
        return Some((
            term.semantic_search_left(point),
            term.semantic_search_right(point),
        ));
    }
    let blank = |col: usize| matches!(row[Column(col)].c, ' ' | '\t');
    let mut start = point.column.0;
    while start > 0 && !blank(start - 1) {
        start -= 1;
    }
    let mut end = point.column.0;
    while end < last && !blank(end + 1) {
        end += 1;
    }
    Some((
        Point::new(point.line, Column(start)),
        Point::new(point.line, Column(end)),
    ))
}

pub fn apply<T: EventListener>(
    term: &mut Term<T>,
    selection: &mut StickySelection,
    state: &mut CopyState,
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
        CopyAction::MoveTo(point) => {
            let line = point
                .line
                .max(term.topmost_line())
                .min(term.bottommost_line());
            let column = Column(point.column.0.min(term.columns() - 1));
            term.vi_goto_point(Point::new(line, column));
        }
        CopyAction::Select(ty) => {
            if state.anchor.is_some() {
                state.anchor = None;
            } else {
                state.anchor = Some((Abs::of(term, term.vi_mode_cursor.point), ty));
            }
        }
        CopyAction::SelectWord { big, around } => {
            let cursor = term.vi_mode_cursor.point;
            if let Some((start, mut end)) = word_bounds(term, cursor, big) {
                if around {
                    let row = &term.grid()[end.line];
                    while end.column.0 + 1 < term.columns()
                        && matches!(row[end.column + 1].c, ' ' | '\t')
                    {
                        end.column += 1;
                    }
                }
                state.anchor = Some((Abs::of(term, start), SelectionType::Simple));
                term.vi_goto_point(end);
            }
        }
        CopyAction::SwapEnds => {
            if let Some((anchor, ty)) = state.anchor {
                let cursor = term.vi_mode_cursor.point;
                state.anchor = Some((Abs::of(term, cursor), ty));
                let to = anchor.point(term);
                term.vi_goto_point(to);
            }
        }
        CopyAction::Copy { stay } => {
            let text = term.selection_to_string();
            return copied(term, selection, state, text, stay);
        }
        CopyAction::CopyLine { stay } => {
            let cursor = term.vi_mode_cursor.point;
            let (top, bottom) = match state.anchor {
                Some((anchor, _)) => {
                    let a = anchor.point(term);
                    (a.min(cursor), a.max(cursor))
                }
                None => (cursor, cursor),
            };
            let start = term.line_search_left(top);
            let end = term.line_search_right(bottom);
            let text = term.bounds_to_string(
                Point::new(start.line, Column(0)),
                Point::new(end.line, Column(term.columns() - 1)),
            );
            let text: Vec<&str> = text.lines().map(str::trim_end).collect();
            return copied(term, selection, state, Some(text.join("\n")), stay);
        }
        CopyAction::Exit => {
            leave(term, selection, state);
            return CopyResult::Exited;
        }
        CopyAction::SearchStart { forward } => {
            state.prompt = Some(Prompt {
                origin: Abs::of(term, term.vi_mode_cursor.point),
                forward,
                before: state.search.clone(),
            });
            return CopyResult::Continue;
        }
        CopyAction::SearchSet(pattern) => {
            let Some(prompt) = state.prompt.as_ref() else {
                return CopyResult::Continue;
            };
            let (origin, forward) = (prompt.origin.point(term), prompt.forward);
            if pattern.is_empty() {
                state.search = None;
                term.vi_goto_point(origin);
            } else if let Some(regex) = regex(&pattern) {
                state.search = Some(Search {
                    pattern,
                    regex,
                    forward,
                    current: None,
                });
                if !find(term, state, origin, forward) {
                    term.vi_goto_point(origin);
                }
            }
        }
        CopyAction::SearchAccept => {
            state.prompt = None;
            return CopyResult::Continue;
        }
        CopyAction::SearchCancel => {
            if let Some(prompt) = state.prompt.take() {
                state.search = prompt.before;
                let origin = prompt.origin.point(term);
                term.vi_goto_point(origin);
            }
        }
        CopyAction::NextMatch { same_way } => {
            if let Some(search) = state.search.as_ref() {
                let forward = search.forward == same_way;
                let from = search
                    .current
                    .map_or(term.vi_mode_cursor.point, |(a, _)| a.point(term));
                find(term, state, from, forward);
            }
        }
        CopyAction::JumpTo(kind, c) => {
            state.last_jump = Some((kind, c));
            jump(term, kind, c, false);
        }
        CopyAction::RepeatJump { same_way } => {
            if let Some((kind, c)) = state.last_jump {
                let kind = if same_way { kind } else { kind.reversed() };
                jump(term, kind, c, true);
            }
        }
    }
    sync_selection(term, selection, state);
    CopyResult::Continue
}

/// The selection goes from the anchor to the cursor (with both cells).
fn sync_selection<T: EventListener>(
    term: &mut Term<T>,
    selection: &mut StickySelection,
    state: &CopyState,
) {
    let Some((anchor, ty)) = state.anchor else {
        if selection.is_active() {
            selection.set(term, None);
        }
        return;
    };
    let mut s = Selection::new(ty, anchor.point(term), Side::Left);
    s.update(term.vi_mode_cursor.point, Side::Left);
    s.include_all();
    selection.set(term, Some(s));
}

fn copied<T: EventListener>(
    term: &mut Term<T>,
    selection: &mut StickySelection,
    state: &mut CopyState,
    text: Option<String>,
    stay: bool,
) -> CopyResult {
    if stay {
        state.anchor = None;
        selection.set(term, None);
    } else {
        leave(term, selection, state);
    }
    match text {
        Some(text) => CopyResult::Copied(text),
        None if stay => CopyResult::Continue,
        None => CopyResult::Exited,
    }
}

/// A regex of the pattern; a pattern that is not a good regex is searched as plain text.
fn regex(pattern: &str) -> Option<RegexSearch> {
    RegexSearch::new(pattern)
        .ok()
        .or_else(|| RegexSearch::new(&escape(pattern)).ok())
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Goes to the next match after `from` (or before it); it goes around the ends. `false` = no match.
fn find<T: EventListener>(
    term: &mut Term<T>,
    state: &mut CopyState,
    from: Point,
    forward: bool,
) -> bool {
    let Some(search) = state.search.as_mut() else {
        return false;
    };
    let (origin, direction) = if forward {
        (from.add(term, Boundary::None, 1), Direction::Right)
    } else {
        (from.sub(term, Boundary::None, 1), Direction::Left)
    };
    let side = if forward { Side::Left } else { Side::Right };
    let Some(found) = term.search_next(&mut search.regex, origin, direction, side, None) else {
        search.current = None;
        return false;
    };
    let (start, end) = (*found.start(), *found.end());
    search.current = Some((Abs::of(term, start), Abs::of(term, end)));
    term.vi_goto_point(start);
    true
}

/// `f` `F` `t` `T` on the cursor line. `again` (`;` `,`) does not stay on the char next to a `t` jump.
fn jump<T: EventListener>(term: &mut Term<T>, kind: Jump, c: char, again: bool) {
    let cursor = term.vi_mode_cursor.point;
    let needle = c.to_string();
    let last = term.columns() - 1;
    let from = match kind {
        Jump::TillForward if again && cursor.column.0 < last => {
            Point::new(cursor.line, cursor.column + 1)
        }
        Jump::TillBackward if again && cursor.column.0 > 0 => {
            Point::new(cursor.line, cursor.column - 1)
        }
        _ => cursor,
    };
    let found = match kind {
        Jump::Forward | Jump::TillForward => term.inline_search_right(from, &needle),
        Jump::Backward | Jump::TillBackward => term.inline_search_left(from, &needle),
    };
    let Ok(mut to) = found else {
        return;
    };
    match kind {
        Jump::TillForward => to = to.sub(term, Boundary::None, 1),
        Jump::TillBackward => to = to.add(term, Boundary::None, 1),
        _ => {}
    }
    term.vi_goto_point(to);
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

fn leave<T: EventListener>(
    term: &mut Term<T>,
    selection: &mut StickySelection,
    state: &mut CopyState,
) {
    if is_active(term) {
        term.toggle_vi_mode();
    }
    state.anchor = None;
    state.prompt = None;
    state.pending.clear();
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

    struct T {
        term: Term<VoidListener>,
        sel: StickySelection,
        state: CopyState,
    }

    impl T {
        fn act(&mut self, action: CopyAction) -> CopyResult {
            apply(&mut self.term, &mut self.sel, &mut self.state, action)
        }

        fn run(&mut self, actions: &[CopyAction]) -> CopyResult {
            let mut last = CopyResult::Continue;
            for action in actions {
                last = self.act(action.clone());
            }
            last
        }

        fn cursor(&self) -> (i32, usize) {
            let p = self.term.vi_mode_cursor.point;
            (p.line.0, p.column.0)
        }

        fn copy(&mut self) -> String {
            match self.act(CopyAction::Copy { stay: true }) {
                CopyResult::Copied(text) => text,
                other => panic!("nothing copied: {other:?}"),
            }
        }

        /// The cursor goes to the start of the screen line `line` (0 = the top of the screen).
        fn at(&mut self, line: i32, column: usize) {
            self.act(CopyAction::MoveTo(Point::new(Line(line), Column(column))));
        }
    }

    /// A terminal `columns` wide, 5 lines high, with this text, in copy mode.
    fn with_text(columns: usize, text: &str) -> T {
        let mut term = Term::new(Config::default(), &GridSize::new(columns, 5), VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut term, text.as_bytes());
        let mut t = T {
            term,
            sel: StickySelection::default(),
            state: CopyState::default(),
        };
        enter(&mut t.term, &mut t.sel, &mut t.state);
        t
    }

    /// A 20x5 terminal with `lines` lines of output: "line 1", "line 2", ...
    fn with_lines(lines: usize) -> T {
        let text: Vec<String> = (1..=lines).map(|i| format!("line {i}")).collect();
        with_text(20, &text.join("\r\n"))
    }

    #[test]
    fn enter_and_exit() {
        let mut t = with_lines(3);
        assert!(is_active(&t.term));
        assert_eq!(t.act(CopyAction::Exit), CopyResult::Exited);
        assert!(!is_active(&t.term));
    }

    #[test]
    fn moving_up_scrolls_into_the_history() {
        let mut t = with_lines(200);
        assert_eq!(t.term.grid().display_offset(), 0);
        for _ in 0..10 {
            t.act(CopyAction::Move(ViMotion::Up));
        }
        // The screen has 5 lines, so 10 lines up must scroll the view.
        assert!(t.term.grid().display_offset() >= 5);
    }

    #[test]
    fn top_goes_to_the_first_line_of_the_history() {
        let mut t = with_lines(200);
        t.act(CopyAction::Top);
        assert_eq!(t.term.grid().display_offset(), t.term.history_size());
        assert_eq!(t.term.vi_mode_cursor.point.line, t.term.topmost_line());
    }

    #[test]
    fn copy_the_whole_history_with_the_keyboard() {
        // The user's case: select text that went off the screen long ago.
        let mut t = with_lines(200);
        let result = t.run(&[
            CopyAction::Top,
            CopyAction::Select(SelectionType::Lines),
            CopyAction::Bottom,
            CopyAction::Copy { stay: false },
        ]);
        let CopyResult::Copied(text) = result else {
            panic!("nothing copied: {result:?}");
        };
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.first(), Some(&"line 1"));
        assert_eq!(lines.last(), Some(&"line 200"));
        assert_eq!(lines.len(), 200);
        // After copy: copy mode is closed, the view is at the bottom, the selection is gone.
        assert!(!is_active(&t.term));
        assert_eq!(t.term.grid().display_offset(), 0);
        assert!(t.term.selection.is_none() && !t.sel.is_active());
    }

    #[test]
    fn page_up_moves_a_screen_up() {
        let mut t = with_lines(200);
        let start = t.term.vi_mode_cursor.point.line;
        t.act(CopyAction::PageUp);
        assert_eq!(t.term.vi_mode_cursor.point.line, start - 5);
        t.act(CopyAction::HalfPageDown);
        assert_eq!(t.term.vi_mode_cursor.point.line, start - 3);
    }

    #[test]
    fn select_twice_stops_the_selection() {
        let mut t = with_lines(10);
        t.act(CopyAction::Select(SelectionType::Simple));
        assert!(t.sel.is_active());
        assert_eq!(t.state.mode(), "visual");
        t.act(CopyAction::Select(SelectionType::Simple));
        assert!(!t.sel.is_active() && t.term.selection.is_none());
        assert_eq!(t.state.mode(), "normal");
    }

    #[test]
    fn a_char_selection_grows_with_the_cursor() {
        // `v` then `l l` must select three chars (alacritty alone keeps an empty selection).
        let mut t = with_text(20, "hello world");
        t.at(0, 0);
        t.run(&[
            CopyAction::Select(SelectionType::Simple),
            CopyAction::Move(ViMotion::Right),
            CopyAction::Move(ViMotion::Right),
        ]);
        assert_eq!(t.copy(), "hel");
        assert!(is_active(&t.term), "copy and stay");
        assert_eq!(t.state.mode(), "normal", "the selection goes away");
    }

    #[test]
    fn selection_in_copy_mode_is_sticky() {
        // The selection goes through StickySelection, so new output does not remove it.
        let mut t = with_lines(10);
        t.run(&[
            CopyAction::Select(SelectionType::Lines),
            CopyAction::Move(ViMotion::Up),
        ]);
        Processor::<StdSyncHandler>::new().advance(&mut t.term, b"\x1b[2J");
        t.sel.sync(&mut t.term);
        assert!(t.term.selection.is_some());
    }

    #[test]
    fn exit_clears_the_selection_and_goes_to_the_bottom() {
        let mut t = with_lines(200);
        t.run(&[CopyAction::Top, CopyAction::Select(SelectionType::Simple)]);
        t.act(CopyAction::Exit);
        assert_eq!(t.term.grid().display_offset(), 0);
        assert!(t.term.selection.is_none() && !t.sel.is_active());
    }

    #[test]
    fn search_goes_to_the_first_match_while_typing() {
        let mut t = with_text(30, "alpha\r\nbeta error\r\ngamma\r\nan error again");
        t.at(0, 0);
        t.act(CopyAction::SearchStart { forward: true });
        assert!(t.state.prompting());
        t.act(CopyAction::SearchSet("err".into()));
        assert_eq!(t.cursor(), (1, 5));
        t.act(CopyAction::SearchSet("error a".into()));
        assert_eq!(
            t.cursor(),
            (3, 3),
            "from the start of the search, not from the last match"
        );
        t.act(CopyAction::SearchAccept);
        assert!(!t.state.prompting());
        assert_eq!(t.state.search_pattern(), Some("error a"));
    }

    #[test]
    fn next_and_previous_match_go_around() {
        let mut t = with_text(30, "one x\r\ntwo x\r\nthree x");
        t.at(0, 0);
        t.run(&[
            CopyAction::SearchStart { forward: true },
            CopyAction::SearchSet("x".into()),
            CopyAction::SearchAccept,
        ]);
        assert_eq!(t.cursor(), (0, 4));
        assert_eq!(t.state.match_count(&t.term), Some((Some(1), 3)));
        t.act(CopyAction::NextMatch { same_way: true });
        assert_eq!(t.cursor(), (1, 4));
        t.act(CopyAction::NextMatch { same_way: true });
        t.act(CopyAction::NextMatch { same_way: true });
        assert_eq!(t.cursor(), (0, 4), "after the last match comes the first");
        t.act(CopyAction::NextMatch { same_way: false });
        assert_eq!(t.cursor(), (2, 6));
        assert_eq!(t.state.match_count(&t.term), Some((Some(3), 3)));
    }

    #[test]
    fn a_backward_search_goes_up() {
        let mut t = with_text(30, "x one\r\nx two\r\nx three");
        t.at(2, 4);
        t.run(&[
            CopyAction::SearchStart { forward: false },
            CopyAction::SearchSet("x".into()),
        ]);
        assert_eq!(t.cursor(), (2, 0));
        t.act(CopyAction::NextMatch { same_way: true });
        assert_eq!(t.cursor(), (1, 0), "n goes the way of ?");
    }

    #[test]
    fn search_cancel_goes_back() {
        let mut t = with_text(30, "a\r\nb target\r\nc");
        t.at(0, 0);
        t.run(&[
            CopyAction::SearchStart { forward: true },
            CopyAction::SearchSet("target".into()),
        ]);
        assert_eq!(t.cursor(), (1, 2));
        t.act(CopyAction::SearchCancel);
        assert_eq!(t.cursor(), (0, 0));
        assert_eq!(t.state.search_pattern(), None, "the search from before");
    }

    #[test]
    fn smart_case_and_plain_text_for_a_bad_regex() {
        let mut t = with_text(30, "Error here\r\nerror there\r\nf(x");
        t.at(0, 5);
        t.run(&[
            CopyAction::SearchStart { forward: true },
            CopyAction::SearchSet("Error".into()),
        ]);
        assert_eq!(
            t.state.match_count(&t.term).unwrap().1,
            1,
            "a capital = exact case"
        );
        t.act(CopyAction::SearchSet("error".into()));
        assert_eq!(t.state.match_count(&t.term).unwrap().1, 2);
        t.act(CopyAction::SearchSet("f(".into()));
        assert_eq!(t.cursor(), (2, 0), "a bad regex is plain text");
    }

    #[test]
    fn a_search_extends_the_selection() {
        let mut t = with_text(30, "start here\r\nmiddle\r\nend mark");
        t.at(0, 0);
        t.run(&[
            CopyAction::Select(SelectionType::Simple),
            CopyAction::SearchStart { forward: true },
            CopyAction::SearchSet("mark".into()),
            CopyAction::SearchAccept,
        ]);
        assert_eq!(t.copy(), "start here\nmiddle\nend m");
    }

    #[test]
    fn visible_matches_for_drawing() {
        let mut t = with_text(30, "a x b x\r\nno\r\nx");
        t.at(0, 0);
        t.run(&[
            CopyAction::SearchStart { forward: true },
            CopyAction::SearchSet("x".into()),
        ]);
        let (list, current) = t.state.visible_matches(&t.term);
        assert_eq!(list.len(), 3);
        assert_eq!(current, Some(0));
        assert_eq!(*list[2].start(), Point::new(Line(2), Column(0)));
    }

    #[test]
    fn jumps_on_the_line_and_repeat() {
        let mut t = with_text(30, "a,b,c,d");
        t.at(0, 0);
        t.act(CopyAction::JumpTo(Jump::Forward, ','));
        assert_eq!(t.cursor(), (0, 1));
        t.act(CopyAction::RepeatJump { same_way: true });
        assert_eq!(t.cursor(), (0, 3));
        t.act(CopyAction::RepeatJump { same_way: false });
        assert_eq!(t.cursor(), (0, 1));
        t.act(CopyAction::JumpTo(Jump::TillForward, ','));
        assert_eq!(t.cursor(), (0, 2), "t stops before the char");
        t.act(CopyAction::RepeatJump { same_way: true });
        assert_eq!(t.cursor(), (0, 4), "; after t goes on to the next one");
        t.act(CopyAction::JumpTo(Jump::TillBackward, 'a'));
        assert_eq!(t.cursor(), (0, 1));
        t.act(CopyAction::JumpTo(Jump::Forward, 'z'));
        assert_eq!(t.cursor(), (0, 1), "no such char: stay");
    }

    #[test]
    fn swap_ends_of_the_selection() {
        let mut t = with_text(30, "0123456789");
        t.at(0, 2);
        t.run(&[
            CopyAction::Select(SelectionType::Simple),
            CopyAction::MoveTo(Point::new(Line(0), Column(5))),
            CopyAction::SwapEnds,
        ]);
        assert_eq!(t.cursor(), (0, 2));
        t.act(CopyAction::Move(ViMotion::Left));
        assert_eq!(t.copy(), "12345", "the other end moved");
    }

    #[test]
    fn select_a_word_inner_around_and_big() {
        let mut t = with_text(40, "run cargo-test now");
        t.at(0, 5);
        t.act(CopyAction::SelectWord {
            big: true,
            around: false,
        });
        assert_eq!(t.copy(), "cargo-test");
        t.at(0, 1);
        t.act(CopyAction::SelectWord {
            big: false,
            around: true,
        });
        assert_eq!(t.copy(), "run ");
        assert_eq!(
            word_at(&t.term, Point::new(Line(0), Column(15))).as_deref(),
            Some("now")
        );
        assert_eq!(word_at(&t.term, Point::new(Line(0), Column(3))), None);
    }

    #[test]
    fn copy_the_line_without_a_selection() {
        let mut t = with_text(30, "first line\r\nsecond line  \r\nthird");
        t.at(1, 3);
        assert_eq!(
            t.act(CopyAction::CopyLine { stay: true }),
            CopyResult::Copied("second line".into())
        );
        t.at(0, 0);
        t.run(&[
            CopyAction::Select(SelectionType::Simple),
            CopyAction::Move(ViMotion::Down),
        ]);
        assert_eq!(
            t.act(CopyAction::CopyLine { stay: false }),
            CopyResult::Copied("first line\nsecond line".into())
        );
        assert!(!is_active(&t.term));
    }

    #[test]
    fn line_text_and_the_row() {
        let t = with_lines(200);
        let (row, total) = cursor_row(&t.term);
        assert_eq!(total, t.term.total_lines());
        assert_eq!(
            row as usize,
            t.term.total_lines(),
            "the cursor is on the last line"
        );
        let p = t.term.vi_mode_cursor.point;
        assert_eq!(line_text(&t.term, p), "line 200");
    }

    #[test]
    fn the_anchor_stays_on_its_text_when_output_comes() {
        let mut t = with_lines(50);
        t.at(0, 0);
        t.act(CopyAction::Select(SelectionType::Lines));
        let first = line_text(&t.term, t.term.vi_mode_cursor.point);
        Processor::<StdSyncHandler>::new().advance(&mut t.term, b"\r\nmore\r\nmore");
        t.sel.sync(&mut t.term);
        t.act(CopyAction::Move(ViMotion::Up));
        t.act(CopyAction::Move(ViMotion::Down));
        let text = t.copy();
        assert_eq!(text.lines().next(), Some(first.as_str()));
    }
}
