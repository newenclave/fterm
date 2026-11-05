//! Copy mode in the app: the key table of the config (with two-key bindings), the search prompt,
//! the jumps that wait for a char, Lua actions, and `fterm.copy_mode` from Lua.

use fterm_config::copy_keys::{CopyBinding, CopyCommand, CopyKey, Lookup};
use fterm_config::load::CopyModeIn;
use fterm_term::copy_mode::{CopyAction, CopyResult, Jump};

use super::*;
use crate::ai_chat::InputBox;
use crate::input::{CopyStep, copy_key, copy_step};

/// What the app keeps for the copy mode keys of the active pane.
#[derive(Default)]
pub(super) struct CopyInput {
    /// The keys of a binding typed so far ("i" of "i w").
    typed: Vec<CopyKey>,
    /// `f` `F` `t` `T` wait for a char.
    jump: Option<Jump>,
    /// The search prompt (`/`, `?`).
    pub(super) prompt: Option<InputBox>,
}

impl App {
    /// A key in copy mode (all keys belong to copy mode).
    pub(super) fn copy_mode_key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        if self.copy_input.prompt.is_some() {
            self.copy_prompt_key(event);
            return;
        }
        if let Some(kind) = self.copy_input.jump.take() {
            self.copy_pending(String::new());
            let c = event
                .text
                .as_deref()
                .and_then(|t| t.chars().next())
                .filter(|c| !c.is_control());
            if let Some(c) = c {
                self.copy_run(CopyAction::JumpTo(kind, c));
            }
            return;
        }
        let key = KeyInput {
            logical: &event.logical_key,
            physical: event.physical_key,
            text: event.text.as_deref(),
            mods: self.mods,
        };
        let Some(key) = copy_key(&key) else {
            return;
        };
        self.copy_input.typed.push(key);
        let found = self
            .config
            .config
            .copy_mode
            .keys
            .lookup(&self.copy_input.typed);
        match found {
            Lookup::Prefix => {
                let text: Vec<String> = self.copy_input.typed.iter().map(|k| k.display()).collect();
                self.copy_pending(text.join(" "));
            }
            Lookup::Found(binding) => {
                self.copy_input.typed.clear();
                self.copy_pending(String::new());
                match binding {
                    CopyBinding::Command(command) => self.copy_command(command),
                    CopyBinding::Lua(index) => self.copy_lua(event_loop, index),
                }
            }
            Lookup::None => {
                self.copy_input.typed.clear();
                self.copy_pending(String::new());
            }
        }
    }

    /// A command of the key table, or from `fterm.copy_mode` (copy mode starts first when it is off).
    pub(super) fn copy_command(&mut self, command: CopyCommand) {
        let Some(session) = self.running.as_ref().and_then(Running::session) else {
            return;
        };
        if !session.with_term(copy_mode::is_active) {
            session.with_copy(copy_mode::enter);
            self.copy_input = CopyInput::default();
        }
        let exit_after_copy = self.config.config.copy_mode.exit_after_copy;
        match copy_step(command, exit_after_copy) {
            CopyStep::WaitJump(kind) => {
                self.copy_input.jump = Some(kind);
                let name = match kind {
                    Jump::Forward => "f",
                    Jump::Backward => "F",
                    Jump::TillForward => "t",
                    Jump::TillBackward => "T",
                };
                self.copy_pending(name.to_owned());
            }
            CopyStep::Run(action) => {
                if matches!(action, CopyAction::SearchStart { .. }) {
                    self.copy_input.prompt = Some(InputBox::default());
                }
                self.copy_run(action);
            }
        }
    }

    /// Runs a step of the engine on the active pane; a copied text goes to the clipboard.
    pub(super) fn copy_run(&mut self, action: CopyAction) {
        let Some(running) = &self.running else {
            return;
        };
        let Some(session) = running.session() else {
            return;
        };
        let result = session
            .with_copy(|term, selection, state| copy_mode::apply(term, selection, state, action));
        let still = session.with_term(copy_mode::is_active);
        running.window.request_redraw();
        if !still {
            self.copy_input = CopyInput::default();
        }
        if let CopyResult::Copied(text) = result {
            self.copy_text(text);
        }
    }

    /// Starts or stops copy mode (`copy_mode`), or opens it with the search prompt (`search`).
    pub(super) fn toggle_copy_mode(&mut self, search: bool) {
        let Some(running) = &self.running else {
            return;
        };
        let Some(session) = running.session() else {
            return;
        };
        let active = session.with_term(copy_mode::is_active);
        if active && !search {
            self.copy_run(CopyAction::Exit);
            return;
        }
        if !active {
            session.with_copy(copy_mode::enter);
            self.copy_input = CopyInput::default();
        }
        running.window.request_redraw();
        if search && self.copy_input.prompt.is_none() {
            self.copy_command(CopyCommand::SearchForward);
        }
    }

    fn copy_pending(&self, text: String) {
        if let Some(session) = self.running.as_ref().and_then(Running::session) {
            session.with_copy(|_, _, state| state.pending = text);
        }
    }

    /// A Lua action of a copy mode key: it gets `(fterm, cm)`.
    fn copy_lua(&mut self, event_loop: &ActiveEventLoop, index: usize) {
        let Some(session) = self.running.as_ref().and_then(Running::session) else {
            return;
        };
        let input = session.with_copy(|term, _, state| {
            let cursor = term.vi_mode_cursor.point;
            CopyModeIn {
                selection: term.selection_to_string().filter(|s| !s.is_empty()),
                line: copy_mode::line_text(term, cursor),
                word: copy_mode::word_at(term, cursor),
                row: copy_mode::cursor_row(term).0,
                column: cursor.column.0 + 1,
                mode: state.mode().to_owned(),
                search: state.search_pattern().map(str::to_owned),
            }
        });
        match self.config.call_copy(index, &input) {
            Ok(calls) => {
                for call in calls {
                    self.run_api_call(event_loop, call);
                }
            }
            Err(err) => {
                tracing::warn!("Lua error: {err}");
                self.notify(
                    None,
                    &tr!("toast.lua_error"),
                    &err,
                    Level::Error,
                    Source::App,
                );
            }
        }
    }

    /// A key in the search prompt: each change searches again; Enter keeps, Esc goes back.
    fn copy_prompt_key(&mut self, event: &KeyEvent) {
        use super::overlay_calls::{edit_line, is_paste, one_line};
        let ctrl = self.mods.control_key();
        let pasted = is_paste(
            &event.logical_key,
            event.physical_key,
            ctrl,
            self.mods.shift_key(),
        )
        .then(|| self.clipboard.paste())
        .flatten();
        let Some(input) = &mut self.copy_input.prompt else {
            return;
        };
        let before = input.text.clone();
        match &event.logical_key {
            _ if pasted.is_some() => input.insert(&one_line(pasted.as_deref().unwrap_or(""))),
            Key::Named(NamedKey::Escape) => {
                self.copy_input.prompt = None;
                self.copy_run(CopyAction::SearchCancel);
                return;
            }
            Key::Named(NamedKey::Enter) => {
                self.copy_input.prompt = None;
                self.copy_run(CopyAction::SearchAccept);
                return;
            }
            key if edit_line(input, key) => {}
            _ => {
                if let Some(text) = event.text.as_deref()
                    && !ctrl
                    && !self.mods.alt_key()
                {
                    let text: String = text.chars().filter(|c| !c.is_control()).collect();
                    input.insert(&text);
                }
            }
        }
        let now = input.text.clone();
        if now != before {
            self.copy_run(CopyAction::SearchSet(now));
        } else if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
}
