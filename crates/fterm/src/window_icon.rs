//! The icon of the window (title bar, Alt+Tab, taskbar), from `fterm_render::icon`.

use winit::window::Icon;

/// The icon in this size, or `None` when winit does not take it.
pub fn window_icon(size: u32) -> Option<Icon> {
    match Icon::from_rgba(fterm_render::icon::rgba(size), size, size) {
        Ok(icon) => Some(icon),
        Err(err) => {
            tracing::warn!("cannot make the window icon: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_icons_are_made() {
        for size in [16, 32, 256] {
            assert!(window_icon(size).is_some(), "{size}");
        }
    }
}
