//! Writes the fterm icon files from `fterm_render::icon`.
//!
//! `cargo run -p fterm-render --example make_icon -- preview DIR` writes a sheet of all sizes.
//! `cargo run -p fterm-render --example make_icon -- DIR` writes fterm.ico (16..256) and fterm-256.png.

use std::path::Path;

use fterm_render::icon::rgba;
use image::{ImageBuffer, Rgba, RgbaImage};

const SIZES: [u32; 6] = [256, 64, 48, 32, 24, 16];

fn icon(size: u32) -> RgbaImage {
    ImageBuffer::from_raw(size, size, rgba(size)).expect("the size is right")
}

/// All sizes, on a dark and on a light background (like a taskbar and a folder).
fn preview(dir: &Path) {
    let width: u32 = SIZES.iter().map(|s| s + 16).sum::<u32>() + 16;
    let row = 256 + 32;
    let mut sheet = RgbaImage::new(width, row * 2);
    for (j, back) in [[32u8, 32, 32, 255], [243, 243, 243, 255]]
        .iter()
        .enumerate()
    {
        let top = j as u32 * row;
        for y in top..top + row {
            for x in 0..width {
                sheet.put_pixel(x, y, Rgba(*back));
            }
        }
        let mut x = 16;
        for size in SIZES {
            image::imageops::overlay(&mut sheet, &icon(size), x as i64, (top + 16) as i64);
            x += size + 16;
        }
    }
    let path = dir.join("icon-preview.png");
    sheet.save(&path).expect("cannot write the preview");
    println!("{}", path.display());
}

/// The .ico (all sizes, as PNG frames) and a big PNG.
fn files(dir: &Path) {
    let frames: Vec<image::codecs::ico::IcoFrame> = SIZES
        .iter()
        .map(|&size| {
            image::codecs::ico::IcoFrame::as_png(
                &rgba(size),
                size,
                size,
                image::ExtendedColorType::Rgba8,
            )
            .expect("cannot make an ico frame")
        })
        .collect();
    let file = std::fs::File::create(dir.join("fterm.ico")).expect("cannot write fterm.ico");
    image::codecs::ico::IcoEncoder::new(file)
        .encode_images(&frames)
        .expect("cannot write fterm.ico");
    icon(256)
        .save(dir.join("fterm-256.png"))
        .expect("cannot write fterm-256.png");
    println!("{}", dir.join("fterm.ico").display());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [mode, dir] if mode == "preview" => preview(Path::new(dir)),
        [dir] => files(Path::new(dir)),
        _ => eprintln!("usage: make_icon [preview] DIR"),
    }
}
