//! A screenshot of a pane: the frame drawn again into a texture, cut to the pane, and saved as PNG.

use std::path::Path;

use anyhow::{Context, Result};

/// Takes `area` of the frame that `draw` draws, and writes it to `path`. Gives the size in pixels.
pub fn take(
    gpu: &mut crate::gpu::Gpu,
    area: (u32, u32, u32, u32),
    draw: impl FnMut(&wgpu::Device, &wgpu::Queue, &wgpu::TextureView, (u32, u32)),
    path: &Path,
) -> Result<(u32, u32)> {
    let shot = gpu.capture(area, draw)?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot make {}", dir.display()))?;
    }
    image::save_buffer_with_format(
        path,
        &shot.rgba,
        shot.width,
        shot.height,
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .with_context(|| format!("cannot write {}", path.display()))?;
    Ok((shot.width, shot.height))
}
