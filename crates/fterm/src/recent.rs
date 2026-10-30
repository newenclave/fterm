//! Recent panes: the panes that had the keys, the newest first, for "go back" (Alt+`) and the list (Ctrl+Shift+H).
//! A pane counts when the keys stay in it for `DWELL`: a quick walk through tabs (Ctrl+Tab) does not fill the list.

use std::time::{Duration, Instant};

use fterm_mux::mux::PaneId;

/// How long the keys must stay in a pane before it counts as visited.
pub const DWELL: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub struct Recent {
    /// Visited panes, the newest first (not the current one).
    list: Vec<PaneId>,
    /// The pane with the keys now, and since when.
    current: Option<(PaneId, Instant)>,
}

impl Recent {
    /// The keys are in `pane` at `now` (called often; only a change does something).
    pub fn focus(&mut self, pane: PaneId, now: Instant) {
        match self.current {
            Some((p, _)) if p == pane => {}
            Some((old, since)) => {
                if now.saturating_duration_since(since) >= DWELL {
                    self.push(old);
                }
                self.current = Some((pane, now));
            }
            None => self.current = Some((pane, now)),
        }
    }

    /// The current pane counts now, however short it had the keys (before a jump from it).
    pub fn keep(&mut self) {
        if let Some((pane, _)) = self.current {
            self.push(pane);
        }
    }

    fn push(&mut self, pane: PaneId) {
        self.list.retain(|p| *p != pane);
        self.list.insert(0, pane);
    }

    /// The pane to go back to: the newest visited one that is not the current one and is still there.
    pub fn previous(&self, alive: impl Fn(PaneId) -> bool) -> Option<PaneId> {
        let current = self.current.map(|(p, _)| p);
        self.list
            .iter()
            .copied()
            .find(|p| Some(*p) != current && alive(*p))
    }

    /// All panes of `panes` but the current one: the visited ones (the newest first), then the others in their
    /// order.
    pub fn order(&self, panes: &[PaneId]) -> Vec<PaneId> {
        let current = self.current.map(|(p, _)| p);
        let visited = self.list.iter().copied().filter(|p| panes.contains(p));
        let others = panes.iter().copied().filter(|p| !self.list.contains(p));
        visited
            .chain(others)
            .filter(|p| Some(*p) != current)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_pane_counts_after_the_keys_stay_there() {
        let t0 = Instant::now();
        let (a, b, c, d) = (PaneId(1), PaneId(2), PaneId(3), PaneId(4));
        let mut r = Recent::default();
        r.focus(a, t0);
        r.focus(a, t0 + ms(5000));
        // Ctrl+Tab three times: b and c only for a moment.
        r.focus(b, t0 + ms(5000));
        r.focus(c, t0 + ms(5200));
        r.focus(d, t0 + ms(5400));
        assert_eq!(r.previous(|_| true), Some(a), "back to where you worked");
        // d stays a while, then the user goes to c.
        r.focus(c, t0 + ms(9000));
        assert_eq!(r.previous(|_| true), Some(d));
        assert_eq!(
            r.order(&[a, b, c, d]),
            [d, a, b],
            "visited first, then the rest"
        );
    }

    #[test]
    fn going_back_twice_comes_back() {
        let t0 = Instant::now();
        let (a, b) = (PaneId(1), PaneId(2));
        let mut r = Recent::default();
        r.focus(a, t0);
        r.focus(b, t0 + ms(3000));
        // Alt+`: a jump from b at once (b had the keys for a moment only).
        let back = r.previous(|_| true).unwrap();
        assert_eq!(back, a);
        r.keep();
        r.focus(back, t0 + ms(3100));
        assert_eq!(r.previous(|_| true), Some(b));
        r.keep();
        r.focus(b, t0 + ms(3200));
        assert_eq!(r.previous(|_| true), Some(a));
    }

    #[test]
    fn closed_panes_are_skipped() {
        let t0 = Instant::now();
        let (a, b, c) = (PaneId(1), PaneId(2), PaneId(3));
        let mut r = Recent::default();
        r.focus(a, t0);
        r.focus(b, t0 + ms(2000));
        r.focus(c, t0 + ms(4000));
        assert_eq!(r.previous(|p| p != b), Some(a));
        assert_eq!(r.order(&[a, c]), [a], "b is gone, c is the current one");
        assert_eq!(Recent::default().previous(|_| true), None);
    }
}
