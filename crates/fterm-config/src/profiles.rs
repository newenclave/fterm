//! Profiles: what to start in a new tab or pane (a shell or an AI tool).

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// The start folder. `None` = the folder of fterm.
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl Profile {
    pub fn new(name: &str, command: &str, args: &[&str]) -> Self {
        Self {
            name: name.to_owned(),
            command: command.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            cwd: None,
            env: Vec::new(),
        }
    }
}

/// Profiles that fterm finds on this computer, when the config has none.
/// `which(name)` = the program is in PATH; `exists(path)` = the file is there.
pub fn detect_profiles(
    windows: bool,
    which: impl Fn(&str) -> bool,
    exists: impl Fn(&Path) -> bool,
) -> Vec<Profile> {
    let mut profiles = Vec::new();
    if windows {
        if which("pwsh") {
            profiles.push(Profile::new("PowerShell", "pwsh.exe", &[]));
        }
        profiles.push(Profile::new("Windows PowerShell", "powershell.exe", &[]));
        profiles.push(Profile::new("Command Prompt", "cmd.exe", &[]));
        for bash in [
            r"C:\Program Files\Git\bin\bash.exe",
            r"C:\Program Files (x86)\Git\bin\bash.exe",
        ] {
            if exists(Path::new(bash)) {
                profiles.push(Profile::new("Git Bash", bash, &["--login", "-i"]));
                break;
            }
        }
        if which("wsl") {
            profiles.push(Profile::new("WSL", "wsl.exe", &[]));
        }
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
        profiles.push(Profile::new("Shell", &shell, &[]));
        for name in ["bash", "zsh", "fish"] {
            if which(name) {
                profiles.push(Profile::new(name, name, &[]));
            }
        }
    }
    // AI tools in PATH.
    for (name, program, args) in [
        ("Claude", "claude", &[][..]),
        ("OpenCode", "opencode", &[][..]),
        // The model can be changed in the config.
        ("Ollama", "ollama", &["run", "llama3.2"][..]),
        ("OpenClaude", "openclaude", &[][..]),
    ] {
        if which(program) {
            profiles.push(Profile::new(name, program, args));
        }
    }
    profiles
}

/// `~/code` → `<home>/code`. Other paths do not change.
pub fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home.to_path_buf(),
        (Some(rest), Some(home)) if rest.starts_with(['/', '\\']) => home.join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

/// The home folder of the user.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// True when `program` is in PATH (on Windows also with `.exe`, `.cmd`, `.bat`).
pub fn which(program: &str) -> bool {
    path_extension(program).is_some()
}

/// The program and arguments to start a profile.
/// On Windows a `.cmd` or `.bat` file (for example, a tool from npm) cannot start alone:
/// it runs through `cmd.exe /c`. `extension_of(name)` gives the extension found in PATH.
pub fn launch_command(
    profile: &Profile,
    windows: bool,
    extension_of: impl Fn(&str) -> Option<String>,
) -> (String, Vec<String>) {
    let command = profile.command.clone();
    let args = profile.args.clone();
    if !windows {
        return (command, args);
    }
    let lower = command.to_ascii_lowercase();
    let script = lower.ends_with(".cmd")
        || lower.ends_with(".bat")
        || (Path::new(&command).extension().is_none()
            && extension_of(&command).is_some_and(|ext| {
                ext.eq_ignore_ascii_case(".cmd") || ext.eq_ignore_ascii_case(".bat")
            }));
    if !script {
        return (command, args);
    }
    let mut all = vec!["/c".to_owned(), command];
    all.extend(args);
    ("cmd.exe".to_owned(), all)
}

/// The first extension that `program` has in PATH: `""` (no extension), `.exe`, `.cmd`, ...
pub fn path_extension(program: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    let extensions: &[&str] = if cfg!(windows) {
        &[".exe", ".com", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    std::env::split_paths(&path).find_map(|dir| {
        extensions
            .iter()
            .find(|ext| dir.join(format!("{program}{ext}")).is_file())
            .map(|ext| (*ext).to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(profiles: &[Profile]) -> Vec<&str> {
        profiles.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn windows_with_everything() {
        let profiles = detect_profiles(
            true,
            |name| ["pwsh", "wsl", "claude", "opencode", "ollama", "openclaude"].contains(&name),
            |path| path.ends_with("bash.exe"),
        );
        assert_eq!(
            names(&profiles),
            [
                "PowerShell",
                "Windows PowerShell",
                "Command Prompt",
                "Git Bash",
                "WSL",
                "Claude",
                "OpenCode",
                "Ollama",
                "OpenClaude",
            ]
        );
        let git_bash = &profiles[3];
        assert!(git_bash.command.ends_with("bash.exe"));
        assert_eq!(git_bash.args, ["--login", "-i"]);
        assert_eq!(profiles[0].command, "pwsh.exe");
    }

    #[test]
    fn windows_with_nothing_extra() {
        let profiles = detect_profiles(true, |_| false, |_| false);
        // Windows PowerShell and cmd are always there.
        assert_eq!(names(&profiles), ["Windows PowerShell", "Command Prompt"]);
    }

    #[test]
    fn unix_profiles() {
        let profiles = detect_profiles(
            false,
            |name| ["zsh", "bash", "claude"].contains(&name),
            |_| false,
        );
        let names = names(&profiles);
        assert_eq!(names[0], "Shell", "the login shell first");
        assert!(names.contains(&"zsh") && names.contains(&"bash"));
        assert!(names.contains(&"Claude"));
        assert!(!names.contains(&"fish"));
    }

    #[test]
    fn home_is_expanded() {
        let home = Path::new("/home/me");
        assert_eq!(
            expand_home("~/code", Some(home)),
            Path::new("/home/me/code")
        );
        assert_eq!(expand_home("~", Some(home)), Path::new("/home/me"));
        assert_eq!(expand_home("/tmp", Some(home)), Path::new("/tmp"));
        assert_eq!(expand_home("~/code", None), Path::new("~/code"));
    }

    #[test]
    fn npm_scripts_run_through_cmd() {
        let found = |name: &str| match name {
            "claude" => Some(".exe".to_owned()),
            "opencode" => Some(".cmd".to_owned()),
            _ => None,
        };
        let claude = Profile::new("Claude", "claude", &["--continue"]);
        assert_eq!(
            launch_command(&claude, true, found),
            ("claude".to_owned(), vec!["--continue".to_owned()])
        );
        let opencode = Profile::new("OpenCode", "opencode", &["run"]);
        assert_eq!(
            launch_command(&opencode, true, found),
            (
                "cmd.exe".to_owned(),
                vec!["/c".to_owned(), "opencode".to_owned(), "run".to_owned()]
            )
        );
        // A full path to a .bat file.
        let bat = Profile::new("Tool", r"C:	ools\go.BAT", &[]);
        assert_eq!(launch_command(&bat, true, found).0, "cmd.exe");
        // Not on Windows: no change.
        assert_eq!(launch_command(&opencode, false, found).0, "opencode");
    }

    #[test]
    fn which_finds_real_programs() {
        if cfg!(windows) {
            assert!(which("cmd"));
            assert!(which("cmd.exe"));
        } else {
            assert!(which("sh"));
        }
        assert!(!which("surely-not-a-program-123"));
    }
}
