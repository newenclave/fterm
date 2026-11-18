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

/// What a walk up the process tree needs to know about one process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcInfo {
    pub parent: Option<u32>,
    /// When it started (0 = not known): only compared with other start times of the same machine (see `start_of`).
    pub start: u64,
    /// The file name of its program (`claude.exe`, `bash`).
    pub name: String,
}

/// One process of a chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub pid: u32,
    pub start: u64,
    pub name: String,
}

/// The chain of the process `pid` and its parents, nearest first, read with `info(pid, fresh)`.
///
/// `peer_start` is the start time of `pid` when it connected: a fresh look must give the same (else the pid is
/// another process now, and the chain is empty). The walk stops (fail closed):
/// - at a process that is not known, or whose start time is not known;
/// - at a parent that started after its child (a reused pid; a fresh look is tried first);
/// - before a process named `fterm` (another window: its panes are not ours);
/// - before `wsl.exe` or `wslhost.exe`: a program in WSL picks which `wsl.exe` starts a Windows program
///   (`WSL_INTEROP`), also the one of another pane;
/// - after an `ftermctl` above the direct parent of `pid` (`ftermctl mux serve` runs panes; `ftermctl steps` is the
///   program of a pane; `ftermctl mcp` may start the peer);
/// - after `max` processes.
pub fn chain_in(
    pid: u32,
    peer_start: u64,
    max: usize,
    mut info: impl FnMut(u32, bool) -> Option<ProcInfo>,
) -> Vec<Link> {
    let mut chain: Vec<Link> = Vec::new();
    // The peer: a fresh look, and the same start time as when it connected.
    let Some(peer) = info(pid, true).filter(|p| p.start != 0 && p.start == peer_start) else {
        return chain;
    };
    let mut next = peer.parent;
    let mut child_start = peer.start;
    chain.push(Link {
        pid,
        start: peer.start,
        name: peer.name,
    });
    while chain.len() < max {
        let Some(parent) = next else {
            break;
        };
        if chain.iter().any(|l| l.pid == parent) {
            break;
        }
        let fits = |p: &ProcInfo| p.start != 0 && p.start <= child_start;
        let found = match info(parent, false) {
            Some(p) if fits(&p) => Some(p),
            // Not in the snapshot yet, or the snapshot is old: a fresh look.
            _ => info(parent, true).filter(fits),
        };
        let Some(p) = found else {
            break;
        };
        if ["fterm", "wsl", "wslhost"]
            .iter()
            .any(|program| is_program(&p.name, program))
        {
            break;
        }
        let last = is_program(&p.name, "ftermctl") && chain.len() >= 2;
        next = p.parent;
        child_start = p.start;
        chain.push(Link {
            pid: parent,
            start: p.start,
            name: p.name,
        });
        if last {
            break;
        }
    }
    chain
}

/// The processes as the OS shows them, for `chain_in`: one snapshot, taken again when it is older than a second,
/// and a fresh look at one process when the walk asks for it.
pub struct Processes {
    system: sysinfo::System,
    taken: Option<std::time::Instant>,
}

impl Default for Processes {
    fn default() -> Self {
        Self::new()
    }
}

impl Processes {
    pub fn new() -> Self {
        Self {
            system: sysinfo::System::new(),
            taken: None,
        }
    }

    /// One process as it is now.
    pub fn fresh(&mut self, pid: u32) -> Option<ProcInfo> {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate};
        let id = Pid::from_u32(pid);
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[id]),
            true,
            ProcessRefreshKind::nothing(),
        );
        self.cached(pid)
    }

    /// One process as the snapshot shows it.
    fn cached(&self, pid: u32) -> Option<ProcInfo> {
        let process = self.system.process(sysinfo::Pid::from_u32(pid))?;
        Some(ProcInfo {
            parent: process.parent().map(|p| p.as_u32()),
            start: start_of(pid, process),
            name: process.name().to_string_lossy().into_owned(),
        })
    }

    /// The chain of `pid` and its parents (see `chain_in`).
    pub fn chain(&mut self, pid: u32, peer_start: u64, max: usize) -> Vec<Link> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
        let now = std::time::Instant::now();
        if self
            .taken
            .is_none_or(|t| now.duration_since(t) > std::time::Duration::from_secs(1))
        {
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing(),
            );
            self.taken = Some(now);
        }
        chain_in(pid, peer_start, max, |p, fresh| {
            if fresh { self.fresh(p) } else { self.cached(p) }
        })
    }
}

/// The start time of a process in a line of `/proc/<pid>/stat` (field 22: clock ticks since boot). It does not move
/// when the clock of the system is set, as a time in seconds since 1970 does on Linux.
pub fn stat_start_ticks(stat: &str) -> Option<u64> {
    // The name is in parens and may have spaces and parens: the fields start after the last ')', with field 3.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(22 - 3)?.parse().ok()
}

/// The start time of a process for the walks: clock ticks since boot on Linux (read from `/proc`), seconds since
/// 1970 elsewhere (the creation time that Windows and macOS keep). 0 = not known.
fn start_of(pid: u32, process: &sysinfo::Process) -> u64 {
    #[cfg(target_os = "linux")]
    {
        let _ = process;
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| stat_start_ticks(&stat))
            .unwrap_or(0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        process.start_time()
    }
}

/// The file name `name` is the program `program` (with or without `.exe`, in any case).
fn is_program(name: &str, program: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == program || name.strip_suffix(".exe") == Some(program)
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

/// True for shells (`powershell`, `cmd`, `bash`, ...): a pane with only its shell runs nothing.
/// Takes a program name or a path, with or without `.exe`.
pub fn is_shell(program: &str) -> bool {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let name = display_name(file).to_ascii_lowercase();
    matches!(
        name.as_str(),
        "powershell"
            | "pwsh"
            | "cmd"
            | "bash"
            | "zsh"
            | "sh"
            | "fish"
            | "nu"
            | "wsl"
            | "dash"
            | "ksh"
    )
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
    fn shells_and_programs() {
        for shell in [
            "powershell.exe",
            "pwsh",
            "cmd.exe",
            "C:\\Windows\\System32\\CMD.EXE",
            "bash",
            "/bin/zsh",
            "sh",
            "fish",
            "nu",
            "wsl.exe",
        ] {
            assert!(is_shell(shell), "{shell}");
        }
        for program in [
            "claude",
            "claude.cmd",
            "node.exe",
            "vim",
            "ollama",
            "opencode",
        ] {
            assert!(!is_shell(program), "{program}");
        }
    }

    #[test]
    fn display_name_drops_exe_in_any_case() {
        assert_eq!(display_name("PING.EXE"), "PING");
        assert_eq!(display_name("claude.exe"), "claude");
        assert_eq!(display_name("node"), "node");
        assert_eq!(display_name(".exe"), ".exe");
    }

    /// A fake process table: pid -> (parent, start, name), and the fresh values that a second look gives.
    struct Table {
        cached: Vec<(u32, Option<u32>, u64, &'static str)>,
        fresh: Vec<(u32, Option<u32>, u64, &'static str)>,
    }

    impl Table {
        fn new(cached: &[(u32, Option<u32>, u64, &'static str)]) -> Self {
            Self {
                cached: cached.to_vec(),
                fresh: cached.to_vec(),
            }
        }

        fn chain(&self, pid: u32, peer_start: u64, max: usize) -> Vec<u32> {
            chain_in(pid, peer_start, max, |p, fresh| {
                let list = if fresh { &self.fresh } else { &self.cached };
                list.iter()
                    .find(|(id, ..)| *id == p)
                    .map(|&(_, parent, start, name)| ProcInfo {
                        parent,
                        start,
                        name: name.to_owned(),
                    })
            })
            .into_iter()
            .map(|l| l.pid)
            .collect()
        }
    }

    #[test]
    fn the_chain_goes_up_to_the_top() {
        let t = Table::new(&[
            (10, Some(7), 500, "ftermctl"),
            (7, Some(3), 400, "claude"),
            (3, Some(1), 300, "pwsh"),
            (1, None, 100, "init"),
        ]);
        assert_eq!(t.chain(10, 500, 32), [10, 7, 3, 1]);
    }

    #[test]
    fn the_peer_must_still_be_the_process_that_connected() {
        let t = Table::new(&[(10, Some(7), 500, "ftermctl"), (7, None, 400, "claude")]);
        assert_eq!(
            t.chain(10, 499, 32),
            Vec::<u32>::new(),
            "another process has the pid now"
        );
    }

    #[test]
    fn a_parent_younger_than_its_child_stops_the_chain() {
        // On Windows the parent pid of a process stays when its parent ends: the pid may belong to a new process.
        let t = Table::new(&[
            (10, Some(7), 500, "ftermctl"),
            (7, Some(3), 600, "pwsh"),
            (3, None, 100, "x"),
        ]);
        assert_eq!(t.chain(10, 500, 32), [10]);
    }

    #[test]
    fn a_parent_of_the_same_second_is_fine() {
        // `sh -c ftermctl` starts both in the same second (times are in seconds).
        let t = Table::new(&[(10, Some(7), 500, "ftermctl"), (7, None, 500, "sh")]);
        assert_eq!(t.chain(10, 500, 32), [10, 7]);
    }

    #[test]
    fn an_unknown_start_time_stops_the_chain() {
        let t = Table::new(&[
            (10, Some(7), 500, "ftermctl"),
            (7, Some(3), 0, "pwsh"),
            (3, None, 100, "x"),
        ]);
        assert_eq!(t.chain(10, 500, 32), [10]);
    }

    #[test]
    fn a_missing_parent_gets_a_fresh_look() {
        // A shell that started less than a second ago is not in the old snapshot yet.
        let mut t = Table::new(&[(10, Some(7), 500, "ftermctl"), (3, None, 100, "pwsh")]);
        t.fresh.push((7, Some(3), 499, "sh"));
        assert_eq!(t.chain(10, 500, 32), [10, 7, 3]);
        let t = Table::new(&[(10, Some(7), 500, "ftermctl")]);
        assert_eq!(
            t.chain(10, 500, 32),
            [10],
            "still missing: the chain stops there"
        );
    }

    #[test]
    fn a_stale_parent_gets_a_fresh_look() {
        let mut t = Table::new(&[
            (10, Some(7), 500, "ftermctl"),
            (7, Some(3), 900, "old"),
            (3, None, 1, "x"),
        ]);
        t.fresh = vec![
            (10, Some(7), 500, "ftermctl"),
            (7, Some(3), 400, "sh"),
            (3, None, 1, "x"),
        ];
        assert_eq!(t.chain(10, 500, 32), [10, 7, 3]);
    }

    #[test]
    fn the_chain_stops_before_another_fterm() {
        // An fterm started from a pane, or a mux server: its panes are not the panes of this window.
        let t = Table::new(&[
            (10, Some(7), 500, "ftermctl"),
            (7, Some(5), 400, "pwsh"),
            (5, Some(3), 300, "fterm.exe"),
            (3, None, 200, "pwsh"),
        ]);
        assert_eq!(t.chain(10, 500, 32), [10, 7]);
        let t = Table::new(&[
            (10, Some(5), 500, "bash"),
            (5, Some(3), 300, "fterm"),
            (3, None, 200, "bash"),
        ]);
        assert_eq!(t.chain(10, 500, 32), [10]);
    }

    #[test]
    fn an_ftermctl_above_the_parent_is_the_last_link() {
        // `ftermctl mux serve` runs panes too: not past it. It is in the chain: `ftermctl steps` is the program of a
        // pane (fterm.run, dashboard steps), and a program in such a pane is in that pane.
        let t = Table::new(&[
            (10, Some(7), 500, "python"),
            (7, Some(5), 400, "bash"),
            (5, Some(3), 300, "ftermctl"),
            (3, None, 200, "bash"),
        ]);
        assert_eq!(t.chain(10, 500, 32), [10, 7, 5]);
        // `ftermctl mcp` starts an ftermctl for a long call: that one is fine.
        let t = Table::new(&[
            (10, Some(7), 500, "ftermctl.exe"),
            (7, Some(5), 400, "ftermctl.exe"),
            (5, Some(3), 300, "claude.exe"),
            (3, None, 200, "pwsh.exe"),
        ]);
        assert_eq!(t.chain(10, 500, 32), [10, 7, 5, 3]);
    }

    #[test]
    fn the_chain_stops_before_wsl() {
        // A program in WSL picks the wsl.exe that starts a Windows program (WSL_INTEROP): it can be the wsl.exe
        // of another pane. So a chain through WSL tells no pane.
        let t = Table::new(&[
            (10, Some(8), 500, "ftermctl.exe"),
            (8, Some(7), 500, "wsl.exe"),
            (7, Some(3), 400, "wsl.exe"),
            (3, None, 200, "pwsh.exe"),
        ]);
        assert_eq!(t.chain(10, 500, 32), [10]);
        let t = Table::new(&[(10, Some(8), 500, "x.exe"), (8, None, 400, "wslhost.exe")]);
        assert_eq!(t.chain(10, 500, 32), [10]);
    }

    #[test]
    fn the_start_ticks_of_a_stat_line() {
        let line = "4321 (a b) c) S 1 4321 4321 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 98765 1234567 100\n";
        assert_eq!(
            stat_start_ticks(line),
            Some(98765),
            "field 22, also with ')' in the name"
        );
        assert_eq!(stat_start_ticks("garbage"), None);
        assert_eq!(stat_start_ticks("1 (x) S 1 2"), None, "too short");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn on_linux_the_start_time_is_the_ticks_since_boot() {
        let me = std::process::id();
        let stat = std::fs::read_to_string(format!("/proc/{me}/stat")).unwrap();
        assert_eq!(
            Processes::new().fresh(me).map(|p| p.start),
            stat_start_ticks(&stat),
            "seconds since 1970 move when the clock is set"
        );
    }

    #[test]
    fn a_loop_ends_and_the_chain_has_a_limit() {
        let t = Table::new(&[(10, Some(7), 500, "a"), (7, Some(10), 400, "b")]);
        assert_eq!(t.chain(10, 500, 32), [10, 7], "a loop");
        let t = Table::new(&[
            (4, Some(3), 9, "a"),
            (3, Some(2), 8, "b"),
            (2, Some(1), 7, "c"),
            (1, None, 6, "d"),
        ]);
        assert_eq!(t.chain(4, 9, 2), [4, 3]);
    }

    #[test]
    fn the_chain_of_a_child_has_us_in_it() {
        let (mut shell, _) = shell_with_child();
        std::thread::sleep(Duration::from_millis(300));
        let mut processes = Processes::new();
        let start = processes
            .fresh(shell.id())
            .expect("the child is there")
            .start;
        let chain = processes.chain(shell.id(), start, 32);
        let _ = shell.kill();
        assert_eq!(chain.first().map(|l| l.pid), Some(shell.id()), "{chain:?}");
        assert!(
            chain.iter().any(|l| l.pid == std::process::id()),
            "this test process is a parent: {chain:?}"
        );
        assert!(
            Processes::new().chain(shell.id(), start + 1, 32).is_empty(),
            "another start time"
        );
    }

    #[test]
    fn a_start_time_is_the_same_on_every_look() {
        let me = std::process::id();
        let first = Processes::new().fresh(me).expect("this process").start;
        std::thread::sleep(Duration::from_millis(1100));
        let second = Processes::new().fresh(me).expect("this process").start;
        assert_ne!(first, 0);
        assert_eq!(first, second);
    }

    #[test]
    fn unknown_pid_has_no_children() {
        assert!(running_children(u32::MAX - 7).is_empty());
    }
}
