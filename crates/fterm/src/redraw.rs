//! How often a change that is not on the screen redraws the window. The spinner of an agent in another tab changes
//! its title about 10 times a second; each change would draw the whole frame. Such a change waits for the next
//! frame, but at most `interval`.

use std::time::{Duration, Instant};

/// When the next frame for changes off the screen is due.
#[derive(Debug)]
pub struct Throttle {
    interval: Duration,
    /// The last frame (for any reason).
    last: Option<Instant>,
    /// A change off the screen waits for a frame.
    pending: bool,
}

/// What to do with a change off the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Redraw {
    /// Draw now.
    Now,
    /// Draw at this time (unless a frame comes before it).
    At(Instant),
}

impl Throttle {
    /// `per_second` frames a second at most for changes off the screen.
    pub fn new(per_second: f64) -> Self {
        Self {
            interval: interval(per_second),
            last: None,
            pending: false,
        }
    }

    pub fn set_rate(&mut self, per_second: f64) {
        self.interval = interval(per_second);
    }

    /// A change off the screen came at `now`.
    pub fn change(&mut self, now: Instant) -> Redraw {
        match self.last {
            Some(last) if now.saturating_duration_since(last) < self.interval => {
                self.pending = true;
                Redraw::At(last + self.interval)
            }
            _ => Redraw::Now,
        }
    }

    /// A frame was drawn: the changes that waited are on it.
    pub fn drawn(&mut self, now: Instant) {
        self.last = Some(now);
        self.pending = false;
    }

    /// When a waiting change needs its frame (`None` = nothing waits).
    pub fn due(&self) -> Option<Instant> {
        self.pending
            .then(|| self.last.map_or_else(Instant::now, |l| l + self.interval))
    }
}

fn interval(per_second: f64) -> Duration {
    Duration::from_secs_f64(1.0 / per_second.clamp(0.1, 60.0))
}

/// Frames per second and the mean frame time, for the log.
#[derive(Debug, Default)]
pub struct FrameStats {
    since: Option<Instant>,
    frames: u32,
    busy: Duration,
}

impl FrameStats {
    /// A frame took `took`. Every `every` it gives (frames per second, mean ms) and starts again.
    pub fn frame(&mut self, now: Instant, took: Duration, every: Duration) -> Option<(f64, f64)> {
        let since = *self.since.get_or_insert(now);
        self.frames += 1;
        self.busy += took;
        let span = now.saturating_duration_since(since);
        if span < every {
            return None;
        }
        let out = (
            f64::from(self.frames) / span.as_secs_f64(),
            self.busy.as_secs_f64() * 1000.0 / f64::from(self.frames),
        );
        *self = Self {
            since: Some(now),
            ..Self::default()
        };
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_first_change_draws_at_once() {
        let mut t = Throttle::new(1.0);
        assert_eq!(t.change(Instant::now()), Redraw::Now);
    }

    #[test]
    fn changes_soon_after_a_frame_wait_for_the_interval() {
        let t0 = Instant::now();
        let mut t = Throttle::new(1.0);
        t.drawn(t0);
        // The spinner changes 10 times a second: one frame a second.
        assert_eq!(t.change(t0 + ms(100)), Redraw::At(t0 + ms(1000)));
        assert_eq!(t.change(t0 + ms(200)), Redraw::At(t0 + ms(1000)));
        assert_eq!(t.due(), Some(t0 + ms(1000)));
        t.drawn(t0 + ms(1000));
        assert_eq!(t.due(), None, "the frame took the changes");
        assert_eq!(
            t.change(t0 + ms(2100)),
            Redraw::Now,
            "a second later: at once"
        );
    }

    #[test]
    fn a_frame_for_another_reason_takes_the_waiting_changes() {
        let t0 = Instant::now();
        let mut t = Throttle::new(1.0);
        t.drawn(t0);
        t.change(t0 + ms(100));
        // The user types: a frame at once, and the change is on it.
        t.drawn(t0 + ms(300));
        assert_eq!(t.due(), None);
        assert_eq!(t.change(t0 + ms(400)), Redraw::At(t0 + ms(1300)));
    }

    #[test]
    fn the_rate_can_change() {
        let t0 = Instant::now();
        let mut t = Throttle::new(1.0);
        t.set_rate(4.0);
        t.drawn(t0);
        assert_eq!(t.change(t0 + ms(100)), Redraw::At(t0 + ms(250)));
        // Bad rates are cut (at most 60, at least one every 10 s).
        t.set_rate(0.0);
        assert_eq!(t.change(t0 + ms(100)), Redraw::At(t0 + ms(10_000)));
    }

    #[test]
    fn frame_stats_every_few_seconds() {
        let t0 = Instant::now();
        let mut s = FrameStats::default();
        for i in 0..9 {
            assert_eq!(s.frame(t0 + ms(i * 500), ms(4), ms(5000)), None);
        }
        // The 11th frame at 5 s: 11 frames in 5 s, 4 ms each.
        assert_eq!(s.frame(t0 + ms(4500), ms(4), ms(5000)), None);
        let (fps, mean) = s.frame(t0 + ms(5000), ms(4), ms(5000)).unwrap();
        assert!((fps - 2.2).abs() < 0.01, "{fps}");
        assert!((mean - 4.0).abs() < 0.01, "{mean}");
        assert_eq!(
            s.frame(t0 + ms(5100), ms(4), ms(5000)),
            None,
            "counting again"
        );
    }
}
