//! The app: it gets window events from winit, sends keys to the session, and draws the grid.

use std::sync::Arc;

use fterm_render::Renderer;
use fterm_term::alacritty_terminal::term::TermMode;
use fterm_term::session::{Session, SessionOptions, TermEvent};
use fterm_term::size::GridSize;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::gpu::Gpu;
use crate::input::{KeyInput, encode_key};

/// Font size in logical pixels. It is multiplied by the scale factor (DPI).
const FONT_SIZE: f32 = 14.0;
/// Empty space around the grid, in logical pixels.
const PADDING: f32 = 6.0;

/// Events from other threads to the window thread.
#[derive(Debug)]
pub enum UserEvent {
    Term(TermEvent),
}

/// Everything that exists only while the window is open.
struct Running {
    window: Arc<Window>,
    gpu: Gpu,
    renderer: Renderer,
    session: Session,
}

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    running: Option<Running>,
    mods: ModifiersState,
    focused: bool,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            proxy,
            running: None,
            mods: ModifiersState::empty(),
            focused: true,
        }
    }

    fn start(&self, event_loop: &ActiveEventLoop) -> anyhow::Result<Running> {
        let attributes = Window::default_attributes()
            .with_title("fterm")
            .with_inner_size(LogicalSize::new(1024.0, 640.0));
        let window = Arc::new(event_loop.create_window(attributes)?);
        let gpu = pollster::block_on(Gpu::new(window.clone(), event_loop.owned_display_handle()))?;
        let scale = window.scale_factor() as f32;
        let renderer = Renderer::new(
            gpu.device(),
            gpu.format(),
            FONT_SIZE * scale,
            PADDING * scale,
        )?;

        let size = grid_size(&window, &renderer);
        let proxy = self.proxy.clone();
        let session = Session::spawn(
            SessionOptions::default(),
            size,
            cell_px(&renderer),
            move |event| {
                // The window may be closed already. Then nobody needs the event.
                let _ = proxy.send_event(UserEvent::Term(event));
            },
        )?;
        tracing::info!(columns = size.columns, rows = size.rows, "terminal started");

        Ok(Running {
            window,
            gpu,
            renderer,
            session,
        })
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        match self.start(event_loop) {
            Ok(running) => {
                running.window.request_redraw();
                self.running = Some(running);
            }
            Err(err) => {
                tracing::error!("cannot start: {err:#}");
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        let Some(running) = &self.running else {
            return;
        };
        match event {
            UserEvent::Term(TermEvent::Redraw) => running.window.request_redraw(),
            UserEvent::Term(TermEvent::Title(title)) => {
                running
                    .window
                    .set_title(if title.is_empty() { "fterm" } else { &title });
            }
            UserEvent::Term(TermEvent::Exit) => {
                tracing::info!("the shell ended, closing the window");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(running) = &mut self.running else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                running.gpu.resize(size);
                update_grid(running);
                running.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                tracing::debug!(scale_factor, "scale factor changed");
                let scale = scale_factor as f32;
                if let Err(err) = running.renderer.set_font_size(
                    running.gpu.device(),
                    FONT_SIZE * scale,
                    PADDING * scale,
                ) {
                    tracing::error!("cannot change the font size: {err:#}");
                }
                // The new window size comes in the next `Resized` event.
                update_grid(running);
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                running.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(mods) => self.mods = mods.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let key = KeyInput {
                    logical: &event.logical_key,
                    physical: event.physical_key,
                    text: event.text.as_deref(),
                    mods: self.mods,
                };
                let app_cursor = running
                    .session
                    .with_term(|term| term.mode().contains(TermMode::APP_CURSOR));
                if let Some(bytes) = encode_key(&key, app_cursor) {
                    running.session.write(bytes);
                }
            }
            WindowEvent::RedrawRequested => {
                let Running {
                    gpu,
                    renderer,
                    session,
                    ..
                } = running;
                let focused = self.focused;
                let result = gpu.frame(|device, queue, view, size| {
                    session.with_term(|term| {
                        renderer.render(device, queue, view, size, term, focused)
                    });
                });
                if let Err(err) = result {
                    tracing::error!("cannot draw: {err:#}");
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }
}

/// Cell size in whole pixels, for the pty (some apps ask for it).
fn cell_px(renderer: &Renderer) -> (u16, u16) {
    let cell = renderer.cell();
    (cell.width as u16, cell.height as u16)
}

fn grid_size(window: &Window, renderer: &Renderer) -> GridSize {
    let size = window.inner_size();
    let cell = renderer.cell();
    GridSize::from_pixels(
        size.width,
        size.height,
        cell.width,
        cell.height,
        renderer.padding(),
    )
}

/// Tells the session the new grid size, but only when it really changed.
fn update_grid(running: &Running) {
    let size = grid_size(&running.window, &running.renderer);
    if size != running.session.grid_size() {
        tracing::debug!(
            columns = size.columns,
            rows = size.rows,
            "grid size changed"
        );
        running.session.resize(size, cell_px(&running.renderer));
    }
}
