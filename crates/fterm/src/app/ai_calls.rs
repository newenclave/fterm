//! The AI panel in the app (Phase 8): send a question, get the streamed answer, the keys of the panel,
//! and the prompt that saves the API key.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use fterm_ai::{AiError, Chat, Event, Kind, Provider, keys, stream};

use super::*;
use crate::ai_chat::{ContextItem, InputBox, build_message, last_code_block, system_prompt};
use crate::text_command::clean_command;

/// "Text to command" that waits for its answer.
pub(super) struct PendingCommand {
    id: u64,
    pane: PaneId,
    /// The task as it was typed (the prompt must still show it when the command comes).
    typed: String,
    answer: String,
    stop: Arc<AtomicBool>,
}

/// A question that did not go to the AI: its context (to put back), and why, for a notification
/// (`None`: the question is empty, or an answer runs).
pub(super) struct Refused {
    pub context: Vec<ContextItem>,
    pub why: Option<(&'static str, String, Level)>,
}

impl App {
    /// The provider from the config, as `fterm_ai` wants it.
    fn ai_provider(&self) -> Result<Provider, String> {
        let ai = &self.config.config.ai;
        let p = ai
            .current()
            .ok_or_else(|| format!("no AI provider `{}`", ai.provider))?;
        if p.model.trim().is_empty() {
            return Err(format!(
                "set a model: ai = {{ providers = {{ {} = {{ model = \"...\" }} }} }} in the config",
                p.name
            ));
        }
        Ok(Provider {
            name: p.name.clone(),
            kind: match p.kind {
                fterm_config::load::AiKind::Anthropic => Kind::Anthropic,
                fterm_config::load::AiKind::OpenAi => Kind::OpenAi,
            },
            url: p.url.clone(),
            model: p.model.clone(),
            key_env: p.key_env.clone(),
            needs_key: p.needs_key,
        })
    }

    /// "anthropic · claude-haiku-4-5-20251001" for the panel title.
    pub(super) fn ai_title(&self) -> String {
        match self.config.config.ai.current() {
            Some(p) if p.model.is_empty() => format!("{} · no model", p.name),
            Some(p) => format!("{} · {}", p.name, p.model),
            None => "no provider".to_owned(),
        }
    }

    /// Sends the text of the input box.
    pub(super) fn ai_send(&mut self) {
        let question = self.ai.input.take();
        if question.trim().is_empty() || self.ai.running.is_some() {
            self.ai.input.set(&question);
            return;
        }
        let context = std::mem::take(&mut self.ai.context);
        if let Err(Refused { context, why }) = self.ai_submit(&question, context, None) {
            self.ai.input.set(&question);
            self.ai.context = context;
            if let Some((title, body, level)) = why {
                self.notify(None, title, &body, level, Source::App);
            }
        }
    }

    /// Asks the AI: the question goes into the chat, and the answer streams in. `from` is the name of
    /// the API client that asks (the chat shows it). Gives the request id, or the context back when the
    /// question did not go (with the reason for a notification).
    pub(super) fn ai_submit(
        &mut self,
        question: &str,
        context: Vec<ContextItem>,
        from: Option<&str>,
    ) -> Result<u64, Refused> {
        if question.trim().is_empty() || self.ai.running.is_some() {
            return Err(Refused { context, why: None });
        }
        let mut sent = build_message(question.trim(), &context);
        // on_ai_request in the config can stop it or change the text (for example, to hide secrets).
        let (provider_name, model) = self
            .config
            .config
            .ai
            .current()
            .map(|p| (p.name.clone(), p.model.clone()))
            .unwrap_or_default();
        let request = fterm_config::load::AiRequestIn {
            question: question.trim().to_owned(),
            text: sent.clone(),
            provider: provider_name,
            model,
        };
        match self.config.on_ai_request(&request) {
            Ok(Some(text)) => sent = text,
            Ok(None) => {
                return Err(Refused {
                    context,
                    why: Some((
                        "The question was not sent",
                        "on_ai_request in your config stopped it.".to_owned(),
                        Level::Info,
                    )),
                });
            }
            Err(err) => {
                return Err(Refused {
                    context,
                    why: Some(("Lua error", err, Level::Error)),
                });
            }
        }
        let mut display = question.trim().to_owned();
        if let Some(from) = from {
            display.push_str(&format!("\n(from {from})"));
        }
        for item in &context {
            display.push_str(&format!("\n+ {}", item.label()));
        }
        let Some(id) = self.ai.ask_with(&display, sent) else {
            return Err(Refused { context, why: None });
        };
        let provider = match self.ai_provider() {
            Ok(provider) => provider,
            Err(err) => {
                // The chat shows the error, and the request has ended.
                self.ai_event(id, Event::Failed(AiError::Network(err)));
                return Ok(id);
            }
        };
        let pane = self.running.as_ref().and_then(Running::active_pane);
        let shell = pane.map_or("a shell".to_owned(), |p| {
            display_name(p.session.program()).to_owned()
        });
        let cwd = pane.and_then(|p| p.shell.cwd.clone());
        let os = match std::env::consts::OS {
            "windows" => "Windows",
            "macos" => "macOS",
            "linux" => "Linux",
            other => other,
        };
        let chat = Chat {
            system: system_prompt(os, &shell, cwd.as_deref(), &self.config.config.ai.system),
            messages: self.ai.messages(),
            max_tokens: self.config.config.ai.max_tokens,
        };
        let stop = Arc::new(AtomicBool::new(false));
        self.ai_stop = Some(stop.clone());
        let proxy = self.proxy.clone();
        tracing::info!(provider = %provider.name, model = %provider.model, "AI question");
        let _ = std::thread::Builder::new()
            .name("fterm-ai".into())
            .spawn(move || {
                let key = if provider.needs_key {
                    keys::get(&provider)
                } else {
                    None
                };
                stream::run(&provider, key.as_deref(), &chat, &stop, &mut |event| {
                    let _ = proxy.send_event(UserEvent::Ai(id, event));
                });
            });
        self.redraw_ai();
        Ok(id)
    }

    /// Stops the running answer. Returns `false` when no answer runs.
    pub(super) fn ai_stop_answer(&self) -> bool {
        match &self.ai_stop {
            Some(stop) => {
                stop.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// A new chat: stops the running answer and forgets all turns.
    pub(super) fn ai_clear_chat(&mut self) {
        if let Some(id) = self.ai.running {
            // As if the answer ended: API clients that wait for it get their answer now.
            self.ai_stop_answer();
            self.ai_event(id, Event::Failed(AiError::Stopped));
        }
        self.ai_stop = None;
        self.ai.clear();
        self.redraw_ai();
    }

    /// An event of a running answer.
    pub(super) fn ai_event(&mut self, id: u64, event: Event) {
        let ended = matches!(event, Event::Done { .. } | Event::Failed(_));
        let stopped = event == Event::Failed(AiError::Stopped);
        let failed = matches!(&event, Event::Failed(_)) && !stopped;
        if !self.ai.event(id, event) {
            return;
        }
        if ended {
            self.ai_stop = None;
            self.ai_answered(id);
            let showing = self
                .running
                .as_ref()
                .is_some_and(|r| r.dock.showing(PanelKind::Ai));
            // A stopped answer needs no notification: somebody stopped it on purpose.
            if (!showing || !self.focused) && !stopped {
                let (title, level) = if failed {
                    ("The AI answer failed", Level::Error)
                } else {
                    ("The AI answer is ready", Level::Success)
                };
                let question = self.ai.last_question.clone().unwrap_or_default();
                let short: String = question.chars().take(60).collect();
                self.notify(None, title, &short, level, Source::App);
            }
        }
        self.redraw_ai();
    }

    pub(super) fn redraw_ai(&self) {
        if let Some(running) = &self.running
            && running.dock.showing(PanelKind::Ai)
        {
            running.window.request_redraw();
        }
    }

    /// A key while the AI panel has the keyboard.
    pub(super) fn ai_key(&mut self, event: &KeyEvent) {
        let shift = self.mods.shift_key();
        let rows = self.ai_rows();
        let ctrl = self.mods.control_key();
        let alt = self.mods.alt_key();
        let letter = physical_letter(event.physical_key);
        match &event.logical_key {
            // Ctrl+Shift+Enter: put the last command of the answer into the prompt (it does not run).
            Key::Named(NamedKey::Enter) if ctrl && shift => return self.ai_put_code(),
            Key::Named(NamedKey::Enter) if shift => self.ai.input.insert("\n"),
            Key::Named(NamedKey::Enter) => return self.ai_send(),
            Key::Named(NamedKey::Escape) => {
                // Esc stops a running answer; a second Esc gives the keyboard back to the terminal.
                if !self.ai_stop_answer()
                    && let Some(running) = &mut self.running
                {
                    running.dock.focused = false;
                }
            }
            // Backspace in an empty input takes away the last chip.
            Key::Named(NamedKey::Backspace) if self.ai.input.text.is_empty() => {
                self.ai.context.pop();
            }
            Key::Named(NamedKey::Backspace) => self.ai.input.backspace(),
            Key::Named(NamedKey::Delete) => self.ai.input.delete(),
            Key::Named(NamedKey::ArrowLeft) => self.ai.input.left(),
            Key::Named(NamedKey::ArrowRight) => self.ai.input.right(),
            Key::Named(NamedKey::Home) => self.ai.input.home(),
            Key::Named(NamedKey::End) => self.ai.input.end(),
            Key::Named(NamedKey::ArrowUp) if self.ai.input.text.is_empty() => {
                if let Some(last) = self.ai.last_question.clone() {
                    self.ai.input.set(&last);
                }
            }
            Key::Named(NamedKey::PageUp) => self.ai.scroll += rows.max(1),
            Key::Named(NamedKey::PageDown) => {
                self.ai.scroll = self.ai.scroll.saturating_sub(rows.max(1))
            }
            // Alt+O: the last command and its output; Alt+S: the selection.
            _ if alt && letter == Some('o') => self.ai_add_output(),
            _ if alt && letter == Some('s') => self.ai_add_selection(),
            // Ctrl+L: a new chat (like clearing a terminal).
            _ if self.mods.control_key() && physical_letter(event.physical_key) == Some('l') => {
                self.ai_clear_chat()
            }
            Key::Named(NamedKey::Tab) => {
                if let Some(running) = &mut self.running {
                    running.dock.next_panel();
                }
            }
            _ => {
                if let Some(text) = event.text.as_deref()
                    && !self.mods.control_key()
                    && !self.mods.alt_key()
                {
                    let text: String = text.chars().filter(|c| !c.is_control()).collect();
                    self.ai.input.insert(&text);
                }
            }
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// The last command of the active pane and its output, as context.
    fn pane_context(&self) -> Vec<ContextItem> {
        match self.running.as_ref().and_then(|r| r.mux.active_pane()) {
            Some(pane) => self.pane_context_of(pane),
            None => Vec::new(),
        }
    }

    /// The last command of a pane and its output, as context.
    pub(super) fn pane_context_of(&self, pane: PaneId) -> Vec<ContextItem> {
        let Some(pane) = self.running.as_ref().and_then(|r| r.panes.get(&pane)) else {
            return Vec::new();
        };
        let mut items = Vec::new();
        if let Some(last) = &pane.last_command {
            items.push(ContextItem::Command {
                command: last
                    .command
                    .clone()
                    .unwrap_or_else(|| "(unknown)".to_owned()),
                exit: last.exit,
            });
        }
        let output = pane.session.with_term(|term| {
            let total = fterm_term::input::total_lines(term);
            pane.shell
                .last_output(total)
                .map(|(start, end)| fterm_term::input::lines_text(term, start, end))
        });
        if let Some(output) = output.filter(|o| !o.trim().is_empty()) {
            items.push(ContextItem::Output(output.trim_end().to_owned()));
        }
        items
    }

    /// Adds context items (one of each kind: a new one takes the place of an old one).
    fn add_context(&mut self, items: Vec<ContextItem>) {
        for item in items {
            let same = |a: &ContextItem| std::mem::discriminant(a) == std::mem::discriminant(&item);
            self.ai.context.retain(|c| !same(c));
            self.ai.context.push(item);
        }
        self.redraw_ai();
    }

    fn ai_add_output(&mut self) {
        let items = self.pane_context();
        if items.is_empty() {
            return self.notify(
                None,
                "No command output",
                "It needs shell integration (see CONFIG.md).",
                Level::Info,
                Source::App,
            );
        }
        self.add_context(items);
    }

    fn ai_add_selection(&mut self) {
        let text = self
            .running
            .as_ref()
            .and_then(Running::session)
            .and_then(|s| s.with_term(|term| term.selection_to_string()))
            .filter(|t| !t.trim().is_empty());
        match text {
            Some(text) => self.add_context(vec![ContextItem::Selection(text)]),
            None => self.notify(None, "Nothing is selected", "", Level::Info, Source::App),
        }
    }

    /// `explain_error`: the last command and its output go to the AI with a question, at once.
    pub(super) fn explain_error(&mut self) {
        let items = self.pane_context();
        let exit = items.iter().find_map(|i| match i {
            ContextItem::Command { exit, .. } => Some(*exit),
            _ => None,
        });
        let Some(exit) = exit else {
            return self.notify(
                None,
                "No last command",
                "It needs shell integration (see CONFIG.md).",
                Level::Info,
                Source::App,
            );
        };
        self.open_ai_panel();
        self.add_context(items);
        let question = match exit {
            Some(0) => "Explain this output.",
            _ => "Why did this command fail, and how do I fix it?",
        };
        self.ai.input.set(question);
        self.ai_send();
    }

    /// `ask_ai_selection`: the selection goes into the context; the user writes the question.
    pub(super) fn ask_ai_selection(&mut self) {
        self.open_ai_panel();
        self.ai_add_selection();
    }

    fn open_ai_panel(&mut self) {
        if let Some(running) = &mut self.running {
            running.dock.open = true;
            running.dock.active = PanelKind::Ai;
            running.dock.focused = true;
        }
        self.dock_changed();
    }

    /// Puts the last code block of the answer into the prompt of the active pane (not run).
    fn ai_put_code(&mut self) {
        let Some(code) = last_code_block(&self.ai.turns) else {
            return self.notify(
                None,
                "No command in the answer",
                "",
                Level::Info,
                Source::App,
            );
        };
        let Some(pane) = self.running.as_ref().and_then(Running::active_pane) else {
            return;
        };
        if pane.shell.is_running() {
            self.copy_text(code);
            return self.notify(
                None,
                "A program runs in this pane",
                "The command is copied. Paste it where you need it.",
                Level::Info,
                Source::App,
            );
        }
        let typed = pane
            .shell
            .input_start()
            .and_then(|start| pane.session.with_term(|term| typed_input(term, start)))
            .map(|i| i.text)
            .unwrap_or_default();
        // A block of many lines goes as one line for the shell to see it all (PowerShell and bash keep it).
        let text = code.trim_end().to_owned();
        self.type_into_prompt(&typed, &text, false);
        if let Some(running) = &mut self.running {
            running.dock.focused = false;
            running.window.request_redraw();
        }
    }

    /// Copy in the AI panel: the last code block, else the last answer.
    pub(super) fn ai_copy(&mut self) {
        let text = last_code_block(&self.ai.turns).or_else(|| {
            self.ai
                .turns
                .iter()
                .rev()
                .find(|t| !t.user)
                .map(|t| t.text.clone())
        });
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            self.copy_text(text);
        }
    }

    /// `text_to_command`: the task typed in the prompt goes to the AI; the command comes in its place.
    pub(super) fn text_to_command(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let Some(pane_id) = running.mux.active_pane() else {
            return;
        };
        let pane = &running.panes[&pane_id];
        if pane.shell.is_running() {
            return self.notify(
                None,
                "A program runs in this pane",
                "Text to command works at the prompt of a shell.",
                Level::Info,
                Source::App,
            );
        }
        let typed = pane
            .shell
            .input_start()
            .and_then(|start| pane.session.with_term(|term| typed_input(term, start)))
            .map(|i| i.text)
            .unwrap_or_default();
        if typed.trim().is_empty() {
            return self.notify(
                None,
                "Type a task first",
                "Write what you want in the prompt (for example: find the 10 biggest files), then press Ctrl+Shift+G.",
                Level::Info,
                Source::App,
            );
        }
        if self.pending_command.is_some() {
            return;
        }
        let shell = display_name(pane.session.program()).to_owned();
        let cwd = pane.shell.cwd.clone();
        let mut provider = match self.ai_provider() {
            Ok(provider) => provider,
            Err(err) => {
                return self.notify(None, "Text to command", &err, Level::Error, Source::App);
            }
        };
        if let Some(model) = &self.config.config.ai.command_model {
            provider.model = model.clone();
        }
        let os = match std::env::consts::OS {
            "windows" => "Windows",
            "macos" => "macOS",
            "linux" => "Linux",
            other => other,
        };
        // Examples first (small models follow them better than rules), then the task.
        let mut messages = Vec::new();
        for (task, command) in crate::text_command::examples(&shell) {
            messages.push(fterm_ai::Message {
                role: fterm_ai::Role::User,
                content: task.to_owned(),
            });
            messages.push(fterm_ai::Message {
                role: fterm_ai::Role::Assistant,
                content: command.to_owned(),
            });
        }
        messages.push(fterm_ai::Message {
            role: fterm_ai::Role::User,
            content: typed.trim().to_owned(),
        });
        let chat = Chat {
            system: crate::text_command::system_prompt(os, &shell, cwd.as_deref()),
            messages,
            max_tokens: 400,
        };
        self.command_next_id += 1;
        let id = self.command_next_id;
        let stop = Arc::new(AtomicBool::new(false));
        self.pending_command = Some(PendingCommand {
            id,
            pane: pane_id,
            typed,
            answer: String::new(),
            stop: stop.clone(),
        });
        let proxy = self.proxy.clone();
        tracing::info!(provider = %provider.name, model = %provider.model, "text to command");
        let _ = std::thread::Builder::new()
            .name("fterm-ai-command".into())
            .spawn(move || {
                let key = if provider.needs_key {
                    keys::get(&provider)
                } else {
                    None
                };
                stream::run(&provider, key.as_deref(), &chat, &stop, &mut |event| {
                    let _ = proxy.send_event(UserEvent::AiCommand(id, event));
                });
            });
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Stops a waiting "text to command". `true` when there was one.
    pub(super) fn stop_command(&mut self) -> bool {
        match self.pending_command.take() {
            Some(pending) => {
                pending.stop.store(true, Ordering::SeqCst);
                if let Some(running) = &self.running {
                    running.window.request_redraw();
                }
                true
            }
            None => false,
        }
    }

    pub(super) fn command_event(&mut self, id: u64, event: Event) {
        let Some(pending) = self.pending_command.as_mut().filter(|p| p.id == id) else {
            return;
        };
        match event {
            Event::Delta(text) => {
                pending.answer.push_str(&text);
                return;
            }
            Event::Failed(AiError::Stopped) => {
                self.pending_command = None;
            }
            Event::Failed(err) => {
                self.pending_command = None;
                self.notify(
                    None,
                    "Text to command failed",
                    &err.to_string(),
                    Level::Error,
                    Source::App,
                );
            }
            Event::Done { .. } => {
                let Some(pending) = self.pending_command.take() else {
                    return;
                };
                let command = clean_command(&pending.answer);
                if command.is_empty() {
                    self.notify(
                        None,
                        "No command came back",
                        &pending.answer,
                        Level::Warning,
                        Source::App,
                    );
                } else {
                    self.put_command(pending.pane, &pending.typed, &command);
                }
            }
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Puts the command in place of the task, when the prompt still shows the task. Else it is copied.
    fn put_command(&mut self, pane_id: PaneId, task: &str, command: &str) {
        let now = self
            .running
            .as_ref()
            .and_then(|r| r.panes.get(&pane_id))
            .and_then(|pane| {
                if pane.shell.is_running() {
                    return None;
                }
                pane.shell
                    .input_start()
                    .and_then(|start| pane.session.with_term(|term| typed_input(term, start)))
                    .map(|i| i.text)
            });
        if now.as_deref() != Some(task) {
            self.copy_text(command.to_owned());
            return self.notify(
                None,
                "The command is ready (copied)",
                command,
                Level::Info,
                Source::App,
            );
        }
        if let Some(session) = self
            .running
            .as_ref()
            .and_then(|r| r.panes.get(&pane_id))
            .map(|p| &p.session)
        {
            session.write(crate::history_popup::replace_input(task, command, false));
        }
    }

    /// The grey text after the cursor while "text to command" waits.
    pub(super) fn pending_hint(&self) -> Option<(String, usize, usize)> {
        let pending = self.pending_command.as_ref()?;
        let running = self.running.as_ref()?;
        if running.mux.active_pane() != Some(pending.pane) {
            return None;
        }
        let pane = running.panes.get(&pending.pane)?;
        let (column, line) = pane.session.with_term(|term| {
            let point = term.grid().cursor.point;
            usize::try_from(point.line.0)
                .ok()
                .map(|l| (point.column.0, l))
        })?;
        Some(("  ⏳ asking AI… (Esc = stop)".to_owned(), column, line))
    }

    /// How many chat lines fit now.
    fn ai_rows(&self) -> usize {
        let Some(layout) = self.dock_layout() else {
            return 0;
        };
        let cell = self.running.as_ref().map(|r| r.renderer.cell());
        let Some(cell) = cell else {
            return 0;
        };
        let width = ((layout.list.width / cell.width) as usize).saturating_sub(2);
        let input = self.ai.input.layout(width).0.len();
        fterm_render::dock::chat_rows(&layout, cell, input)
    }

    /// Paste into the AI input (Ctrl+V when the AI panel has the keyboard).
    pub(super) fn ai_paste(&mut self) {
        if let Some(text) = self.clipboard.paste() {
            self.ai.input.insert(&text.replace("\r\n", "\n"));
            self.redraw_ai();
        }
    }

    /// `set_ai_key`: a small box that asks for the key (the text is hidden).
    pub(super) fn start_key_prompt(&mut self) {
        self.key_prompt = Some(InputBox::default());
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    pub(super) fn key_prompt_lines(&self) -> Option<Vec<String>> {
        let input = self.key_prompt.as_ref()?;
        let name = self.config.config.ai.provider.clone();
        let dots: String = "•".repeat(input.text.chars().count().min(40));
        Some(vec![
            format!("The API key for {name}:"),
            if dots.is_empty() {
                "(paste it with Ctrl+V)".to_owned()
            } else {
                dots
            },
            String::new(),
            "It goes to the Windows Credential Manager, not to a file.".to_owned(),
            "Enter = save, Esc = cancel (an empty key deletes the saved one)".to_owned(),
        ])
    }

    pub(super) fn key_prompt_key(&mut self, event: &KeyEvent) {
        let ctrl = self.mods.control_key();
        let Some(input) = &mut self.key_prompt else {
            return;
        };
        match (&event.logical_key, physical_letter(event.physical_key)) {
            (Key::Named(NamedKey::Escape), _) => self.key_prompt = None,
            (Key::Named(NamedKey::Enter), _) => {
                let key = input.take();
                self.key_prompt = None;
                let name = self.config.config.ai.provider.clone();
                match keys::set(&name, &key) {
                    Ok(()) if key.trim().is_empty() => self.notify(
                        None,
                        "The AI key is deleted",
                        &name,
                        Level::Info,
                        Source::App,
                    ),
                    Ok(()) => self.notify(
                        None,
                        "The AI key is saved",
                        &format!("{name}: in the Windows Credential Manager"),
                        Level::Success,
                        Source::App,
                    ),
                    Err(err) => self.notify(
                        None,
                        "Cannot save the AI key",
                        &err,
                        Level::Error,
                        Source::App,
                    ),
                }
            }
            (Key::Named(NamedKey::Backspace), _) => input.backspace(),
            (_, Some('v')) if ctrl => {
                if let Some(text) = self.clipboard.paste() {
                    input.insert(text.trim());
                }
            }
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
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
}
