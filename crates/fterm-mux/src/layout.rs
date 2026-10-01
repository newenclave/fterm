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

/// A side of a pane, or a way to move between panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Up,
    Down,
}

/// The line between the two parts of a split.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    /// The way to the split in the tree: `false` = first, `true` = second.
    pub path: Vec<bool>,
    /// A 1 px line in window pixels.
    pub rect: Rect,
    pub direction: Direction,
}

/// The smallest and largest part of a split.
pub const MIN_RATIO: f32 = 0.1;
pub const MAX_RATIO: f32 = 0.9;

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

    /// Removes a pane. Its neighbor gets the space. Returns the first pane of the neighbor
    /// (the focus can go there). `None` if the pane is not here or it is the only pane.
    pub fn remove(&mut self, pane: PaneId) -> Option<PaneId> {
        let Layout::Split { first, second, .. } = self else {
            return None;
        };
        let keep = if **first == Layout::Pane(pane) {
            Some(std::mem::replace(second.as_mut(), Layout::Pane(pane)))
        } else if **second == Layout::Pane(pane) {
            Some(std::mem::replace(first.as_mut(), Layout::Pane(pane)))
        } else {
            None
        };
        if let Some(keep) = keep {
            let focus = keep.panes()[0];
            *self = keep;
            return Some(focus);
        }
        first.remove(pane).or_else(|| second.remove(pane))
    }

    /// All lines between panes.
    pub fn dividers(&self, area: Rect) -> Vec<Divider> {
        let mut out = Vec::new();
        self.collect_dividers(area, &mut Vec::new(), &mut out);
        out
    }

    fn collect_dividers(&self, area: Rect, path: &mut Vec<bool>, out: &mut Vec<Divider>) {
        let Layout::Split {
            direction,
            ratio,
            first,
            second,
        } = self
        else {
            return;
        };
        let (a, b) = split_areas(*direction, *ratio, area);
        let rect = match direction {
            Direction::Right => Rect::new(b.x, area.y, 1.0, area.height),
            Direction::Down => Rect::new(area.x, b.y, area.width, 1.0),
        };
        out.push(Divider {
            path: path.clone(),
            rect,
            direction: *direction,
        });
        path.push(false);
        first.collect_dividers(a, path, out);
        path.pop();
        path.push(true);
        second.collect_dividers(b, path, out);
        path.pop();
    }

    /// Sets the ratio of the split at `path` (kept in `MIN_RATIO..=MAX_RATIO`).
    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) {
        let mut node = self;
        for &go_second in path {
            match node {
                Layout::Split { first, second, .. } => {
                    node = if go_second { second } else { first };
                }
                Layout::Pane(_) => return,
            }
        }
        if let Layout::Split { ratio: r, .. } = node {
            *r = ratio.clamp(MIN_RATIO, MAX_RATIO);
        }
    }

    /// The ratio for the split at `path` that puts its divider at the mouse position.
    pub fn ratio_at(&self, path: &[bool], area: Rect, x: f32, y: f32) -> Option<f32> {
        let (node, area) = self.node_at(path, area)?;
        let Layout::Split { direction, .. } = node else {
            return None;
        };
        let ratio = match direction {
            Direction::Right => (x - area.x) / area.width,
            Direction::Down => (y - area.y) / area.height,
        };
        Some(ratio.clamp(MIN_RATIO, MAX_RATIO))
    }

    /// The node at `path` and its area.
    fn node_at(&self, path: &[bool], area: Rect) -> Option<(&Layout, Rect)> {
        let mut node = self;
        let mut area = area;
        for &go_second in path {
            let Layout::Split {
                direction,
                ratio,
                first,
                second,
            } = node
            else {
                return None;
            };
            let (a, b) = split_areas(*direction, *ratio, area);
            (node, area) = if go_second { (second, b) } else { (first, a) };
        }
        Some((node, area))
    }

    /// The way from the root to `pane`.
    fn path_to(&self, pane: PaneId) -> Option<Vec<bool>> {
        match self {
            Layout::Pane(p) => (*p == pane).then(Vec::new),
            Layout::Split { first, second, .. } => {
                if let Some(mut path) = first.path_to(pane) {
                    path.insert(0, false);
                    Some(path)
                } else {
                    let mut path = second.path_to(pane)?;
                    path.insert(0, true);
                    Some(path)
                }
            }
        }
    }

    /// Moves the nearest divider of `pane` toward `edge`, by `step` pixels (keyboard resize).
    pub fn move_divider(&mut self, pane: PaneId, edge: Edge, step: f32, area: Rect) {
        let Some(path) = self.path_to(pane) else {
            return;
        };
        let axis = match edge {
            Edge::Left | Edge::Right => Direction::Right,
            Edge::Up | Edge::Down => Direction::Down,
        };
        // The nearest split above the pane that splits along this axis.
        let mut found = None;
        for depth in 0..path.len() {
            if let Some((
                Layout::Split {
                    direction, ratio, ..
                },
                split_area,
            )) = self.node_at(&path[..depth], area)
                && *direction == axis
            {
                found = Some((depth, *ratio, split_area));
            }
        }
        let Some((depth, ratio, split_area)) = found else {
            return;
        };
        let size = match axis {
            Direction::Right => split_area.width,
            Direction::Down => split_area.height,
        };
        let delta = match edge {
            Edge::Right | Edge::Down => step / size,
            Edge::Left | Edge::Up => -step / size,
        };
        self.set_ratio(&path[..depth], ratio + delta);
    }

    /// The pane next to `pane` toward `edge`. If there are many, the one that touches it the most.
    pub fn neighbor(&self, pane: PaneId, edge: Edge, area: Rect) -> Option<PaneId> {
        let rects = self.rects(area);
        let me = rects.iter().find(|(p, _)| *p == pane)?.1;
        let near = |a: f32, b: f32| (a - b).abs() <= 1.5;
        let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| (a1.min(b1) - a0.max(b0)).max(0.0);
        rects
            .iter()
            .filter(|(p, _)| *p != pane)
            .filter_map(|(p, r)| {
                let touch = match edge {
                    Edge::Right => near(r.x, me.x + me.width),
                    Edge::Left => near(r.x + r.width, me.x),
                    Edge::Down => near(r.y, me.y + me.height),
                    Edge::Up => near(r.y + r.height, me.y),
                };
                let shared = match edge {
                    Edge::Left | Edge::Right => {
                        overlap(me.y, me.y + me.height, r.y, r.y + r.height)
                    }
                    Edge::Up | Edge::Down => overlap(me.x, me.x + me.width, r.x, r.x + r.width),
                };
                (touch && shared > 0.0).then_some((*p, shared))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(p, _)| p)
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
                let (a, b) = split_areas(*direction, *ratio, area);
                let mut rects = first.rects(a);
                rects.extend(second.rects(b));
                rects
            }
        }
    }
}

/// The two parts of a split area. The edge is a whole pixel.
fn split_areas(direction: Direction, ratio: f32, area: Rect) -> (Rect, Rect) {
    match direction {
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
        assert_eq!(layout.remove(B), Some(C));
        assert_eq!(layout.panes(), [A, C]);
        // C now has the whole right half.
        let rects = layout.rects(AREA);
        assert_eq!(rects[1].1.height, AREA.height);
        assert_eq!(layout.remove(A), Some(C));
        assert_eq!(layout, Layout::Pane(C));
    }

    #[test]
    fn the_only_pane_cannot_be_removed() {
        let mut layout = Layout::Pane(A);
        assert_eq!(layout.remove(A), None);
        assert_eq!(layout.remove(B), None);
    }

    #[test]
    fn rect_contains() {
        let r = Rect::new(10.0, 20.0, 30.0, 40.0);
        assert!(r.contains(10.0, 20.0) && r.contains(39.9, 59.9));
        assert!(!r.contains(40.0, 30.0) && !r.contains(9.9, 30.0));
    }

    const D: PaneId = PaneId(4);

    /// A 2x2 grid: A | B on top, C | D below. Columns are split first: (A over C) | (B over D).
    fn grid() -> Layout {
        let mut layout = Layout::Pane(A);
        layout.split(A, B, Direction::Right);
        layout.split(A, C, Direction::Down);
        layout.split(B, D, Direction::Down);
        layout
    }

    #[test]
    fn remove_gives_the_focus_to_the_first_pane_of_the_neighbor() {
        // A | (B over C): removing A gives the space to (B over C), and B gets the focus.
        let mut layout = Layout::Pane(A);
        layout.split(A, B, Direction::Right);
        layout.split(B, C, Direction::Down);
        assert_eq!(layout.remove(A), Some(B));
    }

    #[test]
    fn neighbors_in_a_grid() {
        let layout = grid();
        assert_eq!(layout.neighbor(A, Edge::Right, AREA), Some(B));
        assert_eq!(layout.neighbor(A, Edge::Down, AREA), Some(C));
        assert_eq!(layout.neighbor(D, Edge::Left, AREA), Some(C));
        assert_eq!(layout.neighbor(D, Edge::Up, AREA), Some(B));
        assert_eq!(layout.neighbor(A, Edge::Left, AREA), None);
        assert_eq!(layout.neighbor(A, Edge::Up, AREA), None);
    }

    #[test]
    fn neighbor_is_the_one_that_touches_the_most() {
        // A on the left, (B over C) on the right with B small: from A to the right is C.
        let mut layout = Layout::Pane(A);
        layout.split(A, B, Direction::Right);
        layout.split(B, C, Direction::Down);
        layout.set_ratio(&[true], 0.2);
        assert_eq!(layout.neighbor(A, Edge::Right, AREA), Some(C));
    }

    #[test]
    fn dividers_of_a_grid() {
        let layout = grid();
        let dividers = layout.dividers(AREA);
        assert_eq!(dividers.len(), 3);
        // The first one is the vertical line between the columns.
        let main = &dividers[0];
        assert_eq!(main.path, Vec::<bool>::new());
        assert_eq!(main.direction, Direction::Right);
        assert_eq!(main.rect.width, 1.0);
        assert_eq!(main.rect.height, AREA.height);
        assert_eq!(
            main.rect.x,
            layout.rects(AREA)[2].1.x,
            "at the left edge of B"
        );
    }

    #[test]
    fn ratio_is_kept_in_range() {
        let mut layout = Layout::Pane(A);
        layout.split(A, B, Direction::Right);
        layout.set_ratio(&[], 0.0);
        let rects = layout.rects(AREA);
        assert!((rects[0].1.width / AREA.width - MIN_RATIO).abs() < 0.01);
        layout.set_ratio(&[], 5.0);
        let rects = layout.rects(AREA);
        assert!((rects[0].1.width / AREA.width - MAX_RATIO).abs() < 0.01);
    }

    #[test]
    fn ratio_at_follows_the_mouse() {
        let mut layout = Layout::Pane(A);
        layout.split(A, B, Direction::Right);
        layout.split(B, C, Direction::Down);
        // The main split: x = 200 of 801.
        let ratio = layout.ratio_at(&[], AREA, 200.0, 100.0).unwrap();
        assert!((ratio - 200.0 / 801.0).abs() < 0.001);
        // The split inside the right half: y from its own top.
        let ratio = layout.ratio_at(&[true], AREA, 600.0, 30.0 + 150.0).unwrap();
        assert!((ratio - 0.25).abs() < 0.001);
        assert_eq!(
            layout.ratio_at(&[false], AREA, 0.0, 0.0),
            None,
            "no split there"
        );
    }

    #[test]
    fn keyboard_resize_moves_the_nearest_divider() {
        let mut layout = Layout::Pane(A);
        layout.split(A, B, Direction::Right);
        let before = layout.rects(AREA)[0].1.width;
        layout.move_divider(A, Edge::Right, 40.0, AREA);
        assert_eq!(layout.rects(AREA)[0].1.width, before + 40.0);
        // From B, the same divider moves left.
        layout.move_divider(B, Edge::Left, 40.0, AREA);
        assert_eq!(layout.rects(AREA)[0].1.width, before);
        // No divider up or down: nothing happens.
        layout.move_divider(A, Edge::Up, 40.0, AREA);
        assert_eq!(layout.rects(AREA)[0].1.width, before);
    }
}
