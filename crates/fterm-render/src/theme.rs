//! The colors of the UI by role. The app makes them from the theme; the default is Catppuccin Mocha.
//! This is the only file of the renderer with color values: the others take them from here.

use fterm_term::alacritty_terminal::vte::ansi::Rgb;

macro_rules! ui_colors {
    ($($name:ident = $hex:literal),* $(,)?) => {
        /// The UI colors. The names are the roles of a theme (see docs/THEMES.md).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct UiColors {
            $(pub $name: Rgb,)*
        }

        impl Default for UiColors {
            /// Catppuccin Mocha.
            fn default() -> Self {
                Self {
                    $($name: hex($hex),)*
                }
            }
        }
    };
}

ui_colors!(
    // Tab bar and dock background.
    surface = 0x181825,
    // The active tab.
    surface_active = 0x1e1e2e,
    // Hover, boxes (palette, message box, toasts), the dock edge, scroll bars.
    overlay = 0x313244,
    // Selected rows and the lines between panes.
    selected = 0x45475a,
    // The active tab line, box borders, the frame of the active pane.
    accent = 0xcba6f7,
    text = 0xcdd6f4,
    text_dim = 0x9399b2,
    // The grey history hint after the cursor.
    text_ghost = 0x6c7086,
    // The scroll indicator of a pane.
    scrollbar = 0x7f849c,
    copy_cursor = 0xf9e2af,
    code_bg = 0x11111b,
    input_bg = 0x26273a,
    chip_bg = 0x3a3c55,
    info = 0x89b4fa,
    success = 0xa6e3a1,
    warning = 0xf9e2af,
    error = 0xf38ba8,
    attention = 0xcba6f7,
    agent_working = 0x4c9aff,
    agent_waiting = 0xf5c218,
    agent_done = 0x3fc56b,
    agent_error = 0xf04a4a,
);

const fn hex(v: u32) -> Rgb {
    Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    }
}

#[cfg(test)]
mod tests {
    /// The rule of the renderer: colors come from the theme, not from constants in the drawing code.
    /// The app icon (icon.rs) is a picture, not a part of the UI, so it has its own colors.
    #[test]
    fn no_colors_are_hard_coded_outside_the_theme() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.ends_with(".rs") || name == "theme.rs" || name == "icon.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // Only the code: the tests may make colors.
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            for (i, line) in code.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                if line.contains("Rgb {") && !line.contains("struct") && !line.contains("-> Rgb {")
                {
                    found.push(format!("{name}:{}: {}", i + 1, line.trim()));
                }
            }
        }
        assert!(found.is_empty(), "hard-coded colors:\n{}", found.join("\n"));
    }
}
