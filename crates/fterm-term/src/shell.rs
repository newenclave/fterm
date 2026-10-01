//! What the shell told us with shell integration (OSC 7 and OSC 133): the folder, and the commands.

use std::time::{Duration, Instant};

use crate::osc::{OscEvent, PromptMark};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellEvent {
    /// A command ended. `took` is from the start of the command (OSC 133 C) to its end (D).
    CommandDone { exit: Option<i32>, took: Duration },
}

#[derive(Clone, Debug, Default)]
pub struct ShellState {
    /// The current folder, if the shell tells it.
    pub cwd: Option<String>,
    /// When the running command started.
    running_since: Option<Instant>,
}

impl ShellState {
    /// Takes one OSC event. Returns an event when a command ended.
    pub fn apply(&mut self, event: &OscEvent, now: Instant) -> Option<ShellEvent> {
        match event {
            OscEvent::Cwd(dir) => {
                self.cwd = Some(dir.clone());
                None
            }
            OscEvent::Prompt(PromptMark::CommandExecuted) => {
                self.running_since = Some(now);
                None
            }
            OscEvent::Prompt(PromptMark::CommandFinished(exit)) => {
                let since = self.running_since.take()?;
                Some(ShellEvent::CommandDone {
                    exit: *exit,
                    took: now.saturating_duration_since(since),
                })
            }
            _ => None,
        }
    }

    /// True while a command runs (between OSC 133 C and D).
    pub fn is_running(&self) -> bool {
        self.running_since.is_some()
    }
}

/// The shell integration scripts that come with fterm.
pub const POWERSHELL_SCRIPT: &str = include_str!("../../../assets/shell/fterm.ps1");
pub const BASH_SCRIPT: &str = include_str!("../../../assets/shell/fterm.bash");
pub const ZSH_SCRIPT: &str = include_str!("../../../assets/shell/fterm.zsh");

/// Writes the scripts into `dir` (if they changed) and returns the path of the PowerShell script.
pub fn install_scripts(dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    for (name, text) in [
        ("fterm.ps1", POWERSHELL_SCRIPT),
        ("fterm.bash", BASH_SCRIPT),
        ("fterm.zsh", ZSH_SCRIPT),
    ] {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
            std::fs::write(&path, text)?;
        }
    }
    Ok(dir.join("fterm.ps1"))
}

/// True for `powershell`, `pwsh`, with or without a folder and `.exe`.
pub fn is_powershell(command: &str) -> bool {
    let file = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .to_ascii_lowercase();
    let name = file.strip_suffix(".exe").unwrap_or(&file);
    name == "powershell" || name == "pwsh"
}

/// The arguments that start PowerShell with the integration script (after the user's profile).
/// The user's own arguments come first; `-NoExit -Command` is added only when they have no `-Command` or `-File`.
pub fn powershell_args(args: &[String], script: &std::path::Path) -> Vec<String> {
    let has_command = args.iter().any(|a| {
        let a = a.to_ascii_lowercase();
        ["-command", "-c", "-file", "-f", "-encodedcommand", "-ec"].contains(&a.as_str())
    });
    if has_command {
        return args.to_vec();
    }
    // In a PowerShell single-quoted string, `'` is written as `''`.
    let path = script.display().to_string().replace('\'', "''");
    let mut out = args.to_vec();
    out.extend([
        "-NoExit".to_owned(),
        "-Command".to_owned(),
        format!(". '{path}'"),
    ]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(m: PromptMark) -> OscEvent {
        OscEvent::Prompt(m)
    }

    #[test]
    fn powershell_names() {
        assert!(is_powershell("powershell.exe"));
        assert!(is_powershell("pwsh"));
        assert!(is_powershell(r"C:\Program Files\PowerShell\PWSH.EXE"));
        assert!(!is_powershell("cmd.exe"));
        assert!(!is_powershell("bash"));
    }

    #[test]
    fn powershell_args_load_the_script() {
        let script = std::path::Path::new("C:/tmp/fterm.ps1");
        let args = powershell_args(&["-NoLogo".to_owned()], script);
        assert_eq!(
            args,
            ["-NoLogo", "-NoExit", "-Command", ". 'C:/tmp/fterm.ps1'"]
        );
        // A user command is not changed.
        let user = vec!["-Command".to_owned(), "Get-Date".to_owned()];
        assert_eq!(powershell_args(&user, script), user);
        let file = vec!["-File".to_owned(), "x.ps1".to_owned()];
        assert_eq!(powershell_args(&file, script), file);
    }

    #[test]
    fn scripts_are_written_once() {
        let dir = std::env::temp_dir().join(format!("fterm-shell-test-{}", std::process::id()));
        let path = install_scripts(&dir).unwrap();
        assert!(path.ends_with("fterm.ps1"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), POWERSHELL_SCRIPT);
        assert!(dir.join("fterm.bash").exists() && dir.join("fterm.zsh").exists());
        // A second call works too.
        assert_eq!(install_scripts(&dir).unwrap(), path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cwd_is_kept() {
        let mut shell = ShellState::default();
        assert_eq!(
            shell.apply(&OscEvent::Cwd("C:/work".into()), Instant::now()),
            None
        );
        assert_eq!(shell.cwd.as_deref(), Some("C:/work"));
    }

    #[test]
    fn a_command_from_start_to_end() {
        let mut shell = ShellState::default();
        let t0 = Instant::now();
        shell.apply(&mark(PromptMark::PromptStart), t0);
        shell.apply(&mark(PromptMark::CommandStart), t0);
        assert!(!shell.is_running(), "typing is not running");
        shell.apply(&mark(PromptMark::CommandExecuted), t0);
        assert!(shell.is_running());
        let done = shell.apply(
            &mark(PromptMark::CommandFinished(Some(1))),
            t0 + Duration::from_secs(12),
        );
        assert_eq!(
            done,
            Some(ShellEvent::CommandDone {
                exit: Some(1),
                took: Duration::from_secs(12)
            })
        );
        assert!(!shell.is_running());
    }

    #[test]
    fn end_without_a_start_gives_nothing() {
        // PowerShell without PSReadLine sends no C, so there is no time to measure.
        let mut shell = ShellState::default();
        assert_eq!(
            shell.apply(&mark(PromptMark::CommandFinished(Some(0))), Instant::now()),
            None
        );
    }

    #[test]
    fn notifications_do_not_change_the_state() {
        let mut shell = ShellState::default();
        let t0 = Instant::now();
        shell.apply(&mark(PromptMark::CommandExecuted), t0);
        let note = OscEvent::Notify {
            title: None,
            body: "x".into(),
        };
        assert_eq!(shell.apply(&note, t0), None);
        assert!(shell.is_running());
    }
}
