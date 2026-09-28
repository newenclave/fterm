//! Color helpers for the GPU. The surface is sRGB, so colors go to the GPU as linear values.

use fterm_term::alacritty_terminal::vte::ansi::Rgb;

/// Converts one sRGB color channel (0..=255) to a linear value (0.0..=1.0).
pub fn srgb_to_linear(channel: u8) -> f32 {
    let c = f32::from(channel) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// A color as linear RGBA for the shader.
pub fn linear(rgb: Rgb) -> [f32; 4] {
    [
        srgb_to_linear(rgb.r),
        srgb_to_linear(rgb.g),
        srgb_to_linear(rgb.b),
        1.0,
    ]
}

/// Makes glyph edges a bit stronger. Blending in linear space makes light text on a dark
/// background look thin, and this fixes it. `shader.wgsl` uses the same formula.
pub fn text_alpha(alpha: f32) -> f32 {
    alpha.clamp(0.0, 1.0).powf(1.0 / TEXT_GAMMA)
}

/// The gamma for `text_alpha`. Keep it the same as in `shader.wgsl`.
pub const TEXT_GAMMA: f32 = 1.45;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_alpha_keeps_the_ends() {
        assert_eq!(text_alpha(0.0), 0.0);
        assert!((text_alpha(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn text_alpha_makes_edges_stronger() {
        for a in [0.1, 0.25, 0.5, 0.75, 0.9] {
            assert!(text_alpha(a) > a, "{a}");
        }
        // alpha^(1/1.45): 0.5 -> about 0.62.
        assert!((text_alpha(0.5) - 0.62).abs() < 0.01);
    }

    #[test]
    fn text_alpha_keeps_the_order() {
        let values: Vec<f32> = (0..=10).map(|i| text_alpha(i as f32 / 10.0)).collect();
        assert!(values.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn srgb_to_linear_known_values() {
        assert_eq!(srgb_to_linear(0), 0.0);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);
        // sRGB 128 is about 0.2159 in linear space.
        assert!((srgb_to_linear(128) - 0.2159).abs() < 1e-3);
        // Small values use the linear part of the curve: 10 / 255 / 12.92.
        assert!((srgb_to_linear(10) - 10.0 / 255.0 / 12.92).abs() < 1e-6);
    }

    #[test]
    fn linear_color_is_opaque() {
        let c = linear(Rgb {
            r: 255,
            g: 0,
            b: 128,
        });
        assert!((c[0] - 1.0).abs() < 1e-6);
        assert_eq!(c[1], 0.0);
        assert!((c[2] - srgb_to_linear(128)).abs() < 1e-6);
        assert_eq!(c[3], 1.0);
    }
}
