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

/// What the frame draws for copy mode in the active pane: the matches and the status bar.
#[derive(Default)]
pub(super) struct CopyMarks {
    pub pane: Option<PaneId>,
    pub matches: Vec<(Point, Point)>,
    pub current: Option<usize>,
    /// The text of the status bar and its column on the last row.
    pub status: Option<(String, usize, usize)>,
}

/// The parts of the status bar.
pub(super) struct StatusParts<'a> {
    pub mode: &'a str,
    pub row: i64,
    pub total: usize,
    /// The search: its pattern, the number of the current match, and how many there are.
    pub search: Option<(&'a str, Option<usize>, usize)>,
    /// The prompt while the user types: `/` or `?` and the text with its cursor.
    pub prompt: Option<(char, String)>,
    pub pending: &'a str,
}

/// The text of the copy mode status bar: ` COPY · VISUAL · 120/3400 · /error 3/9 `.
pub(super) fn copy_status(p: &StatusParts) -> String {
    if let Some((mark, text)) = &p.prompt {
        let count = match p.search {
            Some((_, current, total)) if !text.is_empty() => {
                format!("  {}", match_text(current, total))
            }
            _ => String::new(),
        };
        return format!(" {mark}{text}{count} ");
    }
    let mode = match p.mode {
        "visual" => " · VISUAL",
        "visual_line" => " · VISUAL LINE",
        "visual_block" => " · VISUAL BLOCK",
        _ => "",
    };
    let mut out = format!(" COPY{mode} · {}/{}", p.row, p.total);
    if let Some((pattern, current, total)) = p.search {
        out.push_str(&format!(" · /{pattern} {}", match_text(current, total)));
    }
    if !p.pending.is_empty() {
        out.push_str(&format!(" · {}…", p.pending));
    }
    out.push(' ');
    out
}

fn match_text(current: Option<usize>, total: usize) -> String {
    let total = if total >= copy_mode::MAX_COUNT {
        format!("{}+", copy_mode::MAX_COUNT)
    } else {
        total.to_string()
    };
    match current {
        Some(n) => format!("{n}/{total}"),
        None => format!("-/{total}"),
    }
}

impl App {
    /// The matches and the status bar of copy mode in the active pane (empty when copy mode is off).
    pub(super) fn copy_marks(&self) -> CopyMarks {
        let Some(running) = &self.running else {
            return CopyMarks::default();
        };
        let Some(id) = running.mux.input_pane() else {
            return CopyMarks::default();
        };
        let Some(pane) = running.panes.get(&id) else {
            return CopyMarks::default();
        };
        if !pane.session.with_term(copy_mode::is_active) {
            return CopyMarks::default();
        }
        let bar = self.config.config.copy_mode.status_bar;
        let prompt = self
            .copy_input
            .prompt
            .as_ref()
            .map(|input| input.line_with_cursor('▏'));
        pane.session.with_copy(|term, _, state| {
            let (list, current) = state.visible_matches(term);
            let matches = list.iter().map(|m| (*m.start(), *m.end())).collect();
            let status = bar.then(|| {
                let count = state.match_count(term);
                let (row, total) = copy_mode::cursor_row(term);
                let text = copy_status(&StatusParts {
                    mode: state.mode(),
                    row,
                    total,
                    search: state
                        .search_pattern()
                        .zip(count)
                        .map(|(p, (c, t))| (p, c, t)),
                    prompt: prompt
                        .clone()
                        .map(|t| (if state.prompt_forward() { '/' } else { '?' }, t)),
                    pending: &state.pending,
                });
                let columns = term.columns();
                let text: String = text.chars().take(columns).collect();
                let width = text.chars().count();
                (text, columns - width, term.screen_lines() - 1)
            });
            CopyMarks {
                pane: Some(id),
                matches,
                current,
                status,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> StatusParts<'static> {
        StatusParts {
            mode: "normal",
            row: 120,
            total: 3400,
            search: None,
            prompt: None,
            pending: "",
        }
    }

    #[test]
    fn the_status_bar_text() {
        assert_eq!(copy_status(&parts()), " COPY · 120/3400 ");
        let visual = StatusParts {
            mode: "visual_line",
            search: Some(("error", Some(3), 9)),
            ..parts()
        };
        assert_eq!(
            copy_status(&visual),
            " COPY · VISUAL LINE · 120/3400 · /error 3/9 "
        );
        let pending = StatusParts {
            pending: "i",
            search: Some(("x", None, 1000)),
            ..parts()
        };
        assert_eq!(copy_status(&pending), " COPY · 120/3400 · /x -/1000+ · i… ");
    }

    #[test]
    fn the_status_bar_while_typing_a_search() {
        let typing = StatusParts {
            prompt: Some(('?', "err▏".into())),
            search: Some(("err", Some(2), 5)),
            ..parts()
        };
        assert_eq!(copy_status(&typing), " ?err▏  2/5 ");
        let empty = StatusParts {
            prompt: Some(('/', "▏".into())),
            ..parts()
        };
        assert_eq!(copy_status(&empty), " /▏ ");
    }
}
