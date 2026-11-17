//! A form pane: an agent (or a script) asks the user to edit some values, and waits for the answer. The model
//! is `fterm_ui::form::Form`; the app draws it, gives it the keys, and sends the answer.

use std::time::Instant;

use fterm_api::RpcError;
use fterm_render::theme::UiColors;
use fterm_ui::field::{Hint, Problem, Word};
use fterm_ui::form::{Form, Outcome};
use fterm_ui::key::Key as FormKey;

use super::dialog_calls::{
    Dialog, DialogInput, DialogKind, PendingDialog, dialog_deadline, dialog_input,
};
use super::*;
use crate::api::ApiRequest;

/// A form in a pane, and the grid size it was last drawn for.
pub(super) struct FormPane {
    pub form: Form,
    pub drawn: Option<GridSize>,
}

impl App {
    /// `form`: shows a form at the pane of the agent. The answer comes when the user saves or cancels, closes
    /// the pane, or the time ends.
    pub(super) fn api_form(&mut self, event_loop: &ActiveEventLoop, request: ApiRequest) {
        let client = self.api_clients.get(&request.client);
        let name = client.map_or(String::new(), |c| c.name.clone());
        let near = match crate::api::pane_param(&request.params, "pane") {
            Ok(pane) => pane.map(PaneId).or(client.and_then(|c| c.pane)),
            Err(err) => {
                let _ = request.reply.send(Err(err));
                return;
            }
        };
        let asked = match crate::api::form_param(&request.params, &name) {
            Ok(asked) => asked,
            Err(err) => {
                let _ = request.reply.send(Err(err));
                return;
            }
        };
        let Some(deadline) = dialog_deadline(request.asked, asked.timeout, Instant::now()) else {
            // It waited for the access question until its time was over: its client stopped waiting.
            let mut answer = Form::cancelled();
            answer["timed_out"] = true.into();
            let _ = request.reply.send(Ok(answer));
            return;
        };
        let title = tr!("form.tab", title = asked.form.title);
        let dialog = Dialog::Form(Box::new(asked.form));
        let Some(pane) = self.place_dialog(event_loop, dialog, title, near, false) else {
            let _ = request
                .reply
                .send(Err(RpcError::new(RpcError::INTERNAL, "no window")));
            return;
        };
        self.dialogs.push(PendingDialog {
            pane,
            kind: DialogKind::Form,
            client: request.client,
            deadline,
            reply: request.reply,
        });
    }

    /// The pane with the keys is a form (in a tab, a split, or an overlay).
    pub(super) fn active_form(&self) -> Option<PaneId> {
        let running = self.running.as_ref()?;
        let pane = running.mux.input_pane()?;
        running.panes.get(&pane)?.form.as_ref().map(|_| pane)
    }

    /// A key in the active form.
    pub(super) fn form_key(&mut self, event: &KeyEvent) {
        let Some(pane) = self.active_form() else {
            return;
        };
        let input = dialog_input(
            &event.logical_key,
            event.physical_key,
            event.text.as_deref(),
            self.mods.control_key(),
            self.mods.shift_key(),
            self.mods.alt_key(),
        );
        match input {
            Some(DialogInput::Key(key)) => self.form_apply(pane, key),
            Some(DialogInput::Text(text)) => self.form_apply(pane, FormKey::Text(&text)),
            Some(DialogInput::Paste) => {
                if let Some(text) = self.clipboard.paste() {
                    self.form_apply(pane, FormKey::Paste(&text));
                }
            }
            None => {}
        }
    }

    /// Text from the clipboard for the form that has the keys (the `paste` action).
    pub(super) fn form_paste(&mut self, pane: PaneId, text: &str) {
        self.form_apply(pane, FormKey::Paste(text));
    }

    /// Text of the input method (CJK) for the form that has the keys. `false` = no form has the keys.
    pub(super) fn form_ime(&mut self, text: &str) -> bool {
        if self.keys_elsewhere() {
            return false;
        }
        let Some(pane) = self.active_form() else {
            return false;
        };
        self.form_apply(pane, FormKey::Text(text));
        true
    }

    /// One key for the form `pane`: draw it again, or send its answer.
    fn form_apply(&mut self, pane: PaneId, key: FormKey) {
        let Some(running) = self.running.as_mut() else {
            return;
        };
        let Some(p) = running.panes.get_mut(&pane) else {
            return;
        };
        let cols = p.session.grid_size().columns;
        let Some(fp) = p.form.as_mut() else {
            return;
        };
        match fp.form.key(key, cols) {
            Outcome::Nothing => return,
            Outcome::Changed => fp.drawn = None,
            Outcome::Save => {
                let answer = fp.form.answer();
                return self.finish_dialog(pane, answer);
            }
            Outcome::Cancel => return self.finish_dialog(pane, Form::cancelled()),
        }
        self.draw_dialogs();
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
}

/// The UI text of a word of a form.
pub(super) fn form_word(word: Word) -> String {
    use fterm_config::tr;
    match word {
        Word::From("") => String::new(),
        Word::From(agent) => tr!("form.from", agent = agent),
        Word::Save => tr!("form.save"),
        Word::Cancel => tr!("form.cancel"),
        Word::NotSet => tr!("form.not_set"),
        Word::Current => tr!("form.current"),
        Word::Problem(problem) => match problem {
            Problem::Required => tr!("form.problem.required"),
            Problem::NoMatch => tr!("form.problem.no_match"),
            Problem::TooLong(max) => tr!("form.problem.too_long", max = max),
            Problem::NotNumber => tr!("form.problem.not_number"),
            Problem::NotInteger => tr!("form.problem.not_integer"),
            Problem::Min(min) => tr!("form.problem.min", min = min),
            Problem::Max(max) => tr!("form.problem.max", max = max),
        },
        Word::Hint(hint) => match hint {
            Hint::Form => tr!("form.keys.form"),
            Hint::Text => tr!("form.keys.text"),
            Hint::Multiline => tr!("form.keys.multiline"),
            Hint::List => tr!("form.keys.list"),
            Hint::EscAgain => tr!("form.keys.esc_again"),
        },
    }
}

/// The form as ANSI text for its grid. The cursor of the grid goes to the cursor of the form, so the window of
/// the input method opens there.
pub(super) fn render_form(form: &mut Form, cols: usize, rows: usize, ui: &UiColors) -> String {
    let screen = form.screen(cols, rows, &form_word);
    let mut text = crate::ui_look::render(&screen.rows, ui);
    if let Some((row, col)) = screen.cursor {
        text.push_str(&format!("\x1b[{};{}H", row + 1, col + 1));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn form_words() {
        assert_eq!(form_word(Word::From("claude")), "from claude");
        assert_eq!(form_word(Word::Save), "Save");
        assert_eq!(form_word(Word::Problem(Problem::Min(6.0))), "At least 6");
        assert_eq!(form_word(Word::Problem(Problem::Max(1.5))), "At most 1.5");
        assert_eq!(
            form_word(Word::Problem(Problem::TooLong(10))),
            "Too long: at most 10 chars"
        );
        assert!(form_word(Word::Hint(Hint::EscAgain)).contains("Esc"));
    }

    #[test]
    fn the_grid_cursor_is_at_the_form_cursor() {
        let request = json!({ "title": "T", "fields": [
            { "id": "a", "type": "checkbox", "value": true },
            { "id": "b", "type": "text", "value": "xyz" },
        ]});
        let mut form = Form::from_json(&request).unwrap();
        form.key(fterm_ui::key::Key::Tab, 40);
        let (row, col) = form
            .screen(40, 12, &form_word)
            .cursor
            .expect("a cursor in a text field");
        let ansi = render_form(&mut form, 40, 12, &UiColors::default());
        assert!(
            ansi.ends_with(&format!("\x1b[{};{}H", row + 1, col + 1)),
            "{ansi:?}"
        );
        assert!(ansi.contains("xyz"));
    }
}
