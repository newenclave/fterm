//! Overlays (a program over a pane for a moment) and the scratch shell (a drop-down shell of the window).

use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey, PhysicalKey};

use super::*;

/// The arguments that make `shell` run `command` and end (the shell parses it, so PATH, quotes, and pipes work).
pub(super) fn shell_run_args(shell: &str, command: &str) -> Vec<String> {
    let file = shell.rsplit(['/', '\\']).next().unwrap_or(shell);
    let name = file.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    match name {
        // PowerShell ends with 1 after any failed program: give the real exit code (`overlay --wait` shows it). New
        // lines, not `;`, so a comment at the end of the command does not hide the rest.
        "powershell" | "pwsh" => vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-Command".into(),
            format!(
                "{command}
$ok = $?
if ($LASTEXITCODE) {{ exit $LASTEXITCODE }}
if (-not $ok) {{ exit 1 }}"
            ),
        ],
        "cmd" => vec!["/c".into(), command.into()],
        // bash, zsh, sh, fish, nu, and others take `-c`.
        _ => vec!["-c".into(), command.into()],
    }
}

/// What the command box of `run_in_overlay` (and `new_companion`) is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PromptFor {
    /// The overlay of the tab.
    Overlay,
    /// The overlay of the window.
    Window,
    /// A new companion of the active pane (no command = the shell).
    Companion,
}

/// The keys that edit the line of a small command box (arrows, Home, End, Backspace, Delete). `false` = not one of
/// them.
pub(super) fn edit_line(input: &mut crate::ai_chat::InputBox, key: &Key) -> bool {
    match key {
        Key::Named(NamedKey::ArrowLeft) => input.left(),
        Key::Named(NamedKey::ArrowRight) => input.right(),
        Key::Named(NamedKey::Home) => input.home(),
        Key::Named(NamedKey::End) => input.end(),
        Key::Named(NamedKey::Backspace) => input.backspace(),
        Key::Named(NamedKey::Delete) => input.delete(),
        _ => return false,
    }
    true
}

/// The key pastes into a small command box: Ctrl+V, Ctrl+Shift+V (by the place of V, so in any layout), or
/// Shift+Insert.
pub(super) fn is_paste(key: &Key, physical: PhysicalKey, ctrl: bool, shift: bool) -> bool {
    (ctrl && crate::input::physical_letter(physical) == Some('v'))
        || (shift && !ctrl && *key == Key::Named(NamedKey::Insert))
}

/// Pasted text for a box with one line: new lines and tabs are spaces, other control chars go.
pub(super) fn one_line(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let line: String = text
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    line.trim().to_owned()
}

/// The smallest overlay or scratch: 20 columns and 6 rows.
pub(super) fn floating_min(cell: fterm_render::font::CellMetrics, padding: f32) -> (f32, f32) {
    (
        20.0 * cell.width + 2.0 * padding,
        6.0 * cell.height + 2.0 * padding,
    )
}

impl App {
    /// Opens an overlay over the active pane: `command` (run by the shell of `profile` or the default one), or the
    /// program of `profile`. `size` in percent (`None` = the profile's or the config's).
    pub(super) fn open_overlay(
        &mut self,
        command: Option<String>,
        profile: Option<&str>,
        size: Option<u8>,
    ) -> anyhow::Result<PaneId> {
        let running = self.running.as_ref().expect("the window is open");
        if running.mux.overlay().is_some() {
            anyhow::bail!("{}", tr!("toast.overlay_there"));
        }
        let size = size
            .or_else(|| self.profile(profile).and_then(|p| p.overlay_size))
            .unwrap_or(self.config.config.overlay_size);
        let area = running.pane_area();
        let min = floating_min(running.renderer.cell(), running.renderer.padding());
        let rect = fterm_mux::mux::overlay_rect(area, size, min);
        let grid = running.grid_for(rect);
        self.spawn_command = command;
        let id = self.spawn_pane(grid, profile);
        self.spawn_command = None;
        let id = id?;
        let running = self.running.as_mut().expect("the window is open");
        running.mux.open_overlay(id, size);
        running.resize_all_panes();
        running.window.request_redraw();
        tracing::info!(pane = id.0, size, "an overlay opened");
        Ok(id)
    }

    /// Shows or hides the scratch shell; the first time it is made.
    pub(super) fn toggle_scratch(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        if running.mux.toggle_scratch().is_some() {
            running.resize_all_panes();
            running.window.request_redraw();
            return self.update_ime();
        }
        let size = self.config.config.scratch_size;
        let min = floating_min(running.renderer.cell(), running.renderer.padding());
        let rect = fterm_mux::mux::scratch_rect(running.tab_area(), size, min.1);
        let grid = running.grid_for(rect);
        match self.spawn_pane(grid, None) {
            Ok(id) => {
                let running = self.running.as_mut().expect("the window is open");
                running.mux.set_scratch(id, size);
                running.resize_all_panes();
                running.window.request_redraw();
                tracing::info!(pane = id.0, "the scratch shell opened");
            }
            Err(err) => self.notify(
                None,
                &tr!("toast.cannot_start"),
                &format!("{err:#}"),
                Level::Error,
                Source::App,
            ),
        }
    }

    /// The IME window follows the pane with the keys.
    pub(super) fn update_ime(&self) {
        if let Some(running) = &self.running {
            Self::update_ime_area(running);
        }
    }

    /// The overlay of the window (over every tab): shown or hidden; the first time its command runs (`window_overlay`
    /// of the config, or the one the user types).
    pub(super) fn toggle_window_overlay(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        if running.mux.toggle_window_overlay().is_some() {
            running.resize_all_panes();
            running.window.request_redraw();
            return self.update_ime();
        }
        match self.config.config.window_overlay.clone() {
            Some(command) => self.run_window_overlay(command),
            None => self.start_overlay_prompt(PromptFor::Window),
        }
    }

    /// Starts `command` in the overlay of the window.
    fn run_window_overlay(&mut self, command: String) {
        let Some(running) = &self.running else {
            return;
        };
        let size = self.config.config.overlay_size;
        let min = floating_min(running.renderer.cell(), running.renderer.padding());
        let rect = fterm_mux::mux::overlay_rect(running.tab_area(), size, min);
        let grid = running.grid_for(rect);
        self.spawn_command = Some(command);
        let id = self.spawn_pane(grid, None);
        self.spawn_command = None;
        match id {
            Ok(id) => {
                let running = self.running.as_mut().expect("the window is open");
                running.mux.set_window_overlay(id, size);
                running.resize_all_panes();
                running.window.request_redraw();
                tracing::info!(pane = id.0, size, "the window overlay opened");
            }
            Err(err) => self.notify(
                None,
                &tr!("toast.cannot_start"),
                &format!("{err:#}"),
                Level::Error,
                Source::App,
            ),
        }
    }

    /// The box of the actions `run_in_overlay` and `new_companion` for the command.
    pub(super) fn start_overlay_prompt(&mut self, prompt_for: PromptFor) {
        self.overlay_prompt = Some(crate::ai_chat::InputBox::default());
        self.overlay_prompt_for = prompt_for;
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    pub(super) fn overlay_prompt_lines(&self) -> Option<Vec<String>> {
        let input = self.overlay_prompt.as_ref()?;
        Some(vec![
            match self.overlay_prompt_for {
                PromptFor::Overlay => tr!("box.overlay"),
                PromptFor::Window => tr!("box.window_overlay"),
                PromptFor::Companion => tr!("box.companion"),
            },
            input.line_with_cursor('▏'),
            String::new(),
            tr!("box.overlay_keys"),
        ])
    }

    pub(super) fn overlay_prompt_key(&mut self, event: &KeyEvent) {
        let ctrl = self.mods.control_key();
        let pasted = is_paste(
            &event.logical_key,
            event.physical_key,
            ctrl,
            self.mods.shift_key(),
        )
        .then(|| self.clipboard.paste())
        .flatten();
        let Some(input) = &mut self.overlay_prompt else {
            return;
        };
        match &event.logical_key {
            _ if pasted.is_some() => input.insert(&one_line(pasted.as_deref().unwrap_or(""))),
            Key::Named(NamedKey::Escape) => self.overlay_prompt = None,
            Key::Named(NamedKey::Enter) => {
                let command = input.take().trim().to_owned();
                self.overlay_prompt = None;
                let opened = match self.overlay_prompt_for {
                    _ if command.is_empty() && self.overlay_prompt_for != PromptFor::Companion => {
                        Ok(())
                    }
                    PromptFor::Window => {
                        self.run_window_overlay(command);
                        Ok(())
                    }
                    PromptFor::Overlay => self.open_overlay(Some(command), None, None).map(|_| ()),
                    // No command: the shell.
                    PromptFor::Companion => self
                        .open_companion((!command.is_empty()).then_some(command), None)
                        .map(|_| ()),
                };
                if let Err(err) = opened {
                    self.notify(
                        None,
                        &tr!("toast.cannot_start"),
                        &format!("{err:#}"),
                        Level::Error,
                        Source::App,
                    );
                }
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
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_box_moves_its_cursor() {
        let key =
            |input: &mut crate::ai_chat::InputBox, k: NamedKey| edit_line(input, &Key::Named(k));
        let mut input = crate::ai_chat::InputBox::default();
        input.insert("pnig");
        assert!(key(&mut input, NamedKey::ArrowLeft));
        assert!(key(&mut input, NamedKey::ArrowLeft));
        assert!(key(&mut input, NamedKey::Backspace));
        input.insert("in");
        assert!(key(&mut input, NamedKey::Delete));
        assert_eq!(input.text, "ping");
        assert!(key(&mut input, NamedKey::Home));
        input.insert(">");
        assert!(key(&mut input, NamedKey::End));
        input.insert("!");
        assert!(key(&mut input, NamedKey::ArrowLeft));
        assert!(key(&mut input, NamedKey::ArrowRight));
        input.insert("?");
        assert_eq!(input.text, ">ping!?");
        // Other keys are not for the line.
        assert!(!key(&mut input, NamedKey::Enter));
        assert!(!key(&mut input, NamedKey::Escape));
        assert!(!edit_line(&mut input, &Key::Character("a".into())));
    }

    #[test]
    fn a_paste_into_the_command_box() {
        use winit::keyboard::{KeyCode, PhysicalKey};
        let v = PhysicalKey::Code(KeyCode::KeyV);
        let insert = Key::Named(NamedKey::Insert);
        let letter = Key::Character("v".into());
        assert!(is_paste(&letter, v, true, false), "Ctrl+V");
        assert!(is_paste(&letter, v, true, true), "Ctrl+Shift+V");
        assert!(
            is_paste(&insert, PhysicalKey::Code(KeyCode::Insert), false, true),
            "Shift+Insert"
        );
        // Another layout: the key of V still pastes.
        assert!(is_paste(&Key::Character("м".into()), v, true, false));
        assert!(!is_paste(&letter, v, false, false), "a plain v is typed");
        assert!(!is_paste(
            &insert,
            PhysicalKey::Code(KeyCode::Insert),
            false,
            false
        ));
        // A box has one line: new lines are spaces, other control chars go.
        assert_eq!(one_line("git log\r\n--oneline\n"), "git log --oneline");
        assert_eq!(one_line("a\tb\u{1b}c"), "a bc");
    }

    #[test]
    fn each_shell_runs_a_command_its_way() {
        assert_eq!(
            shell_run_args("powershell.exe", "yazi C:/x")[..3],
            ["-NoLogo", "-NoProfile", "-Command"]
        );
        // PowerShell gives 1 for any failed program: the real exit code goes out after the command (on new lines,
        // so a comment at the end of the command does not hide it).
        let script = &shell_run_args("pwsh", "lazygit # the UI")[3];
        assert!(
            script.starts_with(
                "lazygit # the UI
"
            ),
            "{script:?}"
        );
        assert!(script.contains("exit $LASTEXITCODE"), "{script:?}");
        assert_eq!(
            shell_run_args(r"C:\Program Files\PowerShell\7\pwsh.exe", "htop")[2],
            "-Command"
        );
        assert_eq!(shell_run_args("cmd.exe", "dir /w"), ["/c", "dir /w"]);
        assert_eq!(shell_run_args("CMD", "x"), ["/c", "x"]);
        assert_eq!(shell_run_args("/bin/zsh", "lazygit"), ["-c", "lazygit"]);
        assert_eq!(shell_run_args("bash", "a | b"), ["-c", "a | b"]);
        assert_eq!(shell_run_args("fish", "x"), ["-c", "x"]);
    }
}
