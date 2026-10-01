//! The AI panel in the app (Phase 8): send a question, get the streamed answer, the keys of the panel,
//! and the prompt that saves the API key.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use fterm_ai::{AiError, Chat, Event, Kind, Provider, keys, stream};

use super::*;
use crate::ai_chat::{InputBox, system_prompt};

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
        let text = self.ai.input.take();
        let Some(id) = self.ai.ask(&text) else {
            // Empty, or an answer runs: keep the text.
            self.ai.input.set(&text);
            return;
        };
        let provider = match self.ai_provider() {
            Ok(provider) => provider,
            Err(err) => {
                self.ai.event(id, Event::Failed(AiError::Network(err)));
                return;
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
    }

    /// An event of a running answer.
    pub(super) fn ai_event(&mut self, id: u64, event: Event) {
        let ended = matches!(event, Event::Done { .. } | Event::Failed(_));
        let failed = matches!(&event, Event::Failed(e) if *e != AiError::Stopped);
        if !self.ai.event(id, event) {
            return;
        }
        if ended {
            self.ai_stop = None;
            let showing = self
                .running
                .as_ref()
                .is_some_and(|r| r.dock.showing(PanelKind::Ai));
            if !showing || !self.focused {
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

    fn redraw_ai(&self) {
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
        match &event.logical_key {
            Key::Named(NamedKey::Enter) if shift => self.ai.input.insert("\n"),
            Key::Named(NamedKey::Enter) => return self.ai_send(),
            Key::Named(NamedKey::Escape) => {
                // Esc stops a running answer; a second Esc gives the keyboard back to the terminal.
                match &self.ai_stop {
                    Some(stop) => stop.store(true, Ordering::SeqCst),
                    None => {
                        if let Some(running) = &mut self.running {
                            running.dock.focused = false;
                        }
                    }
                }
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
            // Ctrl+L: a new chat (like clearing a terminal).
            _ if self.mods.control_key() && physical_letter(event.physical_key) == Some('l') => {
                if let Some(stop) = self.ai_stop.take() {
                    stop.store(true, Ordering::SeqCst);
                }
                self.ai.running = None;
                self.ai.clear();
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
