//! Lines between panes and the frame of the active pane.

use crate::frame::{Instance, Rect};
use crate::tabbar::solid;
use crate::theme::UiColors;

/// Quads for the lines between panes, and a 1 px frame inside `active`.
/// Give `active` only when the tab has more than one pane.
pub fn build_pane_chrome(dividers: &[Rect], active: Option<Rect>, ui: &UiColors) -> Vec<Instance> {
    let mut quads: Vec<Instance> = dividers
        .iter()
        .map(|&rect| solid(rect, ui.selected))
        .collect();
    if let Some(r) = active {
        quads.extend([
            solid(Rect::new(r.x, r.y, r.width, 1.0), ui.accent),
            solid(
                Rect::new(r.x, r.y + r.height - 1.0, r.width, 1.0),
                ui.accent,
            ),
            solid(Rect::new(r.x, r.y, 1.0, r.height), ui.accent),
            solid(
                Rect::new(r.x + r.width - 1.0, r.y, 1.0, r.height),
                ui.accent,
            ),
        ]);
    }
    quads
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> UiColors {
        UiColors::default()
    }
    use crate::color::linear;

    #[test]
    fn each_divider_is_a_line() {
        let lines = [
            Rect::new(400.0, 30.0, 1.0, 570.0),
            Rect::new(0.0, 300.0, 400.0, 1.0),
        ];
        let quads = build_pane_chrome(&lines, None, &ui());
        assert_eq!(quads.len(), 2);
        assert_eq!(quads[0].rect, [400.0, 30.0, 1.0, 570.0]);
        assert!(quads.iter().all(|q| q.color == linear(ui().selected)));
    }

    #[test]
    fn active_pane_gets_a_frame_inside_it() {
        let pane = Rect::new(10.0, 20.0, 100.0, 50.0);
        let quads = build_pane_chrome(&[], Some(pane), &ui());
        assert_eq!(quads.len(), 4);
        for q in &quads {
            assert_eq!(q.color, linear(ui().accent));
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
        assert!(build_pane_chrome(&[], None, &ui()).is_empty());
    }

    #[test]
    fn a_custom_theme_colors_the_chrome() {
        use fterm_term::alacritty_terminal::vte::ansi::Rgb;
        let custom = UiColors {
            accent: Rgb { r: 255, g: 0, b: 0 },
            selected: Rgb { r: 0, g: 255, b: 0 },
            ..UiColors::default()
        };
        let line = Rect::new(400.0, 30.0, 1.0, 570.0);
        let pane = Rect::new(0.0, 30.0, 400.0, 570.0);
        let quads = build_pane_chrome(&[line], Some(pane), &custom);
        assert_eq!(quads.len(), 5);
        assert_eq!(quads[0].color, linear(custom.selected));
        assert!(quads[1..].iter().all(|q| q.color == linear(custom.accent)));
    }
}
