//! Dialog panes: a Review or a form that an agent (or a script) shows to the user, and waits for the answer.
//! They have no program: the app draws their model into the pane and gives them the keys.

use std::time::Instant;

use fterm_api::RpcError;
use fterm_api::server::ClientId;
use fterm_config::load::{ReviewFocus, ReviewPlace};
use serde_json::Value;
use winit::keyboard::{Key, NamedKey, PhysicalKey};

use super::form_calls::FormPane;
use super::review_calls::ReviewPane;
use super::*;

/// A call that waits for the user in a dialog pane (`review`, `form`).
pub(super) struct PendingDialog {
    pub pane: PaneId,
    pub kind: DialogKind,
    pub client: ClientId,
    pub deadline: Instant,
    pub reply: std::sync::mpsc::Sender<Result<Value, RpcError>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DialogKind {
    Review,
    Form,
}

impl DialogKind {
    /// The answer of a dialog that ended with no answer of the user, when its pane is gone already.
    pub(super) fn cancelled(self) -> Value {
        match self {
            DialogKind::Review => crate::review::Review::new("", "", Vec::new())
                .result(crate::review::Decision::Cancelled),
            DialogKind::Form => fterm_ui::form::Form::cancelled(),
        }
    }
}

/// When the dialog of a call that came at `asked` ends. `None` when its time is over already (the call waited for
/// the access question, and its client stopped waiting): then no dialog opens.
pub(super) fn dialog_deadline(
    asked: Instant,
    timeout: std::time::Duration,
    now: Instant,
) -> Option<Instant> {
    let deadline = asked + timeout;
    (deadline > now).then_some(deadline)
}

/// What a new dialog pane shows.
pub(super) enum Dialog {
    Review(crate::review::Review),
    Form(Box<fterm_ui::form::Form>),
}

impl App {
    /// A new dialog pane at the pane `near` of the agent that asked, by `plan_review.place`: an overlay over it
    /// (else a split), a split next to it (else a tab), or a tab right after its tab. It takes the keys by
    /// `plan_review.focus`: with `auto`, when the user asked for it (R) or looks at `near`.
    pub(super) fn place_dialog(
        &mut self,
        event_loop: &ActiveEventLoop,
        dialog: Dialog,
        title: String,
        near: Option<PaneId>,
        asked_by_user: bool,
    ) -> Option<PaneId> {
        let focused = self.focused;
        let settings = self.config.config.review;
        let overlay_size = settings.size.unwrap_or(self.config.config.overlay_size);
        let running = self.running.as_mut()?;
        let size = running.grid_for(running.tab_area());
        let id = running.mux.new_pane_id();
        let (review, form, waiting) = match dialog {
            Dialog::Review(review) => (
                Some(ReviewPane {
                    review,
                    drawn: None,
                }),
                None,
                tr!("review.waiting"),
            ),
            Dialog::Form(form) => (
                None,
                Some(FormPane {
                    form: *form,
                    drawn: None,
                }),
                tr!("form.waiting"),
            ),
        };
        running.panes.insert(
            id,
            Pane {
                session: Session::scene(size),
                app_title: Some(title.clone()),
                shell: ShellState::default(),
                agent: None,
                agent_hooks: false,
                agent_session: None,
                agent_info: Default::default(),
                agent_row: None,
                pictures: Default::default(),
                picture_pane: None,
                identity: Default::default(),
                last_command: None,
                remote: true,
                opened_by: None,
                profile: None,
                rerun: None,
                intro_lines: 0,
                scene: None,
                review,
                form,
                harmonize: false,
                palette_changes: Some(false),
                mux: None,
            },
        );
        let looking = focused
            && near.is_some()
            && (running.mux.active_pane() == near || running.mux.input_pane() == near);
        let near = near.filter(|p| running.panes.contains_key(p));
        let mut place = settings.place;
        let mut placed = false;
        if let Some(near) = near {
            if place == ReviewPlace::Overlay {
                placed = running.mux.open_overlay_on(near, id, overlay_size);
                if !placed {
                    place = ReviewPlace::Split;
                }
            }
            if place == ReviewPlace::Split && !placed {
                placed = running
                    .mux
                    .split_pane(near, id, fterm_mux::Direction::Right);
            }
            if !placed {
                running.mux.add_tab_after(
                    near,
                    fterm_mux::Layout::Pane(id),
                    id,
                    Some(title.clone()),
                );
                placed = true;
            }
        }
        if !placed {
            running
                .mux
                .add_tab(fterm_mux::Layout::Pane(id), id, Some(title.clone()));
        }
        running.resize_all_panes();
        let go = match settings.focus {
            ReviewFocus::Always => true,
            ReviewFocus::Auto => asked_by_user || looking,
            ReviewFocus::Never => false,
        };
        // An overlay dialog has the keys only while the pane under it (the agent's) is the active pane, so it
        // never takes the keys of another pane that you type in. Going to it goes to that pane.
        if go {
            self.go_to_pane(id);
        }
        tracing::info!(pane = id.0, %title, "new dialog");
        self.draw_dialogs();
        // A yellow dot and a notification: the user must act.
        self.agent_state(event_loop, id, "waiting", &waiting);
        self.tab_changed();
        Some(id)
    }

    /// The answer of a dialog that closes with no answer of the user.
    fn dialog_cancelled(&self, pane: PaneId) -> Option<Value> {
        let p = self.running.as_ref()?.panes.get(&pane)?;
        if let Some(rp) = &p.review {
            return Some(rp.review.result(crate::review::Decision::Cancelled));
        }
        p.form.as_ref().map(|_| fterm_ui::form::Form::cancelled())
    }

    /// The calls that wait for the dialog `pane` (they leave the list).
    fn take_dialog_calls(&mut self, pane: PaneId) -> Vec<PendingDialog> {
        let (done, waiting): (Vec<PendingDialog>, Vec<PendingDialog>) =
            std::mem::take(&mut self.dialogs)
                .into_iter()
                .partition(|d| d.pane == pane);
        self.dialogs = waiting;
        done
    }

    /// Sends the answer of a dialog to the client that waits, and closes its pane.
    pub(super) fn finish_dialog(&mut self, pane: PaneId, answer: Value) {
        for d in self.take_dialog_calls(pane) {
            let _ = d.reply.send(Ok(answer.clone()));
        }
        self.close_dialog_pane(pane);
    }

    /// Closes a dialog pane, unless it is the last tab (then the window would close).
    fn close_dialog_pane(&mut self, pane: PaneId) {
        // A dialog in an overlay or a split always goes; a dialog tab only when it is not the last tab.
        let many = self.running.as_ref().is_some_and(|r| {
            r.mux.all_tabs().count() > 1
                || r.mux
                    .all_tabs()
                    .any(|t| t.overlay.is_some_and(|o| o.pane == pane))
                || r.mux
                    .all_tabs()
                    .any(|t| t.layout.contains(pane) && t.layout.panes().len() > 1)
        });
        if many {
            self.close_pane_now(pane);
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// A dialog pane closed: the client gets "cancelled".
    pub(super) fn dialog_pane_closed(&mut self, pane: PaneId) {
        let answer = self.dialog_cancelled(pane);
        for d in self.take_dialog_calls(pane) {
            let answer = answer.clone().unwrap_or_else(|| d.kind.cancelled());
            let _ = d.reply.send(Ok(answer));
        }
    }

    /// The client stopped waiting (for example the agent was stopped): its dialog panes go.
    pub(super) fn dialog_client_gone(&mut self, client: ClientId) {
        let (gone, waiting): (Vec<PendingDialog>, Vec<PendingDialog>) =
            std::mem::take(&mut self.dialogs)
                .into_iter()
                .partition(|d| d.client == client);
        self.dialogs = waiting;
        for d in gone {
            // A client that only closed its writing side still reads: it gets "cancelled", not a time-out error.
            let answer = self
                .dialog_cancelled(d.pane)
                .unwrap_or_else(|| d.kind.cancelled());
            let _ = d.reply.send(Ok(answer));
            self.close_dialog_pane(d.pane);
        }
    }

    /// Dialogs whose time is over: "cancelled" with `timed_out`, and the pane goes. Gives the next deadline.
    pub(super) fn expire_dialogs(&mut self, now: Instant) -> Option<Instant> {
        let over: Vec<PaneId> = self
            .dialogs
            .iter()
            .filter(|d| d.deadline <= now)
            .map(|d| d.pane)
            .collect();
        for pane in over {
            let answer = self.dialog_cancelled(pane);
            for d in self.take_dialog_calls(pane) {
                let mut answer = answer.clone().unwrap_or_else(|| d.kind.cancelled());
                answer["timed_out"] = true.into();
                let _ = d.reply.send(Ok(answer));
            }
            self.close_dialog_pane(pane);
        }
        self.dialogs.iter().map(|d| d.deadline).min()
    }

    /// Draws the dialogs that are new, changed, changed size, or got new colors.
    pub(super) fn draw_dialogs(&mut self) {
        let ui = self.ui;
        let Some(running) = self.running.as_mut() else {
            return;
        };
        for p in running.panes.values_mut() {
            let size = p.session.grid_size();
            if let Some(rp) = p.review.as_mut()
                && rp.drawn != Some(size)
            {
                let text = crate::review::render(&mut rp.review, size.columns, size.rows, &ui);
                p.session.feed(text.as_bytes());
                rp.drawn = Some(size);
            }
            if let Some(fp) = p.form.as_mut()
                && fp.drawn != Some(size)
            {
                let text =
                    super::form_calls::render_form(&mut fp.form, size.columns, size.rows, &ui);
                p.session.feed(text.as_bytes());
                fp.drawn = Some(size);
            }
        }
    }

    /// Draws every dialog again (for example with a new theme).
    pub(super) fn redraw_dialogs(&mut self) {
        if let Some(running) = self.running.as_mut() {
            for p in running.panes.values_mut() {
                if let Some(rp) = p.review.as_mut() {
                    rp.drawn = None;
                }
                if let Some(fp) = p.form.as_mut() {
                    fp.drawn = None;
                }
            }
        }
        self.draw_dialogs();
    }
}

/// What a key does in a dialog pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DialogInput {
    Key(fterm_ui::key::Key<'static>),
    /// Typed text (no control chars).
    Text(String),
    /// Paste the clipboard (Ctrl+V; Ctrl+Shift+V and Shift+Insert come through the `paste` action).
    Paste,
}

/// The input of a dialog for a key from winit. Ctrl+S and Ctrl+V work with any keyboard layout (Ctrl+V by the place
/// of the key).
pub(super) fn dialog_input(
    key: &Key,
    physical: PhysicalKey,
    text: Option<&str>,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Option<DialogInput> {
    use fterm_ui::key::Key as U;
    if super::overlay_calls::is_paste(key, physical, ctrl, shift) {
        return Some(DialogInput::Paste);
    }
    // Ctrl+S: the letter of the layout (Dvorak too); the place of the key when that letter is not Latin.
    let letter = match key {
        Key::Character(c) if c.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) => {
            c.chars().next().map(|c| c.to_ascii_lowercase())
        }
        _ => crate::input::physical_letter(physical),
    };
    if ctrl && !shift && !alt && letter == Some('s') {
        return Some(DialogInput::Key(U::Save));
    }
    let k = match key {
        Key::Named(NamedKey::ArrowUp) => U::Up,
        Key::Named(NamedKey::ArrowDown) => U::Down,
        Key::Named(NamedKey::PageUp) => U::PageUp,
        Key::Named(NamedKey::PageDown) => U::PageDown,
        Key::Named(NamedKey::Home) => U::Home,
        Key::Named(NamedKey::End) => U::End,
        Key::Named(NamedKey::ArrowLeft) => U::Left,
        Key::Named(NamedKey::ArrowRight) => U::Right,
        Key::Named(NamedKey::Enter) => U::Enter { ctrl, shift },
        Key::Named(NamedKey::Escape) => U::Escape,
        Key::Named(NamedKey::Backspace) => U::Backspace,
        Key::Named(NamedKey::Delete) => U::Delete,
        Key::Named(NamedKey::Tab) if !ctrl && !alt => {
            if shift {
                U::BackTab
            } else {
                U::Tab
            }
        }
        _ => {
            let text: String = text
                .filter(|_| !ctrl && !alt)?
                .chars()
                .filter(|c| !c.is_control())
                .collect();
            return (!text.is_empty()).then_some(DialogInput::Text(text));
        }
    };
    Some(DialogInput::Key(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fterm_ui::key::Key as U;
    use winit::keyboard::KeyCode;

    fn input(key: Key, code: KeyCode, text: Option<&str>, mods: &str) -> Option<DialogInput> {
        dialog_input(
            &key,
            PhysicalKey::Code(code),
            text,
            mods.contains("ctrl"),
            mods.contains("shift"),
            mods.contains("alt"),
        )
    }

    fn named(k: NamedKey) -> Key {
        Key::Named(k)
    }

    #[test]
    fn the_time_of_a_dialog_counts_from_the_call() {
        let now = Instant::now();
        let secs = std::time::Duration::from_secs;
        assert_eq!(dialog_deadline(now, secs(5), now), Some(now + secs(5)));
        // The call waited 10 s for the access question: 5 s are left.
        assert_eq!(
            dialog_deadline(now, secs(15), now + secs(10)),
            Some(now + secs(15))
        );
        assert_eq!(
            dialog_deadline(now, secs(5), now + secs(10)),
            None,
            "its time is over: no dialog"
        );
    }

    #[test]
    fn a_dialog_with_no_pane_still_answers_in_its_own_form() {
        assert_eq!(
            DialogKind::Form.cancelled(),
            serde_json::json!({ "status": "cancelled", "changed": [] })
        );
        let review = DialogKind::Review.cancelled();
        assert_eq!(review["decision"], "cancelled");
        assert!(review["feedback"].is_string() && review["items"].is_array());
    }

    #[test]
    fn dialog_keys_from_winit() {
        let key = |k| Some(DialogInput::Key(k));
        assert_eq!(
            input(
                Key::Character("s".into()),
                KeyCode::KeyS,
                Some("\u{13}"),
                "ctrl"
            ),
            key(U::Save)
        );
        assert_eq!(
            input(Key::Character("ы".into()), KeyCode::KeyS, None, "ctrl"),
            key(U::Save),
            "Ctrl+S by the place of the key (another layout)"
        );
        assert_eq!(
            input(Key::Character("s".into()), KeyCode::Semicolon, None, "ctrl"),
            key(U::Save),
            "Dvorak: the letter s is on another key"
        );
        assert_eq!(
            input(Key::Character("o".into()), KeyCode::KeyS, None, "ctrl"),
            None,
            "Dvorak: Ctrl+O is not Ctrl+S"
        );
        assert_eq!(
            input(named(NamedKey::Enter), KeyCode::Enter, Some("\r"), "ctrl"),
            key(U::Enter {
                ctrl: true,
                shift: false
            })
        );
        assert_eq!(
            input(named(NamedKey::Enter), KeyCode::Enter, Some("\r"), "shift"),
            key(U::Enter {
                ctrl: false,
                shift: true
            })
        );
        assert_eq!(
            input(named(NamedKey::Tab), KeyCode::Tab, Some("\t"), ""),
            key(U::Tab)
        );
        assert_eq!(
            input(named(NamedKey::Tab), KeyCode::Tab, Some("\t"), "shift"),
            key(U::BackTab)
        );
        assert_eq!(
            input(named(NamedKey::Escape), KeyCode::Escape, None, ""),
            key(U::Escape)
        );
        assert_eq!(
            input(named(NamedKey::ArrowUp), KeyCode::ArrowUp, None, ""),
            key(U::Up)
        );
        assert_eq!(
            input(named(NamedKey::Delete), KeyCode::Delete, None, ""),
            key(U::Delete)
        );
        assert_eq!(
            input(Key::Character("v".into()), KeyCode::KeyV, None, "ctrl"),
            Some(DialogInput::Paste)
        );
        assert_eq!(
            input(named(NamedKey::Insert), KeyCode::Insert, None, "shift"),
            Some(DialogInput::Paste)
        );
        assert_eq!(
            input(Key::Character("a".into()), KeyCode::KeyA, Some("a"), ""),
            Some(DialogInput::Text("a".into()))
        );
        assert_eq!(
            input(named(NamedKey::Space), KeyCode::Space, Some(" "), ""),
            Some(DialogInput::Text(" ".into()))
        );
        assert_eq!(
            input(
                Key::Character("a".into()),
                KeyCode::KeyA,
                Some("\u{1}"),
                "ctrl"
            ),
            None,
            "Ctrl+A: nothing"
        );
        assert_eq!(
            input(Key::Character("a".into()), KeyCode::KeyA, Some("a"), "alt"),
            None,
            "Alt+A: nothing"
        );
    }
}
