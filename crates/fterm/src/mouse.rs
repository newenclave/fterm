//! Mouse logic without a window: which cell is under the mouse, auto-scroll speed, double clicks.

use std::time::{Duration, Instant};

use fterm_term::alacritty_terminal::index::{Column, Line, Point, Side};
use fterm_term::size::GridSize;
use winit::event::MouseScrollDelta;

/// Two clicks closer in time than this are a double (or triple) click.
pub const MULTI_CLICK: Duration = Duration::from_millis(400);
/// Max lines per auto-scroll tick.
pub const MAX_AUTOSCROLL: i32 = 10;

/// Where the grid is in the window, in pixels.
#[derive(Clone, Copy, Debug)]
pub struct GridGeometry {
    pub cell_width: f32,
    pub cell_height: f32,
    pub padding: f32,
    pub size: GridSize,
}

impl GridGeometry {
    /// The cell under the mouse (kept inside the grid), and the half of the cell.
    /// `display_offset` is how far the view is scrolled up, so the point is in history coordinates.
    pub fn cell_at(&self, x: f64, y: f64, display_offset: usize) -> (Point, Side) {
        let columns = self.size.columns.max(1);
        let rows = self.size.rows.max(1);
        let col_f = (x as f32 - self.padding) / self.cell_width;
        let row_f = (y as f32 - self.padding) / self.cell_height;
        let col = (col_f.max(0.0) as usize).min(columns - 1);
        let row = (row_f.max(0.0) as usize).min(rows - 1);
        let side = if col_f < 0.0 {
            Side::Left
        } else if col_f >= columns as f32 || col_f.fract() >= 0.5 {
            Side::Right
        } else {
            Side::Left
        };
        let line = Line(row as i32 - display_offset as i32);
        (Point::new(line, Column(col)), side)
    }
}

/// Lines to scroll while the user drags a selection out of the window.
/// More than 0 = up (into the history), less than 0 = down, 0 = the mouse is inside the window.
/// The farther away the mouse is, the faster it scrolls.
pub fn autoscroll_lines(y: f64, window_height: f64) -> i32 {
    // Every 20 pixels away from the window edge: one more line per tick.
    const PIXELS_PER_LINE: f64 = 20.0;
    let speed = |distance: f64| (1 + (distance / PIXELS_PER_LINE) as i32).min(MAX_AUTOSCROLL);
    if y < 0.0 {
        speed(-y)
    } else if y > window_height {
        -speed(y - window_height)
    } else {
        0
    }
}

/// Counts clicks: 1 = single, 2 = double, 3 = triple, then 1 again.
#[derive(Default)]
pub struct ClickCounter {
    last: Option<(Instant, Point)>,
    count: u8,
}

impl ClickCounter {
    pub fn click(&mut self, now: Instant, point: Point) -> u8 {
        let again = self.last.is_some_and(|(time, last_point)| {
            last_point == point && now.duration_since(time) <= MULTI_CLICK
        });
        self.count = if again && self.count < 3 {
            self.count + 1
        } else {
            1
        };
        self.last = Some((now, point));
        self.count
    }
}

/// Lines per wheel step.
pub const WHEEL_LINES: f32 = 3.0;

/// Turns wheel and touchpad movement into whole lines. > 0 = up (into the history).
#[derive(Default)]
pub struct Wheel {
    pixels: f64,
}

impl Wheel {
    pub fn lines(&mut self, delta: MouseScrollDelta, cell_height: f32) -> i32 {
        match delta {
            MouseScrollDelta::LineDelta(_, y) => (y * WHEEL_LINES).round() as i32,
            MouseScrollDelta::PixelDelta(position) => {
                self.pixels += position.y;
                let lines = (self.pixels / f64::from(cell_height)).trunc();
                self.pixels -= lines * f64::from(cell_height);
                lines as i32
            }
        }
    }
}

/// A mouse event for an app that asked for the mouse (vim, htop, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    Press(ReportButton),
    Release(ReportButton),
    /// The mouse moved. `Some` = with this button down.
    Motion(Option<ReportButton>),
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ReportMods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// Bytes for a mouse event. `column` and `row` start at 0 (screen cells).
/// `sgr` = the app asked for SGR mode (1006): `ESC [ < b ; x ; y M/m`.
/// Without SGR, the old X10 form `ESC [ M b x y` is used; it cannot show cells after 223.
pub fn encode_mouse(
    kind: ReportKind,
    column: usize,
    row: usize,
    mods: ReportMods,
    sgr: bool,
) -> Option<Vec<u8>> {
    let button = |b: ReportButton| match b {
        ReportButton::Left => 0,
        ReportButton::Middle => 1,
        ReportButton::Right => 2,
    };
    let mut code: u32 = match kind {
        ReportKind::Press(b) => button(b),
        // X10 has no button for release: it is always 3.
        ReportKind::Release(b) if sgr => button(b),
        ReportKind::Release(_) => 3,
        ReportKind::Motion(Some(b)) => 32 + button(b),
        ReportKind::Motion(None) => 32 + 3,
        ReportKind::WheelUp => 64,
        ReportKind::WheelDown => 65,
    };
    if mods.shift {
        code += 4;
    }
    if mods.alt {
        code += 8;
    }
    if mods.ctrl {
        code += 16;
    }
    let (x, y) = (column + 1, row + 1);
    if sgr {
        let end = if matches!(kind, ReportKind::Release(_)) {
            'm'
        } else {
            'M'
        };
        return Some(format!("[<{code};{x};{y}{end}").into_bytes());
    }
    // X10: each value is one byte plus 32.
    let byte = |v: usize| u8::try_from(v + 32).ok().filter(|&b| b < 255);
    Some(vec![
        0x1b,
        b'[',
        b'M',
        byte(code as usize)?,
        byte(x)?,
        byte(y)?,
    ])
}

#[cfg(test)]
mod tests {
    use winit::dpi::PhysicalPosition;

    use super::*;

    #[test]
    fn wheel_step_is_three_lines() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.lines(MouseScrollDelta::LineDelta(0.0, 1.0), 20.0), 3);
        assert_eq!(
            wheel.lines(MouseScrollDelta::LineDelta(0.0, -2.0), 20.0),
            -6
        );
    }

    #[test]
    fn touchpad_pixels_add_up_to_lines() {
        let mut wheel = Wheel::default();
        let px = |y| MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, y));
        assert_eq!(wheel.lines(px(15.0), 20.0), 0);
        assert_eq!(wheel.lines(px(15.0), 20.0), 1);
        assert_eq!(wheel.lines(px(-50.0), 20.0), -2);
    }

    const GEO: GridGeometry = GridGeometry {
        cell_width: 10.0,
        cell_height: 20.0,
        padding: 5.0,
        size: GridSize {
            columns: 80,
            rows: 24,
        },
    };

    fn point(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    #[test]
    fn cell_under_the_mouse() {
        // x = 5 + 3*10 + 2 -> column 3, left half. y = 5 + 2*20 + 1 -> row 2.
        assert_eq!(GEO.cell_at(37.0, 46.0, 0), (point(2, 3), Side::Left));
        // Right half of the cell.
        assert_eq!(GEO.cell_at(43.0, 46.0, 0), (point(2, 3), Side::Right));
    }

    #[test]
    fn padding_and_outside_go_to_the_edge_cells() {
        assert_eq!(GEO.cell_at(0.0, 0.0, 0), (point(0, 0), Side::Left));
        assert_eq!(GEO.cell_at(-50.0, -50.0, 0), (point(0, 0), Side::Left));
        assert_eq!(GEO.cell_at(5000.0, 5000.0, 0), (point(23, 79), Side::Right));
    }

    #[test]
    fn scrolled_view_gives_history_lines() {
        // The view is 30 lines up: the top row is line -30.
        assert_eq!(GEO.cell_at(7.0, 6.0, 30).0, point(-30, 0));
        assert_eq!(GEO.cell_at(7.0, 46.0, 30).0, point(-28, 0));
    }

    #[test]
    fn no_autoscroll_inside_the_window() {
        assert_eq!(autoscroll_lines(5.0, 500.0), 0);
        assert_eq!(autoscroll_lines(250.0, 500.0), 0);
        assert_eq!(autoscroll_lines(495.0, 500.0), 0);
    }

    #[test]
    fn autoscroll_up_above_and_down_below() {
        assert!(autoscroll_lines(-1.0, 500.0) > 0);
        assert!(autoscroll_lines(501.0, 500.0) < 0);
    }

    #[test]
    fn autoscroll_is_faster_when_farther() {
        let near = autoscroll_lines(-5.0, 500.0);
        let far = autoscroll_lines(-100.0, 500.0);
        assert!(near >= 1 && far > near, "{near} {far}");
        assert_eq!(autoscroll_lines(-10_000.0, 500.0), MAX_AUTOSCROLL);
        assert_eq!(autoscroll_lines(10_000.0, 500.0), -MAX_AUTOSCROLL);
    }

    #[test]
    fn double_and_triple_click() {
        let mut clicks = ClickCounter::default();
        let t = Instant::now();
        let p = point(1, 1);
        assert_eq!(clicks.click(t, p), 1);
        assert_eq!(clicks.click(t + Duration::from_millis(100), p), 2);
        assert_eq!(clicks.click(t + Duration::from_millis(200), p), 3);
        // After a triple click, it starts again.
        assert_eq!(clicks.click(t + Duration::from_millis(300), p), 1);
    }

    #[test]
    fn slow_click_or_other_cell_is_a_single_click() {
        let mut clicks = ClickCounter::default();
        let t = Instant::now();
        assert_eq!(clicks.click(t, point(1, 1)), 1);
        assert_eq!(clicks.click(t + Duration::from_millis(900), point(1, 1)), 1);
        assert_eq!(clicks.click(t + Duration::from_millis(950), point(1, 2)), 1);
    }

    const NO_MODS: ReportMods = ReportMods {
        shift: false,
        alt: false,
        ctrl: false,
    };

    fn sgr(kind: ReportKind, col: usize, row: usize, mods: ReportMods) -> String {
        String::from_utf8(encode_mouse(kind, col, row, mods, true).unwrap()).unwrap()
    }

    #[test]
    fn sgr_press_and_release() {
        use ReportButton::*;
        // Columns and rows start at 1 in the protocol.
        assert_eq!(sgr(ReportKind::Press(Left), 0, 0, NO_MODS), "\x1b[<0;1;1M");
        assert_eq!(
            sgr(ReportKind::Press(Middle), 4, 9, NO_MODS),
            "\x1b[<1;5;10M"
        );
        assert_eq!(
            sgr(ReportKind::Press(Right), 4, 9, NO_MODS),
            "\x1b[<2;5;10M"
        );
        assert_eq!(
            sgr(ReportKind::Release(Left), 4, 9, NO_MODS),
            "\x1b[<0;5;10m"
        );
    }

    #[test]
    fn sgr_motion_wheel_and_modifiers() {
        assert_eq!(
            sgr(ReportKind::Motion(Some(ReportButton::Left)), 2, 3, NO_MODS),
            "\x1b[<32;3;4M"
        );
        assert_eq!(
            sgr(ReportKind::Motion(None), 2, 3, NO_MODS),
            "\x1b[<35;3;4M"
        );
        assert_eq!(sgr(ReportKind::WheelUp, 0, 0, NO_MODS), "\x1b[<64;1;1M");
        assert_eq!(sgr(ReportKind::WheelDown, 0, 0, NO_MODS), "\x1b[<65;1;1M");
        let all = ReportMods {
            shift: true,
            alt: true,
            ctrl: true,
        };
        // shift +4, alt +8, ctrl +16.
        assert_eq!(
            sgr(ReportKind::Press(ReportButton::Left), 0, 0, all),
            "\x1b[<28;1;1M"
        );
    }

    #[test]
    fn x10_form_without_sgr() {
        let bytes = encode_mouse(ReportKind::Press(ReportButton::Left), 0, 0, NO_MODS, false);
        assert_eq!(bytes, Some(vec![0x1b, b'[', b'M', 32, 33, 33]));
        // Release in X10 is button 3.
        let bytes = encode_mouse(
            ReportKind::Release(ReportButton::Right),
            1,
            1,
            NO_MODS,
            false,
        );
        assert_eq!(bytes, Some(vec![0x1b, b'[', b'M', 32 + 3, 34, 34]));
        // X10 cannot send cells after 223.
        assert_eq!(
            encode_mouse(
                ReportKind::Press(ReportButton::Left),
                300,
                0,
                NO_MODS,
                false
            ),
            None
        );
    }
}
