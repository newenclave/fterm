//! Grid size: how many columns and rows fit in the window.

use alacritty_terminal::grid::Dimensions;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub columns: usize,
    pub rows: usize,
}

impl GridSize {
    pub fn new(columns: usize, rows: usize) -> Self {
        Self { columns, rows }
    }

    /// Fits cells of `cell_width` x `cell_height` into the window, with `padding` on every side.
    /// The result is at least 1x1.
    pub fn from_pixels(
        width: u32,
        height: u32,
        cell_width: f32,
        cell_height: f32,
        padding: f32,
    ) -> Self {
        let fit = |pixels: u32, cell: f32| {
            let space = (pixels as f32 - 2.0 * padding).max(0.0);
            ((space / cell).floor() as usize).max(1)
        };
        Self::new(fit(width, cell_width), fit(height, cell_height))
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_fit_in_the_window() {
        assert_eq!(
            GridSize::from_pixels(800, 600, 10.0, 20.0, 0.0),
            GridSize::new(80, 30)
        );
    }

    #[test]
    fn padding_takes_space_on_both_sides() {
        // 800 - 2*5 = 790 -> 79 columns; 600 - 2*5 = 590 -> 29 rows.
        assert_eq!(
            GridSize::from_pixels(800, 600, 10.0, 20.0, 5.0),
            GridSize::new(79, 29)
        );
    }

    #[test]
    fn part_of_a_cell_does_not_count() {
        assert_eq!(
            GridSize::from_pixels(809, 619, 10.0, 20.0, 0.0),
            GridSize::new(80, 30)
        );
    }

    #[test]
    fn tiny_window_still_has_one_cell() {
        assert_eq!(
            GridSize::from_pixels(3, 3, 10.0, 20.0, 5.0),
            GridSize::new(1, 1)
        );
        assert_eq!(
            GridSize::from_pixels(0, 0, 10.0, 20.0, 0.0),
            GridSize::new(1, 1)
        );
    }
}
