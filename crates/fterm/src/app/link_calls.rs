//! Links: Ctrl + click, the link under the mouse while Ctrl is down (underlined, a hand cursor), the hint labels
//! (`open_link_hints`), and the question before a risky link opens.

use fterm_term::alacritty_terminal::index::{Column, Line};
use fterm_term::links::{Link, link_at, links_in};

use super::*;
use crate::link_open::{Narrow, Open, decide, labels, narrow};

/// "Open this link?" for a link that may not be what it shows.
pub(super) struct LinkQuestion {
    pub link: Link,
    pub lines: Vec<String>,
}

/// The hint labels on the links of a pane.
pub(super) struct Hints {
    pub pane: PaneId,
    pub links: Vec<(String, Link)>,
    pub typed: String,
}

/// What a pane shows of the links: cells to underline, and labels `(text, column, row)`.
#[derive(Default)]
pub(super) struct LinkMarks {
    pub pane: Option<PaneId>,
    pub underline: Vec<(Point, Point)>,
    pub labels: Vec<(String, usize, usize)>,
}

impl App {
    /// The link under the mouse in the active pane.
    fn link_under_mouse(&self) -> Option<(PaneId, Link)> {
        let running = self.running.as_ref()?;
        let pane = running.mux.active_pane()?;
        if self.pane_under_mouse() != Some(pane) {
            return None;
        }
        let session = running.session()?;
        let (point, _) = self.mouse_cell(running);
        session
            .with_term(|term| link_at(term, point))
            .map(|link| (pane, link))
    }

    /// While Ctrl is down, the link under the mouse is underlined and the cursor is a hand.
    /// True when the mouse is on a link.
    pub(super) fn update_link_hover(&mut self) -> bool {
        let hover = if self.mods.control_key() {
            self.link_under_mouse()
        } else {
            None
        };
        let on = hover.is_some();
        let was = self.mouse.link_hover.is_some();
        if hover != self.mouse.link_hover {
            self.mouse.link_hover = hover;
            if let Some(running) = &self.running {
                running.window.request_redraw();
            }
        }
        if let Some(running) = &self.running {
            if on {
                running.window.set_cursor(CursorIcon::Pointer);
            } else if was {
                running.window.set_cursor(CursorIcon::Default);
            }
        }
        on
    }

    /// Ctrl + click on a link. True when there was a link.
    pub(super) fn open_link_under_mouse(&mut self) -> bool {
        let Some((_, link)) = self.link_under_mouse() else {
            return false;
        };
        self.open_link(link);
        true
    }

    /// Opens a link: at once, after a question, or not at all (see `link_open::decide`).
    pub(super) fn open_link(&mut self, link: Link) {
        match decide(&link) {
            Open::Now => self.open_url(&link.url),
            Open::Refuse => {
                tracing::warn!(url = %link.url, "a link with a scheme that fterm does not open");
                self.notify(
                    None,
                    &tr!("toast.link_refused"),
                    &link.url,
                    Level::Warning,
                    Source::App,
                );
            }
            Open::Ask => {
                let mut lines = vec![tr!("box.link_open"), String::new(), link.url.clone()];
                if link.osc8 && link.text != link.url {
                    lines.push(tr!("box.link_text", text = link.text));
                }
                lines.push(String::new());
                lines.push(tr!("box.link_keys"));
                self.link_question = Some(LinkQuestion { link, lines });
                if let Some(running) = &self.running {
                    running.window.request_redraw();
                }
            }
        }
    }

    fn open_url(&mut self, url: &str) {
        tracing::info!(%url, "open link");
        if let Err(err) = open::that_detached(url) {
            tracing::warn!("cannot open {url}: {err}");
            self.notify(
                None,
                &tr!("toast.link_error"),
                &format!("{url}: {err}"),
                Level::Error,
                Source::App,
            );
        }
    }

    /// Keys while the link question is open: Enter opens, C copies, Esc does nothing.
    pub(super) fn link_question_key(&mut self, event: &KeyEvent) {
        let copy = physical_letter(event.physical_key) == Some('c');
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => {
                if let Some(q) = self.link_question.take() {
                    self.open_url(&q.link.url);
                }
            }
            Key::Named(NamedKey::Escape) => self.link_question = None,
            _ if copy => {
                if let Some(q) = self.link_question.take() {
                    self.copy_text(q.link.url);
                }
            }
            _ => return,
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// `open_link_hints`: labels on the links of the screen of the active pane (again: off).
    pub(super) fn toggle_link_hints(&mut self) {
        if self.hints.take().is_some() {
            if let Some(running) = &self.running {
                running.window.request_redraw();
            }
            return;
        }
        let Some(running) = &self.running else {
            return;
        };
        let (Some(pane), Some(session)) = (running.mux.active_pane(), running.session()) else {
            return;
        };
        let links = session.with_term(|term| {
            let top = Line(-(term.grid().display_offset() as i32));
            let bottom = top + (term.screen_lines() as i32 - 1);
            links_in(term, top, bottom)
        });
        if links.is_empty() {
            return self.notify(None, &tr!("toast.no_links"), "", Level::Info, Source::App);
        }
        let names = labels(links.len());
        self.hints = Some(Hints {
            pane,
            links: names.into_iter().zip(links).collect(),
            typed: String::new(),
        });
        running.window.request_redraw();
    }

    /// Keys while the labels are on: letters choose a label (Shift = copy), Backspace, Esc or Ctrl + a key = off.
    pub(super) fn hints_key(&mut self, event: &KeyEvent) {
        let Some(hints) = self.hints.as_mut() else {
            return;
        };
        let letter = physical_letter(event.physical_key);
        match (&event.logical_key, letter) {
            (Key::Named(NamedKey::Escape), _) => self.hints = None,
            _ if self.mods.control_key() || self.mods.alt_key() => self.hints = None,
            (Key::Named(NamedKey::Backspace), _) => {
                hints.typed.pop();
            }
            (_, Some(c)) => {
                hints.typed.push(c);
                let names: Vec<String> = hints.links.iter().map(|(l, _)| l.clone()).collect();
                match narrow(&names, &hints.typed) {
                    Narrow::Many => {}
                    Narrow::None => self.hints = None,
                    Narrow::One(i) => {
                        let link = hints.links.swap_remove(i).1;
                        self.hints = None;
                        if self.mods.shift_key() {
                            self.copy_text(link.url);
                        } else {
                            self.open_link(link);
                        }
                    }
                }
            }
            // Shift and other keys alone do nothing.
            _ => return,
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// What to draw for links now: the link under the mouse, or the labels and their links.
    pub(super) fn link_marks(&self) -> LinkMarks {
        let Some(running) = &self.running else {
            return LinkMarks::default();
        };
        if let Some(hints) = &self.hints {
            let Some(pane) = running.panes.get(&hints.pane) else {
                return LinkMarks::default();
            };
            let (offset, rows) = pane.session.with_term(|term| {
                (
                    term.grid().display_offset() as i32,
                    term.screen_lines() as i32,
                )
            });
            let mut marks = LinkMarks {
                pane: Some(hints.pane),
                ..LinkMarks::default()
            };
            for (name, link) in &hints.links {
                if !name.starts_with(&hints.typed) {
                    continue;
                }
                marks.underline.push((link.start, link.end));
                // A link that starts above the screen gets its label at the top row.
                let row = link.start.line.0 + offset;
                let (row, col) = if row < 0 {
                    (0, Column(0))
                } else {
                    (row, link.start.column)
                };
                if row < rows {
                    marks.labels.push((name.clone(), col.0, row as usize));
                }
            }
            return marks;
        }
        match &self.mouse.link_hover {
            Some((pane, link)) => LinkMarks {
                pane: Some(*pane),
                underline: vec![(link.start, link.end)],
                labels: Vec::new(),
            },
            None => LinkMarks::default(),
        }
    }
}
