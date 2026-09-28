//! The app: it gets window events from winit and sends them to the GPU part.

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::gpu::Gpu;

#[derive(Default)]
pub struct App {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title("fterm")
            .with_inner_size(LogicalSize::new(1024.0, 640.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                tracing::error!("cannot create the window: {err}");
                event_loop.exit();
                return;
            }
        };

        match pollster::block_on(Gpu::new(window.clone(), event_loop.owned_display_handle())) {
            Ok(gpu) => self.gpu = Some(gpu),
            Err(err) => {
                tracing::error!("cannot start the GPU: {err:#}");
                event_loop.exit();
                return;
            }
        }
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let (Some(window), Some(gpu)) = (&self.window, &mut self.gpu) else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                gpu.resize(size);
                window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // The new size comes in the next `Resized` event.
                tracing::debug!(scale_factor, "scale factor changed");
            }
            WindowEvent::RedrawRequested => {
                if let Err(err) = gpu.render() {
                    tracing::error!("cannot draw: {err:#}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
}
