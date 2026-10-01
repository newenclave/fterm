//! Colors in the config: `#rrggbb` or `#rgb`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Colors from the config. `None` = use the built-in color.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorConfig {
    pub background: Option<Rgb>,
    pub foreground: Option<Rgb>,
    pub cursor: Option<Rgb>,
    pub selection: Option<Rgb>,
    /// The 8 normal colors (black, red, green, yellow, blue, magenta, cyan, white).
    pub ansi: [Option<Rgb>; 8],
    /// The 8 bright colors.
    pub bright: [Option<Rgb>; 8],
}

/// `#rrggbb` or `#rgb` (the `#` is needed).
pub fn parse_color(text: &str) -> Result<Rgb, String> {
    let bad = || format!("`{text}` is not a color: use #rrggbb or #rgb");
    let hex = text.strip_prefix('#').ok_or_else(bad)?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let digit = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).map_err(|_| bad());
    let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
    match hex.len() {
        6 => Ok(Rgb {
            r: pair(0)?,
            g: pair(2)?,
            b: pair(4)?,
        }),
        // #rgb: each digit is used twice (#fa0 = #ffaa00).
        3 => Ok(Rgb {
            r: digit(0)? * 17,
            g: digit(1)? * 17,
            b: digit(2)? * 17,
        }),
        _ => Err(bad()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_form() {
        assert_eq!(
            parse_color("#1e1e2e"),
            Ok(Rgb {
                r: 0x1e,
                g: 0x1e,
                b: 0x2e
            })
        );
        assert_eq!(
            parse_color("#FFaa00"),
            Ok(Rgb {
                r: 255,
                g: 0xaa,
                b: 0
            })
        );
    }

    #[test]
    fn short_form() {
        assert_eq!(
            parse_color("#fa0"),
            Ok(Rgb {
                r: 0xff,
                g: 0xaa,
                b: 0
            })
        );
    }

    #[test]
    fn bad_colors() {
        for bad in ["1e1e2e", "#12345", "#gggggg", "", "#", "red"] {
            let err = parse_color(bad).unwrap_err();
            assert!(err.contains("color"), "{bad}: {err}");
        }
    }
}
