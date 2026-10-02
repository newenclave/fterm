//! The fterm icon: a bold letter f made of Braille dots (a grid of 3x2 cells, so 6x8 dots), on a dark
//! rounded square. It is drawn here in any size: the window icon, and the .ico of the program.

/// The dots of the f: 6 columns, 8 rows (`#` = on).
pub const LETTER: [&str; 8] = [
    "..####", //
    ".##...", //
    ".##...", //
    "#####.", //
    ".##...", //
    ".##...", //
    ".##...", //
    ".##...", //
];

/// Where the dots are: the top left corner of the grid and the step from one dot to the next.
/// The grid of 6x8 dots is in the middle of the icon.
fn layout(size: u32) -> (f32, f32, f32) {
    let size = size as f32;
    let step = size * 0.095;
    ((size - 6.0 * step) / 2.0, (size - 8.0 * step) / 2.0, step)
}

/// The colors (the Catppuccin Mocha palette of fterm).
const BACKGROUND: [f32; 3] = [30.0, 30.0, 46.0];
const BORDER: [f32; 3] = [49.0, 50.0, 68.0];
const ACCENT: [f32; 3] = [203.0, 166.0, 247.0];

/// Samples per pixel in each direction (for smooth edges).
const SAMPLES: u32 = 4;

/// The icon as RGBA pixels (`size` x `size`, rows from the top).
pub fn rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let radius = s * 0.22;
    let border = (s / 48.0).max(1.0);
    let (gx, gy, step) = layout(size);
    // Small icons: the dots touch, so the f stays a clear shape.
    let gap = if size < 32 { 0.0 } else { step * 0.16 };
    // What is at a point: None = outside the rounded square.
    let sample = |x: f32, y: f32| -> Option<[f32; 3]> {
        // The rounded square: the distance to the inner rectangle.
        let dx = (radius - x).max(x - (s - radius)).max(0.0);
        let dy = (radius - y).max(y - (s - radius)).max(0.0);
        let out = (dx * dx + dy * dy).sqrt();
        if out > radius {
            return None;
        }
        let (cx, cy) = ((x - gx) / step, (y - gy) / step);
        if cx >= 0.0 && cy >= 0.0 && cx < 6.0 && cy < 8.0 {
            let (col, row) = (cx as usize, cy as usize);
            let (fx, fy) = ((x - gx) - col as f32 * step, (y - gy) - row as f32 * step);
            let inside = fx >= gap / 2.0
                && fx < step - gap / 2.0
                && fy >= gap / 2.0
                && fy < step - gap / 2.0;
            if inside && LETTER[row].as_bytes()[col] == b'#' {
                return Some(ACCENT);
            }
        }
        if radius - out < border {
            Some(BORDER)
        } else {
            Some(BACKGROUND)
        }
    };
    let mut image = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for px in 0..size {
            let mut sum = [0.0f32; 3];
            let mut hits = 0u32;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = px as f32 + (sx as f32 + 0.5) / SAMPLES as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SAMPLES as f32;
                    if let Some(c) = sample(x, y) {
                        for i in 0..3 {
                            sum[i] += c[i];
                        }
                        hits += 1;
                    }
                }
            }
            let i = ((py * size + px) * 4) as usize;
            if hits > 0 {
                for c in 0..3 {
                    image[i + c] = (sum[c] / hits as f32).round() as u8;
                }
                image[i + 3] = (255 * hits / (SAMPLES * SAMPLES)) as u8;
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(image: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [image[i], image[i + 1], image[i + 2], image[i + 3]]
    }

    /// The center of the dot in column `col`, row `row` (the same layout as `rgba`).
    fn dot_center(size: u32, col: u32, row: u32) -> (u32, u32) {
        let (x, y, step) = layout(size);
        (
            (x + step * (col as f32 + 0.5)) as u32,
            (y + step * (row as f32 + 0.5)) as u32,
        )
    }

    #[test]
    fn the_icon_has_its_size_and_round_corners() {
        for size in [16, 24, 32, 48, 64, 256] {
            let image = rgba(size);
            assert_eq!(image.len(), (size * size * 4) as usize);
            assert_eq!(
                pixel(&image, size, 0, 0)[3],
                0,
                "a corner is clear at {size}"
            );
            assert_eq!(
                pixel(&image, size, size / 2, 1)[3],
                255,
                "the edge is full at {size}"
            );
        }
    }

    #[test]
    fn the_dots_of_the_f_are_bright() {
        for size in [16, 32, 256] {
            let image = rgba(size);
            let (x, y) = dot_center(size, 1, 4);
            let [r, g, b, _] = pixel(&image, size, x, y);
            assert!(r > 150 && b > 150, "an f dot at {size}: {r},{g},{b}");
            // A dot that is not in the f is the background.
            let (x, y) = dot_center(size, 5, 7);
            let [r, _, b, _] = pixel(&image, size, x, y);
            assert!(r < 60 && b < 80, "the background at {size}: {r},{b}");
        }
    }
}
