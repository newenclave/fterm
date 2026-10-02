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
        draw: impl FnOnce(&wgpu::Device, &wgpu::Queue, &wgpu::TextureView, (u32, u32)),
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

/// Returns the surface size, or `None` when the window has no area (for example, it is minimized).
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
    fn normal_size_is_kept() {
        assert_eq!(surface_size(PhysicalSize::new(800, 600)), Some((800, 600)));
    }
}
