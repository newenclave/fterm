//! The looks of the text UI (`fterm_ui`) in the colors of the theme, for the panes that the app draws (a Review,
//! a form). Colors come by role from the theme.

use fterm_render::theme::UiColors;
use fterm_ui::look::{Look, Rgb, Row, Style};

fn rgb(c: fterm_term::alacritty_terminal::vte::ansi::Rgb) -> Rgb {
    Rgb {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

/// The style of a look.
pub fn style(ui: &UiColors, look: Look) -> Style {
    let fg = |c| Style {
        fg: Some(rgb(c)),
        ..Style::default()
    };
    match look {
        Look::Title | Look::Heading => Style {
            bold: true,
            ..fg(ui.accent)
        },
        Look::Dim | Look::Code => fg(ui.text_dim),
        Look::Text => fg(ui.text),
        Look::Ok => fg(ui.success),
        Look::Remove => Style {
            strike: true,
            ..fg(ui.error)
        },
        Look::Changed => fg(ui.warning),
        Look::Comment => fg(ui.info),
        Look::Input => Style {
            bg: Some(rgb(ui.input_bg)),
            ..fg(ui.text)
        },
        Look::Cursor => Style {
            bg: Some(rgb(ui.text)),
            ..fg(ui.surface_active)
        },
        Look::Error => fg(ui.error),
    }
}

/// Rows as ANSI text for a grid: from the top left, the selected row in the color of a selection.
pub fn render(rows: &[Row], ui: &UiColors) -> String {
    fterm_ui::look::render_ansi(rows, |look| style(ui, look), rgb(ui.selected))
}
