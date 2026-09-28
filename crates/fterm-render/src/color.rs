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

#[cfg(test)]
mod tests {
    use super::*;

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
