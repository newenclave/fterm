//! Mark up the picture of a picture pane: the mouse draws, keys pick the tool and the color, Enter saves the
//! picture (a file, the clipboard) and pastes its path into the pane that the picture was opened next to.

use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

use super::*;
use crate::markup::{Markup, Point, TextMask, Tool};

/// A picture pane: its marks, the picture as it is shown, and where it came from.
pub(super) struct PicturePane {
    pub markup: Markup,
    /// The shown picture and the version of the marks that it shows.
    pub shown: (u64, fterm_term::picture::Picture),
    /// The pane it was opened next to: the path of the result is pasted there.
    pub origin: Option<PaneId>,
    /// The text that is typed now: where (pixels of the shown picture) and what.
    pub text: Option<(Point, String)>,
    /// The mouse draws a mark now (the button is down).
    pub drawing: bool,
}

impl PicturePane {
    pub fn new(picture: fterm_term::picture::Picture, origin: Option<PaneId>) -> Self {
        let markup = Markup::new(picture.clone());
        Self {
            shown: (markup.version(), picture),
            markup,
            origin,
            text: None,
            drawing: false,
        }
    }

    /// Makes the shown picture again when the marks changed.
    pub fn refresh(&mut self) {
        if self.shown.0 != self.markup.version() {
            self.shown = (self.markup.version(), self.markup.render());
        }
    }
}

/// What a key does in a picture pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Tool(Tool),
    Color(usize),
    Undo,
    Redo,
    /// Enter: save the picture.
    Done,
    /// Esc: drop the mark that is drawn or the text that is typed.
    Cancel,
    /// Text that is typed.
    Type(String),
    Back,
    /// Enter while text is typed: the text is a mark.
    PutText,
}

/// The command of a key. `typing` = a text is typed now (then the keys go into it).
pub(super) fn command(
    key: &Key,
    text: Option<&str>,
    ctrl: bool,
    shift: bool,
    typing: bool,
) -> Option<Command> {
    let text = text
        .filter(|_| !ctrl)
        .map(|t| t.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|t| !t.is_empty());
    match key {
        Key::Named(NamedKey::Enter) if typing => return Some(Command::PutText),
        Key::Named(NamedKey::Enter) => return Some(Command::Done),
        Key::Named(NamedKey::Escape) => return Some(Command::Cancel),
        Key::Named(NamedKey::Backspace) if typing => return Some(Command::Back),
        _ => {}
    }
    if typing {
        return text.map(Command::Type);
    }
    if ctrl {
        let c = match key {
            Key::Character(c) => c.to_lowercase(),
            _ => return None,
        };
        // Ctrl+Shift+Z is zoom, so redo is Ctrl+Y only.
        return match (c.as_str(), shift) {
            ("z", false) => Some(Command::Undo),
            ("y", _) => Some(Command::Redo),
            _ => None,
        };
    }
    let c = text?.to_lowercase();
    Some(match c.as_str() {
        "p" => Command::Tool(Tool::Pen),
        "r" => Command::Tool(Tool::Rect),
        "a" => Command::Tool(Tool::Arrow),
        "t" => Command::Tool(Tool::Text),
        "h" => Command::Tool(Tool::Hide),
        "c" => Command::Tool(Tool::Crop),
        "1" | "2" | "3" | "4" => Command::Color(c.parse::<usize>().ok()? - 1),
        _ => return None,
    })
}

/// A mouse point (window pixels) in pixels of the shown picture, which is drawn in `rect`. Points out of the
/// picture are moved to its edge (a drag can go out).
pub(super) fn image_point(mouse: (f32, f32), rect: Rect, picture: (u32, u32)) -> Point {
    let x = ((mouse.0 - rect.x) / rect.width.max(1.0)).clamp(0.0, 1.0) * picture.0 as f32;
    let y = ((mouse.1 - rect.y) / rect.height.max(1.0)).clamp(0.0, 1.0) * picture.1 as f32;
    (x, y)
}

/// The picture of a picture pane in `pane` cells: it fills the pane but its last row (the tool bar).
pub(super) fn picture_cells(
    picture: (u32, u32),
    cell: (f32, f32),
    pane: (usize, usize),
) -> (usize, usize) {
    fterm_term::picture::fit(picture, cell, (pane.0, pane.1.saturating_sub(1).max(1)))
}

fn tool_name(tool: Tool) -> String {
    match tool {
        Tool::Pen => tr!("markup.pen"),
        Tool::Rect => tr!("markup.rect"),
        Tool::Arrow => tr!("markup.arrow"),
        Tool::Text => tr!("markup.text"),
        Tool::Hide => tr!("markup.hide"),
        Tool::Crop => tr!("markup.crop"),
    }
}

/// The tool bar at the bottom of a picture pane.
pub(super) fn bar_text(tool: Tool, color: usize, typing: bool) -> String {
    if typing {
        return tr!("markup.typing");
    }
    tr!("markup.bar", tool = tool_name(tool), color = color + 1)
}

impl App {
    /// The active pane, when it is a picture pane.
    pub(super) fn active_picture(&self) -> Option<PaneId> {
        let running = self.running.as_ref()?;
        let pane = running.mux.active_pane()?;
        running
            .panes
            .get(&pane)?
            .picture_pane
            .as_ref()
            .map(|_| pane)
    }

    /// The picture pane under the mouse, and the rect of its picture (window pixels).
    fn picture_under_mouse(&self) -> Option<(PaneId, Rect)> {
        let pane = self.pane_under_mouse()?;
        Some((pane, self.picture_rect(pane)?))
    }

    /// Where the picture of picture pane `pane` is drawn (window pixels).
    pub(super) fn picture_rect(&self, pane: PaneId) -> Option<Rect> {
        let running = self.running.as_ref()?;
        let p = running.panes.get(&pane)?;
        let pp = p.picture_pane.as_ref()?;
        let area = running
            .mux
            .pane_rects(running.tab_area())
            .into_iter()
            .find(|(id, _)| *id == pane)?
            .1;
        let cell = running.renderer.cell();
        let size = p.session.grid_size();
        let (columns, rows) = picture_cells(
            (pp.shown.1.width, pp.shown.1.height),
            (cell.width, cell.height),
            (size.columns, size.rows),
        );
        let padding = running.renderer.padding();
        Some(Rect::new(
            area.x + padding,
            area.y + padding,
            columns as f32 * cell.width,
            rows as f32 * cell.height,
        ))
    }

    /// Makes the shown pictures of picture panes again (before a frame).
    pub(super) fn refresh_pictures(&mut self) {
        if let Some(running) = &mut self.running {
            for p in running.panes.values_mut() {
                if let Some(pp) = &mut p.picture_pane {
                    pp.refresh();
                }
            }
        }
    }

    fn with_picture(&mut self, pane: PaneId, f: impl FnOnce(&mut PicturePane)) {
        if let Some(running) = &mut self.running {
            if let Some(pp) = running
                .panes
                .get_mut(&pane)
                .and_then(|p| p.picture_pane.as_deref_mut())
            {
                f(pp);
            }
            running.window.request_redraw();
        }
    }

    /// A key in the active picture pane.
    pub(super) fn markup_key(&mut self, event: &KeyEvent) {
        let Some(pane) = self.active_picture() else {
            return;
        };
        let typing = self
            .running
            .as_ref()
            .and_then(|r| r.panes.get(&pane))
            .and_then(|p| p.picture_pane.as_ref())
            .is_some_and(|pp| pp.text.is_some());
        let Some(command) = command(
            &event.logical_key,
            event.text.as_deref(),
            self.mods.control_key() || self.mods.alt_key(),
            self.mods.shift_key(),
            typing,
        ) else {
            return;
        };
        match command {
            Command::Done => return self.finish_markup(pane),
            Command::Type(text) => return self.type_text(pane, &text),
            Command::PutText => return self.put_text(pane),
            Command::Back => {
                self.with_picture(pane, |pp| {
                    if let Some((_, t)) = &mut pp.text {
                        t.pop();
                    }
                });
                return self.show_text(pane);
            }
            _ => {}
        }
        self.with_picture(pane, |pp| match command {
            Command::Tool(tool) => pp.markup.tool = tool,
            Command::Color(color) => pp.markup.color = color,
            Command::Undo => {
                pp.markup.undo();
            }
            Command::Redo => {
                pp.markup.redo();
            }
            Command::Cancel => {
                pp.text = None;
                pp.drawing = false;
                pp.markup.cancel();
            }
            Command::Done | Command::Type(_) | Command::Back | Command::PutText => {}
        });
    }

    /// Text from the keys or the IME for the text that is typed.
    pub(super) fn type_text(&mut self, pane: PaneId, text: &str) {
        self.with_picture(pane, |pp| {
            if let Some((_, t)) = &mut pp.text {
                t.push_str(text);
            }
        });
        self.show_text(pane);
    }

    /// The IME gave text: it goes into a text that is typed in the active picture pane. `true` when it did.
    pub(super) fn markup_ime(&mut self, text: &str) -> bool {
        let Some(pane) = self.active_picture() else {
            return false;
        };
        self.type_text(pane, text);
        true
    }

    /// The text that is typed in `pane`: where, what, and its size in pixels.
    fn typed_text(&self, pane: PaneId) -> Option<(Point, String, f32)> {
        let pp = self
            .running
            .as_ref()?
            .panes
            .get(&pane)?
            .picture_pane
            .as_ref()?;
        pp.text
            .clone()
            .map(|(at, t)| (at, t, pp.markup.text_size()))
    }

    /// The text that is typed becomes a mark: it is drawn once more, without the caret.
    fn put_text(&mut self, pane: PaneId) {
        let Some((at, text, size)) = self.typed_text(pane) else {
            return;
        };
        let mask = self.text_mask(&text, size);
        self.with_picture(pane, |pp| {
            pp.text = None;
            match mask {
                Some(mask) => {
                    pp.markup.text_draft(at, text, mask);
                    pp.markup.commit_draft();
                }
                None => {
                    pp.markup.cancel();
                }
            }
        });
    }

    /// Draws the text that is typed (with a caret) as the draft of the marks.
    fn show_text(&mut self, pane: PaneId) {
        let Some((at, text, size)) = self.typed_text(pane) else {
            return;
        };
        let mask = self.text_mask(&format!("{text}|"), size);
        self.with_picture(pane, |pp| match mask {
            Some(mask) => pp.markup.text_draft(at, text, mask),
            None => {
                pp.markup.cancel();
            }
        });
    }

    /// The mask of a line of text for a picture (the fonts are loaded the first time).
    fn text_mask(&mut self, text: &str, size: f32) -> Option<TextMask> {
        if self.markup_fonts.is_none() {
            self.markup_fonts = fterm_render::font::Fonts::new(16.0)
                .inspect_err(|err| tracing::warn!("no fonts for text on pictures: {err:#}"))
                .ok();
        }
        let image = self.markup_fonts.as_mut()?.rasterize_line(text, size)?;
        let alpha = match image.kind {
            fterm_render::font::ImageKind::Mask => image.data,
            fterm_render::font::ImageKind::Color => image.data.chunks(4).map(|px| px[3]).collect(),
        };
        Some(TextMask {
            width: image.width,
            height: image.height,
            alpha,
        })
    }

    /// The left button in a picture pane: a mark starts or ends. `true` when it was for a picture pane.
    pub(super) fn markup_mouse(&mut self, state: ElementState) -> bool {
        let (mx, my) = (self.mouse.position.0 as f32, self.mouse.position.1 as f32);
        match state {
            ElementState::Pressed => {
                let Some((pane, rect)) = self.picture_under_mouse() else {
                    return false;
                };
                // A text that is typed is put first (without its caret).
                self.put_text(pane);
                let mut text = false;
                self.with_picture(pane, |pp| {
                    let p = image_point((mx, my), rect, (pp.shown.1.width, pp.shown.1.height));
                    if pp.markup.tool == Tool::Text {
                        // A new text starts there.
                        pp.text = Some((p, String::new()));
                        text = true;
                    } else {
                        pp.markup.press(p);
                        pp.drawing = true;
                    }
                });
                if text {
                    self.show_text(pane);
                }
                true
            }
            ElementState::Released => {
                let Some(pane) = self.drawing_picture() else {
                    return false;
                };
                let rect = self.picture_rect(pane);
                self.with_picture(pane, |pp| {
                    if let Some(rect) = rect {
                        let p = image_point((mx, my), rect, (pp.shown.1.width, pp.shown.1.height));
                        pp.markup.release(p);
                    }
                    pp.drawing = false;
                });
                true
            }
        }
    }

    /// The picture pane where the mouse draws now.
    pub(super) fn drawing_picture(&self) -> Option<PaneId> {
        let running = self.running.as_ref()?;
        running
            .panes
            .iter()
            .find_map(|(id, p)| p.picture_pane.as_ref().filter(|pp| pp.drawing).map(|_| *id))
    }

    /// The mouse moved: the mark that is drawn follows it. `true` when a mark is drawn.
    pub(super) fn markup_drag(&mut self) -> bool {
        let Some(pane) = self.drawing_picture() else {
            return false;
        };
        let Some(rect) = self.picture_rect(pane) else {
            return false;
        };
        let (mx, my) = (self.mouse.position.0 as f32, self.mouse.position.1 as f32);
        self.with_picture(pane, |pp| {
            let p = image_point((mx, my), rect, (pp.shown.1.width, pp.shown.1.height));
            pp.markup.drag(p);
        });
        true
    }

    /// The mouse is over a picture (a crosshair cursor).
    pub(super) fn over_picture(&self) -> bool {
        self.picture_under_mouse().is_some_and(|(_, rect)| {
            let (x, y) = (self.mouse.position.0 as f32, self.mouse.position.1 as f32);
            rect.contains(x, y)
        })
    }

    /// Enter in a picture pane: the picture with its marks goes to a file and the clipboard, its path is pasted
    /// into the pane that it was opened next to, and the picture pane closes.
    pub(super) fn finish_markup(&mut self, pane: PaneId) {
        let Some((picture, origin)) = self.running.as_mut().and_then(|r| {
            let pp = r.panes.get_mut(&pane)?.picture_pane.as_mut()?;
            pp.markup.commit_draft();
            Some((pp.markup.render(), pp.origin))
        }) else {
            return;
        };
        let path = match save_picture(&picture) {
            Ok(path) => path,
            Err(err) => {
                return self.notify(
                    None,
                    &tr!("toast.picture_failed"),
                    &err,
                    Level::Error,
                    Source::App,
                );
            }
        };
        let copied = self.clipboard.set_picture(&picture);
        let shown = path.display().to_string();
        let pasted = origin.filter(|o| {
            self.running.as_ref().is_some_and(|r| {
                r.panes.get(o).is_some_and(|p| {
                    p.picture_pane.is_none() && p.scene.is_none() && p.review.is_none()
                })
            })
        });
        if let Some(origin) = pasted
            && let Some(p) = self.running.as_ref().and_then(|r| r.panes.get(&origin))
        {
            let text = if shown.contains(' ') {
                format!("\"{shown}\" ")
            } else {
                format!("{shown} ")
            };
            let bracketed = p
                .session
                .with_term(|t| t.mode().contains(TermMode::BRACKETED_PASTE));
            p.session
                .write(crate::clipboard::paste_bytes(&text, bracketed));
        }
        let body = match (pasted, copied) {
            (Some(_), true) => tr!("toast.markup_saved_pasted", path = shown),
            (Some(_), false) => tr!("toast.markup_saved_pasted_no_clipboard", path = shown),
            (None, true) => tr!("toast.markup_saved_copied", path = shown),
            (None, false) => shown.clone(),
        };
        self.notify(
            None,
            &tr!("toast.markup_saved"),
            &body,
            Level::Success,
            Source::App,
        );
        self.close_pane_now(pane);
        if let Some(origin) = pasted
            && let Some(running) = &mut self.running
        {
            running.mux.focus(origin);
        }
        self.tab_changed();
    }
}

/// Writes the picture as PNG to `data_dir/shots` (or the temp folder). Gives its path.
fn save_picture(picture: &fterm_term::picture::Picture) -> Result<std::path::PathBuf, String> {
    let dir = crate::paths::folder(
        None,
        crate::paths::data_dir().as_deref(),
        "shots",
        Some(std::env::temp_dir()),
    )
    .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let path = dir.join(format!("markup-{}.png", now_ms()));
    image::save_buffer_with_format(
        &path,
        &picture.rgba,
        picture.width,
        picture.height,
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(c: &str) -> Key {
        Key::Character(c.into())
    }

    #[test]
    fn keys_pick_tools_and_colors() {
        let cmd = |key: &Key, text: Option<&str>| command(key, text, false, false, false);
        assert_eq!(cmd(&ch("r"), Some("r")), Some(Command::Tool(Tool::Rect)));
        assert_eq!(cmd(&ch("A"), Some("A")), Some(Command::Tool(Tool::Arrow)));
        assert_eq!(cmd(&ch("h"), Some("h")), Some(Command::Tool(Tool::Hide)));
        assert_eq!(cmd(&ch("3"), Some("3")), Some(Command::Color(2)));
        assert_eq!(cmd(&ch("5"), Some("5")), None);
        assert_eq!(cmd(&ch("x"), Some("x")), None);
        assert_eq!(cmd(&Key::Named(NamedKey::Enter), None), Some(Command::Done));
        assert_eq!(
            cmd(&Key::Named(NamedKey::Escape), None),
            Some(Command::Cancel)
        );
    }

    #[test]
    fn ctrl_z_and_ctrl_y() {
        assert_eq!(
            command(&ch("z"), Some("\u{1a}"), true, false, false),
            Some(Command::Undo)
        );
        assert_eq!(
            command(&ch("y"), None, true, false, false),
            Some(Command::Redo)
        );
        assert_eq!(
            command(&ch("r"), Some("r"), true, false, false),
            None,
            "Ctrl+R is not a tool"
        );
    }

    #[test]
    fn while_text_is_typed_keys_go_into_it() {
        let cmd = |key: &Key, text: Option<&str>| command(key, text, false, false, true);
        assert_eq!(cmd(&ch("r"), Some("r")), Some(Command::Type("r".into())));
        assert_eq!(cmd(&ch("ж"), Some("ж")), Some(Command::Type("ж".into())));
        assert_eq!(
            cmd(&Key::Named(NamedKey::Enter), None),
            Some(Command::PutText)
        );
        assert_eq!(
            cmd(&Key::Named(NamedKey::Backspace), None),
            Some(Command::Back)
        );
        assert_eq!(
            cmd(&Key::Named(NamedKey::Escape), None),
            Some(Command::Cancel)
        );
    }

    #[test]
    fn the_mouse_in_pixels_of_the_picture() {
        // The picture (400×200) is drawn in a rect of 200×100 at (10, 20).
        let rect = Rect::new(10.0, 20.0, 200.0, 100.0);
        assert_eq!(image_point((110.0, 70.0), rect, (400, 200)), (200.0, 100.0));
        assert_eq!(image_point((10.0, 20.0), rect, (400, 200)), (0.0, 0.0));
        // Out of the picture: at its edge.
        assert_eq!(image_point((500.0, -5.0), rect, (400, 200)), (400.0, 0.0));
    }

    #[test]
    fn the_picture_leaves_a_row_for_the_tool_bar() {
        // A wide picture in 80×24 cells of 10×20 px: as wide as the pane.
        assert_eq!(
            picture_cells((2000, 1000), (10.0, 20.0), (80, 24)),
            (80, 20)
        );
        // A high one: as high as 23 rows (460 px).
        assert_eq!(
            picture_cells((1000, 1000), (10.0, 20.0), (80, 24)),
            (46, 23)
        );
    }
}
