//! Pictures in the output: the iTerm2 sequence `OSC 1337 ; File=<args> : <base64> BEL`. This file reads the args,
//! decodes the picture to RGBA, and finds how many cells it takes. No GPU code.

use std::sync::Arc;

/// The most base64 text of one picture (about 24 MiB of file). A longer sequence is dropped.
pub const MAX_PAYLOAD: usize = 32 * 1024 * 1024;
/// The most pixels of a picture before it is decoded (a bad header can say 100000×100000).
pub const MAX_PIXELS: u64 = 40_000_000;
/// The longest side after decoding: a bigger picture is scaled down (the GPU limit is often 8192).
pub const MAX_SIDE: u32 = 4096;

/// A width or a height in the args.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dim {
    /// From the picture (and at most the pane).
    Auto,
    Cells(u32),
    Px(u32),
    /// Of the pane.
    Percent(u32),
}

/// The args of `File=`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileArgs {
    pub name: Option<String>,
    /// `inline=1`: show it. Without it the file is a download, and fterm does not take downloads.
    pub inline: bool,
    pub width: Dim,
    pub height: Dim,
    /// `preserveAspectRatio` (default yes).
    pub keep_aspect: bool,
    /// `ftermRoom=1` (ours): the program makes room for the picture itself (new lines), so the cursor stays.
    pub own_room: bool,
}

/// A decoded picture: RGBA8, `width * height * 4` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// Reads `File=<args>:<base64>` (the text after `1337;`). `None` when it is not a `File=` sequence.
pub fn parse_file(text: &[u8]) -> Option<(FileArgs, &[u8])> {
    let rest = text.strip_prefix(b"File=")?;
    let colon = rest.iter().position(|&b| b == b':')?;
    let (args, data) = (&rest[..colon], &rest[colon + 1..]);
    let mut out = FileArgs {
        name: None,
        inline: false,
        width: Dim::Auto,
        height: Dim::Auto,
        keep_aspect: true,
        own_room: false,
    };
    for arg in String::from_utf8_lossy(args).split(';') {
        let Some((key, value)) = arg.split_once('=') else {
            continue;
        };
        match key {
            "name" => {
                out.name = base64_decode(value.as_bytes())
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
            }
            "inline" => out.inline = value == "1",
            "width" => out.width = dim(value),
            "height" => out.height = dim(value),
            "preserveAspectRatio" => out.keep_aspect = value != "0",
            "ftermRoom" => out.own_room = value == "1",
            _ => {}
        }
    }
    Some((out, data))
}

fn dim(value: &str) -> Dim {
    let number = |s: &str| s.trim().parse::<u32>().ok().filter(|&n| n > 0);
    if value == "auto" {
        Dim::Auto
    } else if let Some(px) = value.strip_suffix("px") {
        number(px).map_or(Dim::Auto, Dim::Px)
    } else if let Some(percent) = value.strip_suffix('%') {
        number(percent).map_or(Dim::Auto, |p| Dim::Percent(p.min(100)))
    } else {
        number(value).map_or(Dim::Auto, Dim::Cells)
    }
}

/// Standard base64 (with or without `=`, white space is skipped). `None` on a bad char.
pub fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for &b in text {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Decodes a PNG, JPEG, or BMP file. A picture with too many pixels is refused before it is decoded; one with a
/// side longer than `max_side` is scaled down.
pub fn decode(bytes: &[u8], max_side: u32) -> Result<Picture, String> {
    use image::ImageReader;
    let reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let Some(format) = reader.format() else {
        return Err("not a picture (PNG, JPEG, or BMP)".into());
    };
    let mut reader = reader;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 4 + 64 * 1024 * 1024);
    reader.limits(limits);
    let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(format!("too big: {w}×{h} pixels"));
    }
    let image = image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())?;
    let image = if w.max(h) > max_side {
        image.resize(max_side, max_side, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.into_rgba8();
    Ok(Picture {
        width: rgba.width(),
        height: rgba.height(),
        rgba: Arc::new(rgba.into_raw()),
    })
}

/// A picture from raw RGBA8 pixels (the clipboard, a screenshot), scaled down when a side is longer than `max_side`.
pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>, max_side: u32) -> Result<Picture, String> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!("a bad size: {width}×{height} pixels"));
    }
    let image = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| format!("the pixels do not fit {width}×{height}"))?;
    let image = if width.max(height) > max_side {
        image::DynamicImage::ImageRgba8(image)
            .resize(max_side, max_side, image::imageops::FilterType::Triangle)
            .into_rgba8()
    } else {
        image
    };
    Ok(Picture {
        width: image.width(),
        height: image.height(),
        rgba: Arc::new(image.into_raw()),
    })
}

/// How many cells (columns, rows) the picture takes. `cell` is the cell size in pixels, `pane` the pane size in
/// cells. A picture is never wider than the pane; its height may be more than the pane (it scrolls).
pub fn cells(
    args: &FileArgs,
    picture: (u32, u32),
    cell: (f32, f32),
    pane: (usize, usize),
) -> (usize, usize) {
    let (cw, ch) = (cell.0.max(1.0), cell.1.max(1.0));
    let (pw, ph) = (picture.0.max(1) as f32, picture.1.max(1) as f32);
    // In pixels.
    let want = |d: Dim, cell: f32, pane: usize| match d {
        Dim::Auto => None,
        Dim::Cells(n) => Some(n as f32 * cell),
        Dim::Px(n) => Some(n as f32),
        Dim::Percent(p) => Some(pane as f32 * cell * p as f32 / 100.0),
    };
    let max_w = pane.0.max(1) as f32 * cw;
    let (w, h) = match (want(args.width, cw, pane.0), want(args.height, ch, pane.1)) {
        (None, None) => (pw, ph),
        (Some(w), None) => (w, w * ph / pw),
        (None, Some(h)) => (h * pw / ph, h),
        (Some(w), Some(h)) if args.keep_aspect => {
            let scale = (w / pw).min(h / ph);
            (pw * scale, ph * scale)
        }
        (Some(w), Some(h)) => (w, h),
    };
    // Too wide: smaller, and with the same shape.
    let (w, h) = if w > max_w {
        (max_w, if args.keep_aspect { h * max_w / w } else { h })
    } else {
        (w, h)
    };
    (
        ((w / cw).ceil() as usize).clamp(1, pane.0.max(1)),
        ((h / ch).ceil() as usize).max(1),
    )
}

/// The cells of a picture that fills a pane of `pane` cells as much as it can, with its shape (a picture pane).
pub fn fit(picture: (u32, u32), cell: (f32, f32), pane: (usize, usize)) -> (usize, usize) {
    let args = FileArgs {
        name: None,
        inline: true,
        width: Dim::Percent(100),
        height: Dim::Percent(100),
        keep_aspect: true,
        own_room: false,
    };
    cells(&args, picture, cell, pane)
}

/// The pictures whose top-left cell is on the grid lines `lines` (screen lines: negative = the history):
/// (id, line, column). Lines out of the grid are skipped.
pub fn marks<T>(
    term: &alacritty_terminal::term::Term<T>,
    lines: std::ops::Range<i32>,
) -> Vec<(u32, i32, usize)> {
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line};
    let grid = term.grid();
    let top = -(grid.history_size() as i32);
    let bottom = grid.screen_lines() as i32;
    let mut out = Vec::new();
    for line in lines.start.max(top)..lines.end.min(bottom) {
        let row = &grid[Line(line)];
        for column in 0..grid.columns() {
            if let Some(link) = row[Column(column)].hyperlink()
                && let Some(id) = crate::osc::picture_id(link.uri())
            {
                out.push((id, line, column));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(format: image::ImageFormat, w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x * 10) as u8, (y * 10) as u8, 200])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    fn b64(bytes: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for chunk in bytes.chunks(3) {
            let n =
                chunk.iter().fold(0u32, |a, &b| (a << 8) | u32::from(b)) << (8 * (3 - chunk.len()));
            for i in 0..4 {
                if i <= chunk.len() {
                    s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }

    #[test]
    fn the_args_of_a_file() {
        let text = format!(
            "File=name={};size=10;width=20;height=50%;inline=1:QUJD",
            b64(b"cat.png")
        );
        let (args, data) = parse_file(text.as_bytes()).unwrap();
        assert_eq!(args.name.as_deref(), Some("cat.png"));
        assert!(args.inline);
        assert_eq!(args.width, Dim::Cells(20));
        assert_eq!(args.height, Dim::Percent(50));
        assert!(args.keep_aspect);
        assert_eq!(data, b"QUJD");
        let (args, _) = parse_file(b"File=width=300px;height=auto;preserveAspectRatio=0:").unwrap();
        assert_eq!((args.width, args.height), (Dim::Px(300), Dim::Auto));
        assert!(!args.keep_aspect && !args.inline, "no inline=1: a download");
        let (args, _) = parse_file(b"File=inline=0:").unwrap();
        assert!(!args.inline);
        // Bad values are `auto`; other 1337 sequences are not files.
        let (args, _) = parse_file(b"File=width=-3;height=x%;inline=1:").unwrap();
        assert_eq!((args.width, args.height), (Dim::Auto, Dim::Auto));
        assert_eq!(parse_file(b"SetMark"), None);
        assert_eq!(parse_file(b"File=inline=1"), None, "no `:` and no data");
    }

    #[test]
    fn base64_with_and_without_padding() {
        assert_eq!(base64_decode(b"QUJD").unwrap(), b"ABC");
        assert_eq!(base64_decode(b"QUI=").unwrap(), b"AB");
        assert_eq!(base64_decode(b"QUI").unwrap(), b"AB");
        assert_eq!(base64_decode(b"QQ==\r\n").unwrap(), b"A");
        assert_eq!(
            base64_decode(b"QU\nJD").unwrap(),
            b"ABC",
            "lines are skipped"
        );
        assert_eq!(base64_decode(b"QU*D"), None);
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(base64_decode(b64(&all).as_bytes()).unwrap(), all);
    }

    #[test]
    fn png_jpeg_and_bmp_decode() {
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::Bmp,
        ] {
            let picture = decode(&encode(format, 7, 3), MAX_SIDE).unwrap();
            assert_eq!((picture.width, picture.height), (7, 3), "{format:?}");
            assert_eq!(picture.rgba.len(), 7 * 3 * 4);
            assert_eq!(picture.rgba[3], 255, "alpha");
        }
    }

    #[test]
    fn raw_pixels_become_a_picture() {
        let picture = from_rgba(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8], MAX_SIDE).unwrap();
        assert_eq!((picture.width, picture.height), (2, 1));
        assert_eq!(*picture.rgba, [1, 2, 3, 4, 5, 6, 7, 8]);
        // Too big a side: scaled down with its shape.
        let picture = from_rgba(400, 100, vec![255; 400 * 100 * 4], 200).unwrap();
        assert_eq!((picture.width, picture.height), (200, 50));
        // The bytes must fit the size.
        assert!(from_rgba(2, 2, vec![0; 3], MAX_SIDE).is_err());
        assert!(from_rgba(0, 2, Vec::new(), MAX_SIDE).is_err());
    }

    #[test]
    fn a_big_picture_is_scaled_down() {
        let picture = decode(&encode(image::ImageFormat::Png, 400, 100), 200).unwrap();
        assert_eq!((picture.width, picture.height), (200, 50));
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(decode(b"hello, not a picture", MAX_SIDE).is_err());
        assert!(decode(b"", MAX_SIDE).is_err());
        // A PNG header that says 100000×100000: refused before the pixels are decoded.
        let mut png = encode(image::ImageFormat::Png, 1, 1);
        png[16..24].copy_from_slice(&[0, 1, 0x86, 0xA0, 0, 1, 0x86, 0xA0]);
        // The CRC of the IHDR chunk (its type and data) must fit, or the decoder stops at the CRC.
        let crc = crc32(&png[12..29]);
        png[29..33].copy_from_slice(&crc.to_be_bytes());
        let err = decode(&png, MAX_SIDE).unwrap_err();
        assert!(err.contains("too big") || err.contains("limit"), "{err}");
        // A cut file.
        let png = encode(image::ImageFormat::Png, 20, 20);
        assert!(decode(&png[..png.len() / 2], MAX_SIDE).is_err());
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn args(width: Dim, height: Dim) -> FileArgs {
        FileArgs {
            name: None,
            inline: true,
            width,
            height,
            keep_aspect: true,
            own_room: false,
        }
    }

    #[test]
    fn the_cells_of_a_picture() {
        let cell = (10.0, 20.0);
        let pane = (80, 24);
        // Its own size: 100×100 px = 10 columns, 5 rows.
        assert_eq!(
            cells(&args(Dim::Auto, Dim::Auto), (100, 100), cell, pane),
            (10, 5)
        );
        // A part of a cell is a whole cell.
        assert_eq!(
            cells(&args(Dim::Auto, Dim::Auto), (101, 41), cell, pane),
            (11, 3)
        );
        // A width in cells: the height keeps the shape (200 px wide -> 100 px high).
        assert_eq!(
            cells(&args(Dim::Cells(20), Dim::Auto), (100, 50), cell, pane),
            (20, 5)
        );
        // A height in pixels.
        assert_eq!(
            cells(&args(Dim::Auto, Dim::Px(60)), (100, 100), cell, pane),
            (6, 3)
        );
        // 50% of the pane width (400 px).
        assert_eq!(
            cells(&args(Dim::Percent(50), Dim::Auto), (100, 100), cell, pane),
            (40, 20)
        );
        // Both: it fits in the box and keeps its shape...
        assert_eq!(
            cells(&args(Dim::Cells(40), Dim::Cells(5)), (100, 100), cell, pane),
            (10, 5)
        );
        // ...or fills it with preserveAspectRatio=0.
        let mut stretch = args(Dim::Cells(40), Dim::Cells(5));
        stretch.keep_aspect = false;
        assert_eq!(cells(&stretch, (100, 100), cell, pane), (40, 5));
    }

    #[test]
    fn a_picture_fits_a_pane() {
        let cell = (10.0, 20.0);
        // A wide picture: the pane width (80 cols = 800 px) -> 400 px high = 20 rows.
        assert_eq!(fit((2000, 1000), cell, (80, 24)), (80, 20));
        // A high picture: the pane height (24 rows = 480 px) -> 480 px wide = 48 cols.
        assert_eq!(fit((1000, 1000), cell, (80, 24)), (48, 24));
        // A small picture grows to the pane too.
        assert_eq!(fit((10, 10), cell, (80, 24)), (48, 24));
    }

    #[test]
    fn a_picture_is_never_wider_than_the_pane() {
        // 2000×1000 px in a 80-column pane (800 px): 80 columns, 400 px high = 20 rows.
        assert_eq!(
            cells(
                &args(Dim::Auto, Dim::Auto),
                (2000, 1000),
                (10.0, 20.0),
                (80, 24)
            ),
            (80, 20)
        );
        assert_eq!(
            cells(
                &args(Dim::Cells(500), Dim::Auto),
                (100, 100),
                (10.0, 20.0),
                (80, 24)
            ),
            (80, 40),
            "it may be higher than the pane: it scrolls"
        );
    }
}
