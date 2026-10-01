//! The tree of panes in a tab: a pane, or a split of two smaller trees.

use crate::mux::PaneId;

/// A rectangle in window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Where the second pane of a split goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Side by side: the second pane is on the right.
    Right,
    /// One above the other: the second pane is below.
    Down,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    Pane(PaneId),
    Split {
        direction: Direction,
        /// The part of the space for `first` (0.0..1.0).
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

impl Layout {
    /// All panes, from left to right and from top to bottom.
    pub fn panes(&self) -> Vec<PaneId> {
        match self {
            Layout::Pane(pane) => vec![*pane],
            Layout::Split { first, second, .. } => {
                let mut panes = first.panes();
                panes.extend(second.panes());
                panes
            }
        }
    }

    pub fn contains(&self, pane: PaneId) -> bool {
        match self {
            Layout::Pane(p) => *p == pane,
            Layout::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
    }

    /// Splits `target`: it keeps the first half, `new` gets the second. False if `target` is not here.
    pub fn split(&mut self, target: PaneId, new: PaneId, direction: Direction) -> bool {
        match self {
            Layout::Pane(pane) if *pane == target => {
                *self = Layout::Split {
                    direction,
                    ratio: 0.5,
                    first: Box::new(Layout::Pane(target)),
                    second: Box::new(Layout::Pane(new)),
                };
                true
            }
            Layout::Pane(_) => false,
            Layout::Split { first, second, .. } => {
                first.split(target, new, direction) || second.split(target, new, direction)
            }
        }
    }

    /// Removes a pane. Its neighbor gets the space. False if the pane is not here
    /// or it is the only pane (a tab always has at least one pane).
    pub fn remove(&mut self, pane: PaneId) -> bool {
        let Layout::Split { first, second, .. } = self else {
            return false;
        };
        if **first == Layout::Pane(pane) {
            *self = std::mem::replace(second.as_mut(), Layout::Pane(pane));
            return true;
        }
        if **second == Layout::Pane(pane) {
            *self = std::mem::replace(first.as_mut(), Layout::Pane(pane));
            return true;
        }
        first.remove(pane) || second.remove(pane)
    }

    /// The place of each pane in `area`. Edges are whole pixels, with no gaps and no overlap.
    pub fn rects(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        match self {
            Layout::Pane(pane) => vec![(*pane, area)],
            Layout::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (a, b) = match direction {
                    Direction::Right => {
                        let edge = (area.x + area.width * ratio).round();
                        (
                            Rect::new(area.x, area.y, edge - area.x, area.height),
                            Rect::new(edge, area.y, area.x + area.width - edge, area.height),
                        )
                    }
                    Direction::Down => {
                        let edge = (area.y + area.height * ratio).round();
                        (
                            Rect::new(area.x, area.y, area.width, edge - area.y),
                            Rect::new(area.x, edge, area.width, area.y + area.height - edge),
                        )
                    }
                };
                let mut rects = first.rects(a);
                rects.extend(second.rects(b));
                rects
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: PaneId = PaneId(1);
    const B: PaneId = PaneId(2);
    const C: PaneId = PaneId(3);
    const AREA: Rect = Rect {
        x: 0.0,
        y: 30.0,
        width: 801.0,
        height: 600.0,
    };

    #[test]
    fn one_pane_takes_the_whole_area() {
        let layout = Layout::Pane(A);
        assert_eq!(layout.panes(), [A]);
        assert_eq!(layout.rects(AREA), [(A, AREA)]);
        assert!(layout.contains(A) && !layout.contains(B));
    }

    #[test]
    fn split_right_makes_two_columns_with_no_gap() {
        let mut layout = Layout::Pane(A);
        assert!(layout.split(A, B, Direction::Right));
        assert_eq!(layout.panes(), [A, B]);
        let rects = layout.rects(AREA);
        let (a, b) = (rects[0].1, rects[1].1);
        assert_eq!(a.x, 0.0);
        assert_eq!(a.x + a.width, b.x, "no gap");
        assert_eq!(b.x + b.width, AREA.width);
        assert_eq!(a.width.fract(), 0.0, "whole pixels");
        assert_eq!((a.y, a.height), (AREA.y, AREA.height));
        assert!((a.width - b.width).abs() <= 1.0);
    }

    #[test]
    fn split_down_makes_two_rows() {
        let mut layout = Layout::Pane(A);
        layout.split(A, B, Direction::Down);
        let rects = layout.rects(AREA);
        let (a, b) = (rects[0].1, rects[1].1);
        assert_eq!(a.y, AREA.y);
        assert_eq!(a.y + a.height, b.y);
        assert_eq!(b.y + b.height, AREA.y + AREA.height);
        assert_eq!(a.width, AREA.width);
    }

    #[test]
    fn nested_split() {
        // A | (B over C)
        let mut layout = Layout::Pane(A);
        layout.split(A, B, Direction::Right);
        assert!(layout.split(B, C, Direction::Down));
        assert_eq!(layout.panes(), [A, B, C]);
        let rects = layout.rects(AREA);
        let (b, c) = (rects[1].1, rects[2].1);
        assert_eq!(b.x, c.x);
        assert_eq!(b.y + b.height, c.y);
    }

    #[test]
    fn split_of_an_unknown_pane_does_nothing() {
        let mut layout = Layout::Pane(A);
        assert!(!layout.split(C, B, Direction::Right));
        assert_eq!(layout, Layout::Pane(A));
    }

    #[test]
    fn remove_gives_the_space_to_the_neighbor() {
        let mut layout = Layout::Pane(A);
        layout.split(A, B, Direction::Right);
        layout.split(B, C, Direction::Down);
        assert!(layout.remove(B));
        assert_eq!(layout.panes(), [A, C]);
        // C now has the whole right half.
        let rects = layout.rects(AREA);
        assert_eq!(rects[1].1.height, AREA.height);
        assert!(layout.remove(A));
        assert_eq!(layout, Layout::Pane(C));
    }

    #[test]
    fn the_only_pane_cannot_be_removed() {
        let mut layout = Layout::Pane(A);
        assert!(!layout.remove(A));
        assert!(!layout.remove(B));
    }

    #[test]
    fn rect_contains() {
        let r = Rect::new(10.0, 20.0, 30.0, 40.0);
        assert!(r.contains(10.0, 20.0) && r.contains(39.9, 59.9));
        assert!(!r.contains(40.0, 30.0) && !r.contains(9.9, 30.0));
    }
}
