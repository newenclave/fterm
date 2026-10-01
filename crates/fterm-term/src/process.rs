//! Processes: which programs run inside a shell (for "Close the tab? claude is running").

/// Names of all programs that run under the process `pid` (children, their children, ...).
/// Empty when the shell only waits for input.
pub fn running_children(pid: u32) -> Vec<String> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let processes = system.processes();

    // Walk down the tree: children first, then their children.
    let mut names = Vec::new();
    let mut parents = vec![Pid::from_u32(pid)];
    while let Some(parent) = parents.pop() {
        for (child_pid, process) in processes {
            if process.parent() == Some(parent) {
                let name = process.name().to_string_lossy().into_owned();
                // The console host is a helper of Windows, not a program of the user.
                if !name.eq_ignore_ascii_case("conhost.exe") {
                    names.push(name);
                }
                parents.push(*child_pid);
            }
        }
    }
    names
}

/// A program name for people: `PING.EXE` -> `PING`, `claude.exe` -> `claude`.
pub fn display_name(name: &str) -> &str {
    let n = name.len();
    if n > 4 && name[n - 4..].eq_ignore_ascii_case(".exe") {
        &name[..n - 4]
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    use super::*;

    /// A shell that runs one child program for a few seconds.
    fn shell_with_child() -> (Child, &'static str) {
        if cfg!(windows) {
            let child = Command::new("cmd")
                .args(["/c", "ping -n 6 127.0.0.1 >nul"])
                .spawn()
                .unwrap();
            (child, "ping")
        } else {
            let child = Command::new("sh")
                .args(["-c", "sleep 5; true"])
                .spawn()
                .unwrap();
            (child, "sleep")
        }
    }

    #[test]
    fn finds_the_program_that_runs_in_the_shell() {
        let (mut shell, name) = shell_with_child();
        std::thread::sleep(Duration::from_millis(700));
        let children = running_children(shell.id());
        let _ = shell.kill();
        assert!(
            children.iter().any(|c| c.to_lowercase().starts_with(name)),
            "{children:?}"
        );
    }

    #[test]
    fn a_process_without_children_has_none() {
        let mut lonely = if cfg!(windows) {
            Command::new("ping")
                .args(["-n", "6", "127.0.0.1"])
                .stdout(Stdio::null())
                .spawn()
                .unwrap()
        } else {
            Command::new("sleep").arg("5").spawn().unwrap()
        };
        std::thread::sleep(Duration::from_millis(500));
        let children = running_children(lonely.id());
        let _ = lonely.kill();
        assert!(children.is_empty(), "{children:?}");
    }

    #[test]
    fn display_name_drops_exe_in_any_case() {
        assert_eq!(display_name("PING.EXE"), "PING");
        assert_eq!(display_name("claude.exe"), "claude");
        assert_eq!(display_name("node"), "node");
        assert_eq!(display_name(".exe"), ".exe");
    }

    #[test]
    fn unknown_pid_has_no_children() {
        assert!(running_children(u32::MAX - 7).is_empty());
    }
}
