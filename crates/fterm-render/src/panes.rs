//! Lines between panes and the frame of the active pane.

use fterm_term::alacritty_terminal::vte::ansi::Rgb;

use crate::frame::{Instance, Rect};
use crate::tabbar::solid;

/// Lines between panes (Catppuccin Mocha "surface1").
pub const DIVIDER: Rgb = Rgb {
    r: 0x45,
    g: 0x47,
    b: 0x5a,
};
/// The frame of the active pane (Catppuccin Mocha "mauve").
pub const ACTIVE_FRAME: Rgb = Rgb {
    r: 0xcb,
    g: 0xa6,
    b: 0xf7,
};

/// Quads for the lines between panes, and a 1 px frame inside `active`.
/// Give `active` only when the tab has more than one pane.
pub fn build_pane_chrome(dividers: &[Rect], active: Option<Rect>) -> Vec<Instance> {
    let mut quads: Vec<Instance> = dividers.iter().map(|&rect| solid(rect, DIVIDER)).collect();
    if let Some(r) = active {
        quads.extend([
            solid(Rect::new(r.x, r.y, r.width, 1.0), ACTIVE_FRAME),
            solid(
                Rect::new(r.x, r.y + r.height - 1.0, r.width, 1.0),
                ACTIVE_FRAME,
            ),
            solid(Rect::new(r.x, r.y, 1.0, r.height), ACTIVE_FRAME),
            solid(
                Rect::new(r.x + r.width - 1.0, r.y, 1.0, r.height),
                ACTIVE_FRAME,
            ),
        ]);
    }
    quads
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::linear;

    #[test]
    fn each_divider_is_a_line() {
        let lines = [
            Rect::new(400.0, 30.0, 1.0, 570.0),
            Rect::new(0.0, 300.0, 400.0, 1.0),
        ];
        let quads = build_pane_chrome(&lines, None);
        assert_eq!(quads.len(), 2);
        assert_eq!(quads[0].rect, [400.0, 30.0, 1.0, 570.0]);
        assert!(quads.iter().all(|q| q.color == linear(DIVIDER)));
    }

    #[test]
    fn active_pane_gets_a_frame_inside_it() {
        let pane = Rect::new(10.0, 20.0, 100.0, 50.0);
        let quads = build_pane_chrome(&[], Some(pane));
        assert_eq!(quads.len(), 4);
        for q in &quads {
            assert_eq!(q.color, linear(ACTIVE_FRAME));
            let [x, y, w, h] = q.rect;
            assert!(
                x >= 10.0 && y >= 20.0 && x + w <= 110.0 && y + h <= 70.0,
                "{:?}",
                q.rect
            );
            assert!(w == 1.0 || h == 1.0, "thin");
        }
    }

    #[test]
    fn one_pane_has_no_chrome() {
        assert!(build_pane_chrome(&[], None).is_empty());
    }
}
