// No console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod access;
mod agent;
mod ai_chat;
mod api;
mod app;
mod claude_hooks;
mod clipboard;
mod close;
mod env;
mod gpu;
mod hints;
mod history_popup;
mod inbox;
mod input;
mod mouse;
mod notify;
mod palette;
mod panels;
mod paths;
mod screenshot;
mod session_state;
mod text_command;
mod themes;
mod title;
mod waits;
mod window_icon;

use tracing_subscriber::EnvFilter;
use winit::event_loop::{ControlFlow, EventLoop};

fn main() -> anyhow::Result<()> {
    // RUST_LOG changes the log level, for example RUST_LOG=debug.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let event_loop = EventLoop::<app::UserEvent>::with_user_event().build()?;
    // Sleep until the next event. We draw only when something changes.
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = app::App::new(event_loop.create_proxy());
    event_loop.run_app(&mut app)?;
    Ok(())
}
