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
    /// The WSL distro of the profile (its panes have Linux folders).
    pub wsl: Option<String>,
    /// The color of the tabs that this profile opens.
    pub tab_color: Option<[u8; 3]>,
    /// The colors of its programs may fit the theme (`harmonize = false`: exact colors, for example btop).
    pub harmonize: bool,
    /// Its programs may change the palette (`None` = the value of the config).
    pub palette_changes: Option<bool>,
}

impl Profile {
    pub fn new(name: &str, command: &str, args: &[&str]) -> Self {
        Self {
            name: name.to_owned(),
            command: command.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            cwd: None,
            env: Vec::new(),
            wsl: None,
            tab_color: None,
            harmonize: true,
            palette_changes: None,
        }
    }
}

/// The names in the output of `wsl.exe -l -q` (UTF-16LE). Docker's own distros are not for people.
pub fn wsl_distros(output: &[u8]) -> Vec<String> {
    let units: Vec<u16> = output
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .lines()
        .map(|line| line.trim_matches(|c: char| c == '\u{feff}' || c.is_whitespace() || c == '\0'))
        .filter(|name| !name.is_empty() && !name.starts_with("docker-desktop"))
        .map(str::to_owned)
        .collect()
}

/// The WSL distros of this computer (`wsl.exe -l -q`, with no console window).
pub fn installed_wsl_distros() -> Vec<String> {
    let mut command = std::process::Command::new("wsl.exe");
    command.args(["-l", "-q"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    match command.output() {
        Ok(out) if out.status.success() => wsl_distros(&out.stdout),
        _ => Vec::new(),
    }
}

impl Profile {
    /// A profile that opens a WSL distro in its home folder.
    pub fn wsl(name: &str, distro: &str) -> Self {
        Self {
            wsl: Some(distro.to_owned()),
            ..Self::new(name, "wsl.exe", &["-d", distro, "--cd", "~"])
        }
    }
}

/// Profiles that fterm finds on this computer, when the config has none.
/// `which(name)` = the program is in PATH; `exists(path)` = the file is there.
pub fn detect_profiles(
    windows: bool,
    which: impl Fn(&str) -> bool,
    exists: impl Fn(&Path) -> bool,
    distros: &[String],
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
            for distro in distros {
                profiles.push(Profile::wsl(distro, distro));
            }
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
    fn the_list_of_wsl_distros() {
        // `wsl.exe -l -q` writes UTF-16LE, with CRLF, sometimes with a BOM.
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "Ubuntu\r\nopenSUSE-Tumbleweed\r\n\r\ndocker-desktop\r\ndocker-desktop-data\r\n"
            .encode_utf16()
        {
            bytes.extend(unit.to_le_bytes());
        }
        assert_eq!(wsl_distros(&bytes), ["Ubuntu", "openSUSE-Tumbleweed"]);
        assert_eq!(wsl_distros(b""), Vec::<String>::new());
    }

    #[test]
    fn a_profile_for_each_wsl_distro() {
        let distros = ["Ubuntu".to_owned(), "Debian".to_owned()];
        let profiles = detect_profiles(true, |name| name == "wsl", |_| false, &distros);
        assert_eq!(
            names(&profiles),
            ["Windows PowerShell", "Command Prompt", "Ubuntu", "Debian"]
        );
        let ubuntu = &profiles[2];
        assert_eq!(ubuntu.command, "wsl.exe");
        assert_eq!(ubuntu.args, ["-d", "Ubuntu", "--cd", "~"]);
        assert_eq!(ubuntu.wsl.as_deref(), Some("Ubuntu"));
        assert_eq!(profiles[0].wsl, None);
        // wsl.exe is on every Windows 11, also without WSL: no distros, no profile.
        let profiles = detect_profiles(true, |name| name == "wsl", |_| false, &[]);
        assert_eq!(names(&profiles), ["Windows PowerShell", "Command Prompt"]);
    }

    #[test]
    fn windows_with_everything() {
        let profiles = detect_profiles(
            true,
            |name| ["pwsh", "wsl", "claude", "opencode", "ollama", "openclaude"].contains(&name),
            // A Windows path is one name on Unix (`\` is not a separator there).
            |path| path.to_string_lossy().ends_with("bash.exe"),
            &["Ubuntu".to_owned()],
        );
        assert_eq!(
            names(&profiles),
            [
                "PowerShell",
                "Windows PowerShell",
                "Command Prompt",
                "Git Bash",
                "Ubuntu",
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
        let profiles = detect_profiles(true, |_| false, |_| false, &[]);
        // Windows PowerShell and cmd are always there.
        assert_eq!(names(&profiles), ["Windows PowerShell", "Command Prompt"]);
    }

    #[test]
    fn unix_profiles() {
        let profiles = detect_profiles(
            false,
            |name| ["zsh", "bash", "claude"].contains(&name),
            |_| false,
            &[],
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
