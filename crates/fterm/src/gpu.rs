//! GPU state: the wgpu surface, device, and drawing of one frame.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use winit::dpi::PhysicalSize;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

pub struct Gpu {
    instance: wgpu::Instance,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
}

impl Gpu {
    /// The GPU of the config (`gpu.backend`, `gpu.power`). When a backend does not start (for example a
    /// weak OpenGL in a VM), the next one is tried. WGPU_BACKEND (dx12, vulkan, gl, metal) wins over all.
    pub async fn new(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        config: fterm_config::load::GpuConfig,
    ) -> Result<Self> {
        let power = match config.power {
            fterm_config::load::GpuPower::High => wgpu::PowerPreference::HighPerformance,
            fterm_config::load::GpuPower::Low => wgpu::PowerPreference::LowPower,
        };
        let tries: Vec<Option<wgpu::Backends>> = if std::env::var_os("WGPU_BACKEND").is_some() {
            vec![None]
        } else {
            backend_tries(config.backend, std::env::consts::OS)
                .into_iter()
                .map(Some)
                .collect()
        };
        let mut last = None;
        for backends in tries {
            match Self::with_backends(window.clone(), display.clone(), backends, power).await {
                Ok(gpu) => return Ok(gpu),
                Err(err) => {
                    tracing::warn!(?backends, "this GPU backend does not start: {err:#}");
                    last = Some(err);
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("no GPU backend")))
    }

    /// One try: `backends` (`None` = from WGPU_BACKEND).
    async fn with_backends(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        backends: Option<wgpu::Backends>,
        power: wgpu::PowerPreference,
    ) -> Result<Self> {
        let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
        let desc = match backends {
            Some(backends) => {
                desc.backends = backends;
                desc
            }
            None => desc.with_env(),
        };
        let instance = wgpu::Instance::new(desc);
        let surface = instance
            .create_surface(window.clone())
            .context("cannot create the GPU surface")?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: power,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .context("no GPU adapter can draw to this window")?;
        let info = adapter.get_info();
        tracing::info!(adapter = %info.name, backend = ?info.backend, "GPU adapter");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("fterm device"),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            })
            .await
            .context("cannot create the GPU device")?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| anyhow!("the surface has no texture formats"))?;
        let (width, height) = surface_size(window.inner_size()).unwrap_or((1, 1));
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        tracing::debug!(?format, width, height, "surface configured");

        Ok(Self {
            instance,
            window,
            surface,
            device,
            queue,
            config,
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        let Some((width, height)) = surface_size(size) else {
            return;
        };
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Gets the next frame, lets `draw` fill it, and shows it.
    /// An error here means the GPU cannot work any more.
    pub fn frame(
        &mut self,
        mut draw: impl FnMut(&wgpu::Device, &wgpu::Queue, &wgpu::TextureView, (u32, u32)),
    ) -> Result<()> {
        let (frame, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            // Draw this frame, then set up the surface again after `present`.
            // wgpu does not allow `configure` while we hold a frame.
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                tracing::warn!("GPU surface lost, making a new one");
                self.surface = self
                    .instance
                    .create_surface(self.window.clone())
                    .context("cannot create the GPU surface again")?;
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(anyhow!("GPU validation error while getting the next frame"));
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        draw(
            &self.device,
            &self.queue,
            &view,
            (self.config.width, self.config.height),
        );
        self.window.pre_present_notify();
        self.queue.present(frame);
        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(())
    }
}

/// A rect of the window in whole pixels: from the pixel where it starts to the one where it ends.
pub fn pixel_area(rect: fterm_mux::Rect) -> (u32, u32, u32, u32) {
    let x0 = rect.x.max(0.0).floor();
    let y0 = rect.y.max(0.0).floor();
    let x1 = (rect.x + rect.width).max(0.0).ceil();
    let y1 = (rect.y + rect.height).max(0.0).ceil();
    (
        x0 as u32,
        y0 as u32,
        (x1 - x0).max(0.0) as u32,
        (y1 - y0).max(0.0) as u32,
    )
}

/// The bytes of one row in a texture-to-buffer copy: a multiple of 256.
pub fn padded_bytes_per_row(width: u32) -> u32 {
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    (width * 4).div_ceil(align) * align
}

/// The pixels of a copied texture as RGBA rows with no padding.
pub fn unpad(data: &[u8], width: u32, height: u32, padded: u32, bgra: bool) -> Vec<u8> {
    let row = (width * 4) as usize;
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        let start = y * padded as usize;
        out.extend_from_slice(&data[start..start + row]);
    }
    if bgra {
        for pixel in out.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    out
}

/// A picture of a part of the window: RGBA pixels, rows from the top.
pub struct Shot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Gpu {
    /// Draws a frame with `draw` into a texture of its own (not the window), and gives the pixels of
    /// `area` (x, y, width, height; cut to the window). It works when the window is covered too.
    pub fn capture(
        &mut self,
        area: (u32, u32, u32, u32),
        mut draw: impl FnMut(&wgpu::Device, &wgpu::Queue, &wgpu::TextureView, (u32, u32)),
    ) -> Result<Shot> {
        let (fw, fh) = (self.config.width, self.config.height);
        let x = area.0.min(fw);
        let y = area.1.min(fh);
        let width = area.2.min(fw - x);
        let height = area.3.min(fh - y);
        if width == 0 || height == 0 {
            return Err(anyhow!("nothing to take: the area is empty"));
        }
        let size = wgpu::Extent3d {
            width: fw,
            height: fh,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        draw(&self.device, &self.queue, &view, (fw, fh));

        let padded = padded_bytes_per_row(width);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot pixels"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("screenshot copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(5)),
            })
            .context("the GPU did not finish the screenshot")?;
        rx.recv()
            .context("the GPU did not answer")?
            .context("cannot read the screenshot from the GPU")?;
        let data = slice
            .get_mapped_range()
            .map_err(|err| anyhow!("cannot read the screenshot: {err:?}"))?
            .to_vec();
        buffer.unmap();
        let bgra = matches!(
            self.config.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        Ok(Shot {
            width,
            height,
            rgba: unpad(&data, width, height, padded, bgra),
        })
    }
}

/// Returns the surface size, or `None` when the window has no area (for example, it is minimized).
/// The backends to try, in order: the chosen one, then the usual ones of this OS.
pub fn backend_tries(choice: fterm_config::load::GpuBackend, os: &str) -> Vec<wgpu::Backends> {
    use fterm_config::load::GpuBackend as B;
    // DX12 is the native API on Windows (else wgpu may pick Vulkan), Metal on macOS.
    let usual = match os {
        "windows" => vec![
            wgpu::Backends::DX12,
            wgpu::Backends::VULKAN,
            wgpu::Backends::GL,
        ],
        "macos" => vec![wgpu::Backends::METAL, wgpu::Backends::GL],
        _ => vec![wgpu::Backends::VULKAN, wgpu::Backends::GL],
    };
    let first = match choice {
        B::Auto => None,
        B::Dx12 => Some(wgpu::Backends::DX12),
        B::Vulkan => Some(wgpu::Backends::VULKAN),
        B::Gl => Some(wgpu::Backends::GL),
        B::Metal => Some(wgpu::Backends::METAL),
    };
    let mut tries: Vec<wgpu::Backends> = first.into_iter().collect();
    tries.extend(usual.into_iter().filter(|b| Some(*b) != first));
    tries
}

/// The panes take a new window size only when the window is really there: not when it is minimized
/// (Windows gives 0x0 then), so the programs in them keep their size.
pub fn panes_follow(size: PhysicalSize<u32>, minimized: bool) -> bool {
    !minimized && surface_size(size).is_some()
}

pub fn surface_size(size: PhysicalSize<u32>) -> Option<(u32, u32)> {
    (size.width > 0 && size.height > 0).then_some((size.width, size.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_size_has_no_surface() {
        assert_eq!(surface_size(PhysicalSize::new(0, 0)), None);
        assert_eq!(surface_size(PhysicalSize::new(800, 0)), None);
        assert_eq!(surface_size(PhysicalSize::new(0, 600)), None);
    }

    #[test]
    fn the_chosen_backend_first_then_the_others() {
        use fterm_config::load::GpuBackend as B;
        use wgpu::Backends as W;
        assert_eq!(
            backend_tries(B::Auto, "windows"),
            [W::DX12, W::VULKAN, W::GL]
        );
        assert_eq!(backend_tries(B::Gl, "windows"), [W::GL, W::DX12, W::VULKAN]);
        assert_eq!(
            backend_tries(B::Vulkan, "windows"),
            [W::VULKAN, W::DX12, W::GL]
        );
        assert_eq!(backend_tries(B::Auto, "macos"), [W::METAL, W::GL]);
        assert_eq!(backend_tries(B::Auto, "linux"), [W::VULKAN, W::GL]);
        // A backend that this system does not have is tried first (it fails at once), then the usual ones.
        assert_eq!(backend_tries(B::Dx12, "linux"), [W::DX12, W::VULKAN, W::GL]);
    }

    #[test]
    fn a_minimized_window_keeps_the_pane_sizes() {
        // Windows gives a size of 0x0 when the window is minimized: the programs in the panes
        // must not get a terminal of one cell (they draw their screen again for it).
        assert!(!panes_follow(PhysicalSize::new(0, 0), false));
        assert!(!panes_follow(PhysicalSize::new(1024, 0), false));
        assert!(
            !panes_follow(PhysicalSize::new(1024, 640), true),
            "minimized"
        );
        assert!(panes_follow(PhysicalSize::new(1024, 640), false));
        assert!(
            panes_follow(PhysicalSize::new(200, 120), false),
            "a small window is a real size"
        );
    }

    #[test]
    fn a_pane_in_whole_pixels() {
        use fterm_mux::Rect;
        assert_eq!(
            pixel_area(Rect::new(10.4, 20.6, 100.2, 50.5)),
            (10, 20, 101, 52)
        );
        // Only the part on the window: from 0 to 7 across, from 0 to 9 down.
        assert_eq!(pixel_area(Rect::new(-3.0, -1.0, 10.0, 10.0)), (0, 0, 7, 9));
        assert_eq!(pixel_area(Rect::new(0.0, 0.0, 0.0, 0.0)), (0, 0, 0, 0));
    }

    #[test]
    fn a_copied_row_is_a_multiple_of_256_bytes() {
        assert_eq!(padded_bytes_per_row(10), 256);
        assert_eq!(padded_bytes_per_row(64), 256);
        assert_eq!(padded_bytes_per_row(65), 512);
        assert_eq!(padded_bytes_per_row(1920), 7680);
    }

    #[test]
    fn the_rows_without_padding_and_in_rgba() {
        // 2x2 pixels, rows of 256 bytes; the GPU gives BGRA.
        let mut data = vec![0u8; 512];
        data[0..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        data[256..264].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);
        assert_eq!(
            unpad(&data, 2, 2, 256, true),
            [3, 2, 1, 4, 7, 6, 5, 8, 11, 10, 9, 12, 15, 14, 13, 16]
        );
        assert_eq!(unpad(&data, 2, 1, 256, false), [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn normal_size_is_kept() {
        assert_eq!(surface_size(PhysicalSize::new(800, 600)), Some((800, 600)));
    }
}
