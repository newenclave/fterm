//! The app: it gets window events from winit, sends keys to the session, and draws the grid.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fterm_render::Renderer;
use fterm_term::alacritty_terminal::grid::Scroll;
use fterm_term::alacritty_terminal::index::{Point, Side};
use fterm_term::alacritty_terminal::selection::{Selection, SelectionType};
use fterm_term::alacritty_terminal::term::TermMode;
use fterm_term::copy_mode::{self, CopyAction, CopyResult};
use fterm_term::links::url_at;
use fterm_term::session::{Session, SessionOptions, TermEvent};
use fterm_term::size::GridSize;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::clipboard::{Clipboard, paste_bytes};
use crate::gpu::Gpu;
use crate::input::{KeyInput, copy_mode_action, encode_key};
use crate::mouse::{
    ClickCounter, GridGeometry, ReportButton, ReportKind, ReportMods, Wheel, autoscroll_lines,
    encode_mouse,
};

/// Font size in logical pixels. It is multiplied by the scale factor (DPI).
const FONT_SIZE: f32 = 14.0;
/// Empty space around the grid, in logical pixels.
const PADDING: f32 = 6.0;
/// How often auto-scroll moves while the user drags a selection out of the window.
const AUTOSCROLL_TICK: Duration = Duration::from_millis(16);
/// How long "Copied N lines" stays in the window title.
const TITLE_MESSAGE: Duration = Duration::from_millis(1500);

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

/// The state of the mouse.
#[derive(Default)]
struct MouseState {
    /// Last mouse position in the window, in pixels.
    position: (f64, f64),
    /// The left button is down and the user is selecting.
    selecting: bool,
    /// The mouse moved to another cell after the press (so it is a drag, not a click).
    dragged: bool,
    clicks: ClickCounter,
    wheel: Wheel,
    /// The button that the app knows is down (when the app gets the mouse).
    reported_button: Option<ReportButton>,
    /// The last cell sent to the app, so we send motion only when the cell changes.
    reported_cell: Option<(usize, usize)>,
}

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    running: Option<Running>,
    mods: ModifiersState,
    focused: bool,
    mouse: MouseState,
    clipboard: Clipboard,
    /// The title that the shell asked for.
    title: String,
    /// When to put the shell title back after a short message.
    title_message_until: Option<Instant>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            proxy,
            running: None,
            mods: ModifiersState::empty(),
            focused: true,
            mouse: MouseState::default(),
            clipboard: Clipboard::new(),
            title: "fterm".to_owned(),
            title_message_until: None,
        }
    }

    fn start(&self, event_loop: &ActiveEventLoop) -> anyhow::Result<Running> {
        let attributes = Window::default_attributes()
            .with_title("fterm")
            .with_inner_size(LogicalSize::new(1024.0, 640.0));
        let window = Arc::new(event_loop.create_window(attributes)?);
        // IME: input methods for Chinese, Japanese, Korean, and others.
        window.set_ime_allowed(true);
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

        // Dev only: FTERM_RUN="command" types this command into the shell after start.
        // Test scripts use it, so they do not need to send keys to the window.
        if cfg!(debug_assertions)
            && let Ok(command) = std::env::var("FTERM_RUN")
        {
            tracing::info!(%command, "FTERM_RUN");
            session.write(format!("{command}\r").into_bytes());
        }

        Ok(Running {
            window,
            gpu,
            renderer,
            session,
        })
    }

    /// Shows a short message in the window title.
    fn title_message(&mut self, message: &str) {
        if let Some(running) = &self.running {
            running
                .window
                .set_title(&format!("{} — {message}", self.title));
            self.title_message_until = Some(Instant::now() + TITLE_MESSAGE);
        }
    }

    fn copy_text(&mut self, text: String) {
        let lines = text.lines().count().max(1);
        self.clipboard.copy(&text);
        let message = if lines == 1 {
            "Copied 1 line".to_owned()
        } else {
            format!("Copied {lines} lines")
        };
        self.title_message(&message);
    }

    fn paste(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let Some(text) = self.clipboard.paste() else {
            return;
        };
        let bracketed = running
            .session
            .with_term(|term| term.mode().contains(TermMode::BRACKETED_PASTE));
        running
            .session
            .with_term_mut(|term, _| term.scroll_display(Scroll::Bottom));
        running.session.write(paste_bytes(&text, bracketed));
    }

    /// The keys of fterm itself (copy, paste, scroll, copy mode). Returns true when the key is used.
    fn handle_app_key(&mut self, event: &KeyEvent) -> bool {
        let Some(running) = &self.running else {
            return false;
        };
        let session = &running.session;
        let (ctrl, shift) = (self.mods.control_key(), self.mods.shift_key());
        let physical = event.physical_key;
        let key = KeyInput {
            logical: &event.logical_key,
            physical,
            text: event.text.as_deref(),
            mods: self.mods,
        };

        // Ctrl+Shift+Space: copy mode on or off.
        if ctrl && shift && physical == PhysicalKey::Code(KeyCode::Space) {
            let active = session.with_term(copy_mode::is_active);
            session.with_term_mut(|term, selection| {
                if active {
                    copy_mode::apply(term, selection, CopyAction::Exit);
                } else {
                    copy_mode::enter(term, selection);
                }
            });
            running.window.request_redraw();
            return true;
        }

        // In copy mode, all keys belong to copy mode.
        if session.with_term(copy_mode::is_active) {
            if let Some(action) = copy_mode_action(&key) {
                let result = session
                    .with_term_mut(|term, selection| copy_mode::apply(term, selection, action));
                running.window.request_redraw();
                if let CopyResult::Copied(text) = result {
                    self.copy_text(text);
                }
            }
            return true;
        }

        let selected = || session.with_term(|term| term.selection_to_string());
        // Ctrl+Shift+C always copies. Ctrl+C copies only when there is a selection, else it is ^C.
        if ctrl && physical == PhysicalKey::Code(KeyCode::KeyC) {
            if let Some(text) = selected().filter(|t| !t.is_empty()) {
                self.copy_text(text);
                return true;
            }
            if shift {
                return true;
            }
        }
        // Ctrl+Shift+V and Shift+Insert paste.
        let paste = (ctrl && shift && physical == PhysicalKey::Code(KeyCode::KeyV))
            || (shift && event.logical_key == Key::Named(NamedKey::Insert));
        if paste {
            self.paste();
            return true;
        }

        // Shift + PageUp/PageDown/Home/End scroll the view.
        if shift && !ctrl {
            let scroll = match event.logical_key {
                Key::Named(NamedKey::PageUp) => Some(Scroll::PageUp),
                Key::Named(NamedKey::PageDown) => Some(Scroll::PageDown),
                Key::Named(NamedKey::Home) => Some(Scroll::Top),
                Key::Named(NamedKey::End) => Some(Scroll::Bottom),
                _ => None,
            };
            if let Some(scroll) = scroll {
                session.with_term_mut(|term, _| term.scroll_display(scroll));
                running.window.request_redraw();
                return true;
            }
        }
        false
    }

    /// True when the app wants the mouse and Shift is not held (Shift always selects).
    fn app_wants_mouse(&self) -> bool {
        let Some(running) = &self.running else {
            return false;
        };
        !self.mods.shift_key()
            && running
                .session
                .with_term(|term| term.mode().intersects(TermMode::MOUSE_MODE))
    }

    /// Sends a mouse event to the app at the current mouse cell.
    fn report_mouse(&mut self, kind: ReportKind) {
        let Some(running) = &self.running else {
            return;
        };
        let (point, _) =
            Self::geometry(running).cell_at(self.mouse.position.0, self.mouse.position.1, 0);
        let cell = (point.column.0, point.line.0.max(0) as usize);
        let sgr = running
            .session
            .with_term(|term| term.mode().contains(TermMode::SGR_MOUSE));
        let mods = ReportMods {
            shift: self.mods.shift_key(),
            alt: self.mods.alt_key(),
            ctrl: self.mods.control_key(),
        };
        if let Some(bytes) = encode_mouse(kind, cell.0, cell.1, mods, sgr) {
            running.session.write(bytes);
        }
        self.mouse.reported_cell = Some(cell);
    }

    fn mouse_button(&mut self, state: ElementState, button: MouseButton) {
        let report_button = match button {
            MouseButton::Left => Some(ReportButton::Left),
            MouseButton::Middle => Some(ReportButton::Middle),
            MouseButton::Right => Some(ReportButton::Right),
            _ => None,
        };
        if let Some(b) = report_button
            && self.app_wants_mouse()
        {
            match state {
                ElementState::Pressed => {
                    self.mouse.reported_button = Some(b);
                    self.report_mouse(ReportKind::Press(b));
                }
                ElementState::Released => {
                    self.mouse.reported_button = None;
                    self.report_mouse(ReportKind::Release(b));
                }
            }
            return;
        }
        match (button, state) {
            (MouseButton::Left, ElementState::Pressed) => {
                if self.mods.control_key() && self.open_link_under_mouse() {
                    return;
                }
                self.mouse_press();
            }
            (MouseButton::Left, ElementState::Released) => self.mouse_release(),
            (MouseButton::Right, ElementState::Pressed) => self.paste(),
            _ => {}
        }
    }

    /// Ctrl + click on a URL opens it. Returns true when there was a URL.
    fn open_link_under_mouse(&self) -> bool {
        let Some(running) = &self.running else {
            return false;
        };
        let (point, _) = self.mouse_cell(running);
        let Some(url) = running.session.with_term(|term| url_at(term, point)) else {
            return false;
        };
        tracing::info!(%url, "open link");
        if let Err(err) = open::that_detached(&url) {
            tracing::warn!("cannot open {url}: {err}");
        }
        true
    }

    /// IME text is ready: send it like typed text.
    fn ime_commit(&self, text: &str) {
        let Some(running) = &self.running else {
            return;
        };
        running.session.with_term_mut(|term, sticky| {
            term.scroll_display(Scroll::Bottom);
            if sticky.is_active() {
                sticky.set(term, None);
            }
        });
        running.session.write(text.as_bytes().to_vec());
    }

    /// Tells the IME where the cursor is, so its window opens next to the cursor.
    fn update_ime_area(running: &Running) {
        let cell = running.renderer.cell();
        let padding = running.renderer.padding();
        let (line, column) = running.session.with_term(|term| {
            let point = term.grid().cursor.point;
            (
                point.line.0 + term.grid().display_offset() as i32,
                point.column.0,
            )
        });
        let x = padding + column as f32 * cell.width;
        let y = padding + line.max(0) as f32 * cell.height;
        running.window.set_ime_cursor_area(
            PhysicalPosition::new(x as f64, y as f64),
            winit::dpi::PhysicalSize::new(cell.width as f64, cell.height as f64),
        );
    }

    fn geometry(running: &Running) -> GridGeometry {
        let cell = running.renderer.cell();
        GridGeometry {
            cell_width: cell.width,
            cell_height: cell.height,
            padding: running.renderer.padding(),
            size: running.session.grid_size(),
        }
    }

    /// The cell under the mouse now, in history coordinates.
    fn mouse_cell(&self, running: &Running) -> (Point, Side) {
        let offset = running
            .session
            .with_term(|term| term.grid().display_offset());
        let (x, y) = self.mouse.position;
        Self::geometry(running).cell_at(x, y, offset)
    }

    /// Moves the end of the selection to the mouse.
    fn extend_selection_to_mouse(&self, running: &Running) {
        let (point, side) = self.mouse_cell(running);
        running.session.with_term_mut(|term, sticky| {
            if let Some(mut selection) = term.selection.clone() {
                selection.update(point, side);
                sticky.set(term, Some(selection));
            }
        });
        running.window.request_redraw();
    }

    fn mouse_press(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let (point, side) = self.mouse_cell(running);
        let has_selection = running.session.with_term(|term| term.selection.is_some());
        self.mouse.selecting = true;
        self.mouse.dragged = false;

        // Shift + click: move the end of the selection (also after scrolling far away).
        if self.mods.shift_key() && has_selection {
            self.mouse.dragged = true;
            self.extend_selection_to_mouse(running);
            return;
        }

        let ty = match self.mouse.clicks.click(Instant::now(), point) {
            2 => SelectionType::Semantic,
            3 => SelectionType::Lines,
            _ if self.mods.alt_key() => SelectionType::Block,
            _ => SelectionType::Simple,
        };
        if ty != SelectionType::Simple && ty != SelectionType::Block {
            self.mouse.dragged = true;
        }
        running.session.with_term_mut(|term, sticky| {
            sticky.set(term, Some(Selection::new(ty, point, side)));
        });
        running.window.request_redraw();
    }

    fn mouse_release(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        self.mouse.selecting = false;
        // A click without a drag removes the selection.
        if !self.mouse.dragged {
            running
                .session
                .with_term_mut(|term, sticky| sticky.set(term, None));
            running.window.request_redraw();
        }
    }

    fn mouse_moved(&mut self, x: f64, y: f64) {
        let old = self.mouse.position;
        self.mouse.position = (x, y);
        if self.app_wants_mouse() {
            let Some(running) = &self.running else {
                return;
            };
            let (motion, drag) = running.session.with_term(|term| {
                let mode = term.mode();
                (
                    mode.contains(TermMode::MOUSE_MOTION),
                    mode.contains(TermMode::MOUSE_DRAG),
                )
            });
            let button = self.mouse.reported_button;
            let (point, _) = Self::geometry(running).cell_at(x, y, 0);
            let cell = (point.column.0, point.line.0.max(0) as usize);
            if (motion || (drag && button.is_some())) && self.mouse.reported_cell != Some(cell) {
                self.report_mouse(ReportKind::Motion(button));
            }
            return;
        }
        let Some(running) = &self.running else {
            return;
        };
        if !self.mouse.selecting {
            return;
        }
        let geometry = Self::geometry(running);
        let old_cell = geometry.cell_at(old.0, old.1, 0).0;
        let new_cell = geometry.cell_at(x, y, 0).0;
        if old_cell != new_cell {
            self.mouse.dragged = true;
        }
        self.extend_selection_to_mouse(running);
    }

    fn mouse_wheel(&mut self, delta: winit::event::MouseScrollDelta) {
        let Some(running) = &self.running else {
            return;
        };
        let lines = self
            .mouse
            .wheel
            .lines(delta, running.renderer.cell().height);
        if lines == 0 {
            return;
        }
        if self.app_wants_mouse() {
            let kind = if lines > 0 {
                ReportKind::WheelUp
            } else {
                ReportKind::WheelDown
            };
            for _ in 0..lines.unsigned_abs().min(10) {
                self.report_mouse(kind);
            }
            return;
        }
        let alt_screen = running
            .session
            .with_term(|term| term.mode().contains(TermMode::ALT_SCREEN));
        if alt_screen {
            // Full-screen apps (vim, less) have no history: the wheel sends arrow keys.
            let arrow: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
            running
                .session
                .write(arrow.repeat(lines.unsigned_abs() as usize));
            return;
        }
        running
            .session
            .with_term_mut(|term, _| term.scroll_display(Scroll::Delta(lines)));
        // While selecting, the end of the selection follows the mouse in the new view.
        if self.mouse.selecting {
            self.extend_selection_to_mouse(running);
        }
        running.window.request_redraw();
    }

    /// Scrolls while the user drags the selection above or below the window.
    fn autoscroll(&mut self) -> bool {
        let Some(running) = &self.running else {
            return false;
        };
        if !self.mouse.selecting {
            return false;
        }
        let height = f64::from(running.window.inner_size().height);
        let lines = autoscroll_lines(self.mouse.position.1, height);
        if lines == 0 {
            return false;
        }
        running
            .session
            .with_term_mut(|term, _| term.scroll_display(Scroll::Delta(lines)));
        self.mouse.dragged = true;
        self.extend_selection_to_mouse(running);
        true
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
                self.title = if title.is_empty() {
                    "fterm".to_owned()
                } else {
                    title
                };
                if self.title_message_until.is_none() {
                    running.window.set_title(&self.title);
                }
            }
            UserEvent::Term(TermEvent::Exit) => {
                tracing::info!("the shell ended, closing the window");
                event_loop.exit();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let mut wake_at = None;
        if self.autoscroll() {
            wake_at = Some(now + AUTOSCROLL_TICK);
        }
        if let Some(until) = self.title_message_until {
            if now >= until {
                self.title_message_until = None;
                if let Some(running) = &self.running {
                    running.window.set_title(&self.title);
                }
            } else {
                wake_at = Some(wake_at.map_or(until, |t: Instant| t.min(until)));
            }
        }
        event_loop.set_control_flow(match wake_at {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.running.is_none() {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                let running = self.running.as_mut().unwrap();
                running.gpu.resize(size);
                update_grid(running);
                running.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let running = self.running.as_mut().unwrap();
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
                let running = self.running.as_ref().unwrap();
                let report = running
                    .session
                    .with_term(|term| term.mode().contains(TermMode::FOCUS_IN_OUT));
                if report {
                    let bytes: &[u8] = if focused { b"\x1b[I" } else { b"\x1b[O" };
                    running.session.write(bytes.to_vec());
                }
                running.window.request_redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => self.ime_commit(&text),
            WindowEvent::ModifiersChanged(mods) => self.mods = mods.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.handle_app_key(&event) {
                    return;
                }
                let running = self.running.as_ref().unwrap();
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
                    // Typing goes back to the bottom and removes the selection.
                    running.session.with_term_mut(|term, sticky| {
                        term.scroll_display(Scroll::Bottom);
                        if sticky.is_active() {
                            sticky.set(term, None);
                        }
                    });
                    running.session.write(bytes);
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.mouse_moved(position.x, position.y),
            WindowEvent::MouseInput { state, button, .. } => self.mouse_button(state, button),
            WindowEvent::MouseWheel { delta, .. } => self.mouse_wheel(delta),
            WindowEvent::RedrawRequested => {
                let Running {
                    gpu,
                    renderer,
                    session,
                    ..
                } = self.running.as_mut().unwrap();
                let focused = self.focused;
                let result = gpu.frame(|device, queue, view, size| {
                    session.with_term(|term| {
                        renderer.render(device, queue, view, size, term, focused)
                    });
                });
                if let Err(err) = result {
                    tracing::error!("cannot draw: {err:#}");
                    event_loop.exit();
                    return;
                }
                Self::update_ime_area(self.running.as_ref().unwrap());
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
