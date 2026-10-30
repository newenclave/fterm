//! Overlays (a program over a pane for a moment) and the scratch shell (a drop-down shell of the window).

use winit::event::KeyEvent;
use winit::keyboard::{Key, NamedKey};

use super::*;

/// The arguments that make `shell` run `command` and end (the shell parses it, so PATH, quotes, and pipes work).
pub(super) fn shell_run_args(shell: &str, command: &str) -> Vec<String> {
    let file = shell.rsplit(['/', '\\']).next().unwrap_or(shell);
    let name = file.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    match name {
        "powershell" | "pwsh" => vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-Command".into(),
            command.into(),
        ],
        "cmd" => vec!["/c".into(), command.into()],
        // bash, zsh, sh, fish, nu, and others take `-c`.
        _ => vec!["-c".into(), command.into()],
    }
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
    fn update_ime(&self) {
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
            None => self.start_overlay_prompt(true),
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

    /// The box of the action `run_in_overlay` for the command (`window` = for the overlay of the window).
    pub(super) fn start_overlay_prompt(&mut self, window: bool) {
        self.overlay_prompt = Some(crate::ai_chat::InputBox::default());
        self.overlay_prompt_window = window;
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    pub(super) fn overlay_prompt_lines(&self) -> Option<Vec<String>> {
        let input = self.overlay_prompt.as_ref()?;
        Some(vec![
            if self.overlay_prompt_window {
                tr!("box.window_overlay")
            } else {
                tr!("box.overlay")
            },
            format!("{}▏", input.text),
            String::new(),
            tr!("box.overlay_keys"),
        ])
    }

    pub(super) fn overlay_prompt_key(&mut self, event: &KeyEvent) {
        let ctrl = self.mods.control_key();
        let Some(input) = &mut self.overlay_prompt else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.overlay_prompt = None,
            Key::Named(NamedKey::Enter) => {
                let command = input.take().trim().to_owned();
                self.overlay_prompt = None;
                if command.is_empty() {
                } else if self.overlay_prompt_window {
                    self.run_window_overlay(command);
                } else if let Err(err) = self.open_overlay(Some(command), None, None) {
                    self.notify(
                        None,
                        &tr!("toast.cannot_start"),
                        &format!("{err:#}"),
                        Level::Error,
                        Source::App,
                    );
                }
            }
            Key::Named(NamedKey::Backspace) => input.backspace(),
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
    fn each_shell_runs_a_command_its_way() {
        assert_eq!(
            shell_run_args("powershell.exe", "yazi C:/x"),
            ["-NoLogo", "-NoProfile", "-Command", "yazi C:/x"]
        );
        assert_eq!(
            shell_run_args(r"C:\Program Files\PowerShell\7\pwsh.exe", "htop")[2..],
            ["-Command", "htop"]
        );
        assert_eq!(shell_run_args("cmd.exe", "dir /w"), ["/c", "dir /w"]);
        assert_eq!(shell_run_args("CMD", "x"), ["/c", "x"]);
        assert_eq!(shell_run_args("/bin/zsh", "lazygit"), ["-c", "lazygit"]);
        assert_eq!(shell_run_args("bash", "a | b"), ["-c", "a | b"]);
        assert_eq!(shell_run_args("fish", "x"), ["-c", "x"]);
    }
}
