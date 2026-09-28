//! GPU state: the wgpu surface, device, and drawing of one frame.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use winit::dpi::PhysicalSize;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

/// Background color of the window (Catppuccin Mocha "base").
const BACKGROUND: [u8; 3] = [0x1e, 0x1e, 0x2e];

pub struct Gpu {
    instance: wgpu::Instance,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    clear: wgpu::Color,
}

impl Gpu {
    pub async fn new(window: Arc<Window>, display: OwnedDisplayHandle) -> Result<Self> {
        let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display));
        if cfg!(windows) {
            // DX12 is the native API on Windows. Without this, wgpu may pick Vulkan.
            desc.backends = wgpu::Backends::DX12;
        }
        // `with_env` lets the user pick a backend with WGPU_BACKEND (dx12, vulkan, gl, metal).
        let instance = wgpu::Instance::new(desc.with_env());
        let surface = instance
            .create_surface(window.clone())
            .context("cannot create the GPU surface")?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
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
            clear: clear_color(BACKGROUND, format.is_srgb()),
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

    /// Draws one frame. An error here means the GPU cannot work any more.
    pub fn render(&mut self) -> Result<()> {
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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(self.clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(frame);
        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(())
    }
}

/// Returns the surface size, or `None` when the window has no area (for example, it is minimized).
pub fn surface_size(size: PhysicalSize<u32>) -> Option<(u32, u32)> {
    (size.width > 0 && size.height > 0).then_some((size.width, size.height))
}

/// Converts one sRGB color channel (0..=255) to a linear value (0.0..=1.0).
pub fn srgb_to_linear(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Makes the clear color. An sRGB surface needs linear values, other surfaces take sRGB values as is.
pub fn clear_color(rgb: [u8; 3], srgb_surface: bool) -> wgpu::Color {
    let channel = |c: u8| {
        if srgb_surface {
            srgb_to_linear(c)
        } else {
            f64::from(c) / 255.0
        }
    };
    wgpu::Color {
        r: channel(rgb[0]),
        g: channel(rgb[1]),
        b: channel(rgb[2]),
        a: 1.0,
    }
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
    fn normal_size_is_kept() {
        assert_eq!(surface_size(PhysicalSize::new(800, 600)), Some((800, 600)));
    }

    #[test]
    fn srgb_to_linear_known_values() {
        assert_eq!(srgb_to_linear(0), 0.0);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-9);
        // sRGB 128 is about 0.2159 in linear space.
        assert!((srgb_to_linear(128) - 0.2159).abs() < 1e-3);
        // Small values use the linear part of the curve: 10 / 255 / 12.92.
        assert!((srgb_to_linear(10) - 10.0 / 255.0 / 12.92).abs() < 1e-9);
    }

    #[test]
    fn clear_color_for_srgb_surface_is_linear() {
        let c = clear_color([128, 0, 255], true);
        assert!((c.r - srgb_to_linear(128)).abs() < 1e-9);
        assert_eq!(c.g, 0.0);
        assert!((c.b - 1.0).abs() < 1e-9);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn clear_color_for_plain_surface_is_not_changed() {
        let c = clear_color([128, 0, 255], false);
        assert!((c.r - 128.0 / 255.0).abs() < 1e-9);
        assert_eq!(c.g, 0.0);
        assert!((c.b - 1.0).abs() < 1e-9);
        assert_eq!(c.a, 1.0);
    }
}
