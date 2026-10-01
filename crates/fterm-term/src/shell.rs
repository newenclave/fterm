//! What the shell told us with shell integration (OSC 7 and OSC 133): the folder, and the commands.

use std::time::{Duration, Instant};

use crate::osc::{OscEvent, PromptMark};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellEvent {
    /// A command ended. `took` is from the start of the command (OSC 133 C) to its end (D).
    /// `command` is its text (OSC 633;E) and `cwd` the folder where it started, when the shell tells them.
    CommandDone {
        exit: Option<i32>,
        took: Duration,
        command: Option<String>,
        cwd: Option<String>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct ShellState {
    /// The current folder, if the shell tells it.
    pub cwd: Option<String>,
    /// When the running command started.
    running_since: Option<Instant>,
    /// The running command: its text and the folder where it started.
    running: (Option<String>, Option<String>),
    /// The text of the next command (633;E comes just before 133;C).
    typed: Option<String>,
    /// Where the typed text starts (history line, column), while the shell waits for a command.
    input_start: Option<(usize, usize)>,
    /// The lines of the last output: (start, end). `end` is `None` while the command runs.
    output: Option<(usize, Option<usize>)>,
    /// The next `OutputMark` is the start (after 133;C) or the end (after 133;D) of an output.
    next_mark: Option<OutputMarkKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputMarkKind {
    Start,
    End,
}

impl ShellState {
    /// Takes one OSC event. Returns an event when a command ended.
    pub fn apply(&mut self, event: &OscEvent, now: Instant) -> Option<ShellEvent> {
        match event {
            OscEvent::Cwd(dir) => {
                self.cwd = Some(dir.clone());
                None
            }
            OscEvent::Prompt(PromptMark::PromptStart | PromptMark::CommandStart) => {
                self.input_start = None;
                None
            }
            OscEvent::InputStart { line, column } => {
                self.input_start = Some((*line, *column));
                None
            }
            OscEvent::CommandLine(text) => {
                self.typed = Some(text.clone());
                None
            }
            OscEvent::OutputMark { line, column } => {
                match self.next_mark.take() {
                    // The cursor can still be after the typed command: then the output starts below it.
                    Some(OutputMarkKind::Start) => {
                        let start = if *column > 0 { line + 1 } else { *line };
                        self.output = Some((start, None));
                    }
                    Some(OutputMarkKind::End) => {
                        if let Some((_, end)) = &mut self.output {
                            *end = Some(*line);
                        }
                    }
                    None => {}
                }
                None
            }
            OscEvent::Prompt(PromptMark::CommandExecuted) => {
                self.next_mark = Some(OutputMarkKind::Start);
                self.running_since = Some(now);
                self.running = (self.typed.take(), self.cwd.clone());
                self.input_start = None;
                None
            }
            OscEvent::Prompt(PromptMark::CommandFinished(exit)) => {
                self.next_mark = Some(OutputMarkKind::End);
                let since = self.running_since.take()?;
                let (command, cwd) = std::mem::take(&mut self.running);
                Some(ShellEvent::CommandDone {
                    exit: *exit,
                    took: now.saturating_duration_since(since),
                    command,
                    cwd,
                })
            }
            _ => None,
        }
    }

    /// Where the typed text starts: (line from the top of the history, column).
    /// `None` when a command runs, or the shell has no integration.
    pub fn input_start(&self) -> Option<(usize, usize)> {
        self.input_start
    }

    /// The lines (from the top of the history) of the last command output: `start..end`.
    /// While the command runs, the end is `total` (all lines now).
    pub fn last_output(&self, total: usize) -> Option<(usize, usize)> {
        match self.output? {
            (start, Some(end)) => Some((start, end.max(start))),
            (start, None) => Some((start, total.max(start))),
        }
    }

    /// The shell waits for a command, and we know where the typed text starts.
    pub fn at_prompt(&self) -> bool {
        self.input_start.is_some() && !self.is_running()
    }

    /// The text of the running command, when the shell told it (OSC 633;E).
    pub fn running_command(&self) -> Option<&str> {
        if !self.is_running() {
            return None;
        }
        self.running.0.as_deref()
    }

    /// True while a command runs (between OSC 133 C and D).
    pub fn is_running(&self) -> bool {
        self.running_since.is_some()
    }
}

/// The arguments of `wsl.exe` for a pane of `distro` in `cwd` (`~`, a Linux path, or a Windows path).
/// With `script` (the Windows path of `fterm-wsl.bash`), bash starts with the fterm shell integration.
pub fn wsl_args(distro: &str, cwd: &str, script: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec!["-d".into(), distro.into(), "--cd".into(), cwd.into()];
    if let Some(script) = script {
        let quoted = script.replace('\'', r"'\''");
        // Only bash gets the script; zsh, fish, and others start as they are.
        let launcher = format!(
            r#"f=$(wslpath '{quoted}' 2>/dev/null); case "${{SHELL##*/}}" in bash) [ -r "$f" ] && exec bash --rcfile "$f" -i;; esac; exec "${{SHELL:-/bin/sh}}" -l"#
        );
        args.extend(["--exec".into(), "sh".into(), "-c".into(), launcher]);
    }
    args
}

/// The folder for `wsl.exe --cd` when a pane of `distro` opens from `cwd` (the folder of another pane,
/// or the saved one): a Linux path as it is, a Windows path, or `~` when WSL cannot open it.
pub fn wsl_cwd(distro: &str, cwd: Option<&str>) -> String {
    let Some(cwd) = cwd.filter(|c| !c.is_empty()) else {
        return "~".to_owned();
    };
    let windows = cwd.replace('/', "\\");
    if let Some(unc) = windows.strip_prefix(r"\\") {
        // `\\wsl$\Ubuntu\home\me` or `\\wsl.localhost\Ubuntu\home\me`.
        let mut parts = unc.splitn(3, '\\');
        let (host, name, rest) = (parts.next(), parts.next(), parts.next().unwrap_or(""));
        let wsl_host = host.is_some_and(|h| {
            h.eq_ignore_ascii_case("wsl$") || h.eq_ignore_ascii_case("wsl.localhost")
        });
        return match name {
            Some(name) if wsl_host && name.eq_ignore_ascii_case(distro) => {
                format!("/{}", rest.replace('\\', "/").trim_end_matches('/'))
            }
            _ => "~".to_owned(),
        };
    }
    if cwd.starts_with('/') {
        return cwd.to_owned();
    }
    let drive = cwd.as_bytes();
    if drive.len() >= 2 && drive[0].is_ascii_alphabetic() && drive[1] == b':' {
        return windows;
    }
    "~".to_owned()
}

/// The program is bash (`bash`, `/bin/bash`, `C:\\...\\bash.exe`).
pub fn is_bash(program: &str) -> bool {
    program_name(program) == "bash"
}

/// `C:\\Git\\bin\\bash.exe` -> `bash`.
fn program_name(program: &str) -> String {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let lower = file.to_ascii_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
}

/// The program is zsh.
pub fn is_zsh(program: &str) -> bool {
    program_name(program) == "zsh"
}

/// The arguments that start bash with the fterm shell integration (`scripts` = the folder of the scripts).
/// `None` = leave the arguments as they are (bash runs a command or a script, or has its own rc file).
pub fn bash_args(args: &[String], scripts: &std::path::Path) -> Option<Vec<String>> {
    let mut login = false;
    for arg in args {
        match arg.as_str() {
            "-i" => {}
            "--login" | "-l" | "-il" | "-li" => login = true,
            // A command, a script, its own rc file, or another mode: bash does its own work.
            _ => return None,
        }
    }
    let name = if login {
        "fterm-login.bash"
    } else {
        "fterm-rc.bash"
    };
    // Git Bash takes `C:/...`; `\` would be an escape there.
    let rc = scripts.join(name).display().to_string().replace('\\', "/");
    Some(vec!["--rcfile".to_owned(), rc, "-i".to_owned()])
}

/// The env vars that start zsh with the fterm shell integration: `ZDOTDIR` goes to the fterm zsh folder,
/// and `FTERM_USER_ZDOTDIR` keeps the user's own one (`zdotdir`, or empty).
pub fn zsh_env(zdotdir: Option<&str>, scripts: &std::path::Path) -> Vec<(String, String)> {
    let ours = scripts.join("zsh").display().to_string().replace('\\', "/");
    vec![
        ("ZDOTDIR".to_owned(), ours),
        (
            "FTERM_USER_ZDOTDIR".to_owned(),
            zdotdir.unwrap_or_default().to_owned(),
        ),
    ]
}

/// The shell integration scripts that come with fterm.
pub const POWERSHELL_SCRIPT: &str = include_str!("../../../assets/shell/fterm.ps1");
pub const BASH_SCRIPT: &str = include_str!("../../../assets/shell/fterm.bash");
pub const ZSH_SCRIPT: &str = include_str!("../../../assets/shell/fterm.zsh");
/// The `--rcfile` of a login bash (WSL, Git Bash): what a login bash reads, and then `fterm.bash`.
pub const LOGIN_BASH_SCRIPT: &str = include_str!("../../../assets/shell/fterm-login.bash");
/// The `--rcfile` of a normal bash (Linux): what bash reads, and then `fterm.bash`.
pub const RC_BASH_SCRIPT: &str = include_str!("../../../assets/shell/fterm-rc.bash");
/// The zsh files for `ZDOTDIR`: they load the user's own files, and `.zshrc` loads `fterm.zsh` too.
pub const ZSH_ZSHENV: &str = include_str!("../../../assets/shell/zsh/.zshenv");
pub const ZSH_ZPROFILE: &str = include_str!("../../../assets/shell/zsh/.zprofile");
pub const ZSH_ZSHRC: &str = include_str!("../../../assets/shell/zsh/.zshrc");

/// Writes the scripts into `dir` (if they changed) and returns the path of the PowerShell script.
pub fn install_scripts(dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    for (name, text) in [
        ("fterm.ps1", POWERSHELL_SCRIPT),
        ("fterm.bash", BASH_SCRIPT),
        ("fterm.zsh", ZSH_SCRIPT),
        ("fterm-login.bash", LOGIN_BASH_SCRIPT),
        ("fterm-rc.bash", RC_BASH_SCRIPT),
        ("zsh/.zshenv", ZSH_ZSHENV),
        ("zsh/.zprofile", ZSH_ZPROFILE),
        ("zsh/.zshrc", ZSH_ZSHRC),
    ] {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
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
                took: Duration::from_secs(12),
                command: None,
                cwd: None,
            })
        );
        assert!(!shell.is_running());
    }

    #[test]
    fn the_command_text_and_the_folder_come_with_the_end() {
        let mut shell = ShellState::default();
        let t0 = Instant::now();
        shell.apply(&OscEvent::Cwd("C:/work".into()), t0);
        shell.apply(&mark(PromptMark::CommandStart), t0);
        shell.apply(&OscEvent::CommandLine("cargo test".into()), t0);
        shell.apply(&mark(PromptMark::CommandExecuted), t0);
        // The folder can change while the command runs (`cd`): the command ran in the old one.
        shell.apply(&OscEvent::Cwd("C:/other".into()), t0);
        let done = shell.apply(&mark(PromptMark::CommandFinished(Some(0))), t0);
        assert_eq!(
            done,
            Some(ShellEvent::CommandDone {
                exit: Some(0),
                took: Duration::ZERO,
                command: Some("cargo test".into()),
                cwd: Some("C:/work".into()),
            })
        );
        // The next command has no text until its own 633;E.
        shell.apply(&mark(PromptMark::CommandExecuted), t0);
        let Some(ShellEvent::CommandDone { command, .. }) =
            shell.apply(&mark(PromptMark::CommandFinished(None)), t0)
        else {
            panic!("a command ended");
        };
        assert_eq!(command, None);
    }

    #[test]
    fn bash_and_zsh_by_name() {
        for bash in [
            "bash",
            "/bin/bash",
            r"C:\Program Files\Git\bin\bash.exe",
            "BASH.EXE",
        ] {
            assert!(is_bash(bash), "{bash}");
        }
        for zsh in ["zsh", "/usr/bin/zsh", "/bin/zsh"] {
            assert!(is_zsh(zsh), "{zsh}");
        }
        assert!(!is_bash("bashful") && !is_bash("zsh") && !is_zsh("bash"));
    }

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn bash_gets_the_rc_file() {
        let dir = std::path::Path::new("/home/me/.local/share/fterm/shell");
        assert_eq!(
            bash_args(&[], dir),
            Some(strings(&[
                "--rcfile",
                "/home/me/.local/share/fterm/shell/fterm-rc.bash",
                "-i"
            ]))
        );
        assert_eq!(bash_args(&strings(&["-i"]), dir), bash_args(&[], dir));
        // A login shell (Git Bash: `--login -i`) reads the profile files: bash does not take
        // --rcfile with --login, so the login script does that.
        for login in [&["--login", "-i"][..], &["-l"][..], &["-il"][..]] {
            assert_eq!(
                bash_args(&strings(login), dir),
                Some(strings(&[
                    "--rcfile",
                    "/home/me/.local/share/fterm/shell/fterm-login.bash",
                    "-i"
                ])),
                "{login:?}"
            );
        }
    }

    #[test]
    fn bash_keeps_its_own_work() {
        let dir = std::path::Path::new("/s");
        for args in [
            &["-c", "make"][..],
            &["script.sh"][..],
            &["--rcfile", "my.rc"][..],
            &["--norc"][..],
            &["--posix"][..],
        ] {
            assert_eq!(bash_args(&strings(args), dir), None, "{args:?}");
        }
    }

    #[test]
    fn a_windows_folder_has_forward_slashes_for_bash() {
        let dir = std::path::Path::new(r"C:\Users\me\AppData\Local\fterm\shell");
        assert_eq!(
            bash_args(&[], dir).unwrap()[1],
            "C:/Users/me/AppData/Local/fterm/shell/fterm-rc.bash"
        );
    }

    #[test]
    fn zsh_starts_in_the_fterm_folder() {
        let dir = std::path::Path::new("/home/me/.local/share/fterm/shell");
        assert_eq!(
            zsh_env(None, dir),
            [
                (
                    "ZDOTDIR".to_owned(),
                    "/home/me/.local/share/fterm/shell/zsh".to_owned()
                ),
                ("FTERM_USER_ZDOTDIR".to_owned(), String::new()),
            ]
        );
        assert_eq!(
            zsh_env(Some("/home/me/.config/zsh"), dir)[1].1,
            "/home/me/.config/zsh"
        );
    }

    #[test]
    fn the_wsl_folder_of_a_new_pane() {
        // No folder: home.
        assert_eq!(wsl_cwd("Ubuntu", None), "~");
        // A Linux folder (from a WSL pane): as it is.
        assert_eq!(wsl_cwd("Ubuntu", Some("/tmp/a b")), "/tmp/a b");
        // A Windows folder (OSC 7 of PowerShell has `/`): wsl.exe takes a Windows path.
        assert_eq!(wsl_cwd("Ubuntu", Some("C:/work/fterm")), r"C:\work\fterm");
        // A folder of the same distro, seen from Windows: the Linux path.
        assert_eq!(
            wsl_cwd("Ubuntu", Some(r"\\wsl$\Ubuntu\home\me")),
            "/home/me"
        );
        assert_eq!(
            wsl_cwd("ubuntu", Some("//wsl.localhost/Ubuntu/home/me/src")),
            "/home/me/src"
        );
        assert_eq!(wsl_cwd("Ubuntu", Some(r"\\wsl.localhost\Ubuntu")), "/");
        // A folder of another distro cannot be opened here.
        assert_eq!(wsl_cwd("Debian", Some(r"\\wsl$\Ubuntu\home\me")), "~");
        // Other network folders neither.
        assert_eq!(wsl_cwd("Ubuntu", Some(r"\\server\share")), "~");
    }

    #[test]
    fn a_wsl_pane_without_integration() {
        assert_eq!(wsl_args("Ubuntu", "~", None), ["-d", "Ubuntu", "--cd", "~"]);
        assert_eq!(
            wsl_args("Debian", "/tmp", None),
            ["-d", "Debian", "--cd", "/tmp"]
        );
    }

    #[test]
    fn a_wsl_pane_loads_the_bash_script() {
        let args = wsl_args(
            "Ubuntu",
            "~",
            Some(r"C:\Users\me\AppData\Local\fterm\shell\fterm-wsl.bash"),
        );
        assert_eq!(args[..6], ["-d", "Ubuntu", "--cd", "~", "--exec", "sh"]);
        assert_eq!(args[6], "-c");
        let launcher = &args[7];
        // Linux finds the Windows file itself (the drives are not always in /mnt).
        assert!(
            launcher.contains(r"wslpath 'C:\Users\me\AppData\Local\fterm\shell\fterm-wsl.bash'"),
            "{launcher}"
        );
        assert!(
            launcher.contains(r#"exec bash --rcfile "$f" -i"#),
            "{launcher}"
        );
        // Other shells (zsh, fish) start as they are, as a login shell.
        assert!(
            launcher.contains(r#"exec "${SHELL:-/bin/sh}" -l"#),
            "{launcher}"
        );
    }

    #[test]
    fn a_quote_in_the_script_path_is_safe() {
        let args = wsl_args("Ubuntu", "~", Some(r"C:\Users\o'neil\fterm-wsl.bash"));
        assert!(
            args[7].contains(r"wslpath 'C:\Users\o'\''neil\fterm-wsl.bash'"),
            "{}",
            args[7]
        );
    }

    #[test]
    fn the_text_of_the_running_command() {
        let mut shell = ShellState::default();
        let t0 = Instant::now();
        assert_eq!(shell.running_command(), None);
        shell.apply(&OscEvent::CommandLine("npm run dev".into()), t0);
        assert_eq!(
            shell.running_command(),
            None,
            "typed, but it does not run yet"
        );
        shell.apply(&mark(PromptMark::CommandExecuted), t0);
        assert_eq!(shell.running_command(), Some("npm run dev"));
        shell.apply(&mark(PromptMark::CommandFinished(Some(0))), t0);
        assert_eq!(shell.running_command(), None);
    }

    #[test]
    fn the_lines_of_the_last_output() {
        let mut shell = ShellState::default();
        let t0 = Instant::now();
        assert_eq!(shell.last_output(100), None);
        shell.apply(&mark(PromptMark::CommandExecuted), t0);
        shell.apply(
            &OscEvent::OutputMark {
                line: 10,
                column: 0,
            },
            t0,
        );
        // While it runs: from the start to the end of the history now.
        assert_eq!(shell.last_output(14), Some((10, 14)));
        shell.apply(&mark(PromptMark::CommandFinished(Some(0))), t0);
        shell.apply(
            &OscEvent::OutputMark {
                line: 13,
                column: 0,
            },
            t0,
        );
        assert_eq!(shell.last_output(50), Some((10, 13)), "it ended at line 13");
        // The cursor was still after the typed command at 133;C: the output starts on the next line.
        shell.apply(&mark(PromptMark::CommandExecuted), t0);
        shell.apply(
            &OscEvent::OutputMark {
                line: 20,
                column: 5,
            },
            t0,
        );
        shell.apply(&mark(PromptMark::CommandFinished(Some(1))), t0);
        shell.apply(
            &OscEvent::OutputMark {
                line: 22,
                column: 0,
            },
            t0,
        );
        assert_eq!(shell.last_output(50), Some((21, 22)));
    }

    #[test]
    fn the_input_start_is_known_only_at_the_prompt() {
        let mut shell = ShellState::default();
        let t0 = Instant::now();
        assert_eq!(shell.input_start(), None);
        shell.apply(&mark(PromptMark::CommandStart), t0);
        shell.apply(
            &OscEvent::InputStart {
                line: 40,
                column: 7,
            },
            t0,
        );
        assert_eq!(shell.input_start(), Some((40, 7)));
        assert!(shell.at_prompt());
        shell.apply(&mark(PromptMark::CommandExecuted), t0);
        assert_eq!(shell.input_start(), None, "the command runs: no typing now");
        assert!(!shell.at_prompt());
        shell.apply(&mark(PromptMark::CommandFinished(Some(0))), t0);
        shell.apply(&mark(PromptMark::PromptStart), t0);
        assert!(
            !shell.at_prompt(),
            "the prompt is drawn, but there is no input place yet"
        );
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
