//! The folder and command history of fterm.
//!
//! Two JSON lines files (one record per line), so many fterm windows can add to them at the same time:
//! - `commands.jsonl`: every command that ran (its text, folder, exit code, time), or `{"forget": "..."}`;
//! - `dirs.jsonl`: folder events (`visit`, `pin`, `unpin`, `forget`, and `state` after a compaction).
//!
//! The history lives in memory. New records go to the end of the files. When a file has two times
//! more lines than the limit, it is written again with only the folded state (a temp file, then rename).

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const COMMANDS_FILE: &str = "commands.jsonl";
pub const DIRS_FILE: &str = "dirs.jsonl";
/// Longer commands are not saved (they are often pasted data, not commands).
pub const MAX_COMMAND: usize = 4096;

/// One command that ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRecord {
    pub cmd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// When it started (unix time in ms).
    pub start: u64,
    #[serde(default)]
    pub took_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

/// One line in the list of commands: all runs of the same text together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandEntry {
    pub cmd: String,
    /// The last run.
    pub last: u64,
    pub count: u32,
    pub exit: Option<i32>,
    pub cwd: Option<String>,
}

/// Which commands to show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandFilter {
    /// Only commands that ran in this folder.
    pub cwd: Option<String>,
    /// Only commands whose last run had exit code 0.
    pub only_ok: bool,
}

/// One line in the list of folders.
#[derive(Clone, Debug, PartialEq)]
pub struct DirEntry {
    /// The folder as the shell said it (for example `C:/work/fterm`).
    pub dir: String,
    pub count: u32,
    pub last: u64,
    pub pinned: bool,
    /// Frecency: often and recently used folders have a bigger score.
    pub score: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub commands: usize,
    pub dirs: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            commands: 10_000,
            dirs: 500,
        }
    }
}

/// The history folder: `FTERM_HISTORY_DIR`, else `%APPDATA%\fterm\history` on Windows,
/// else `$XDG_DATA_HOME/fterm/history` or `~/.local/share/fterm/history`.
pub fn default_folder() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FTERM_HISTORY_DIR") {
        return Some(PathBuf::from(dir));
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    Some(base?.join("fterm").join("history"))
}

/// Now, in unix ms (the time in the history files).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Should this command go to the history? Not empty, not too long, and (with `ignore_space`)
/// not started with a space, like `HISTCONTROL=ignorespace` in bash.
pub fn should_save(cmd: &str, ignore_space: bool) -> bool {
    !cmd.trim().is_empty() && cmd.len() <= MAX_COMMAND && !(ignore_space && cmd.starts_with(' '))
}

/// The folder as a map key: `/` for `\`, no `/` at the end, and on Windows no case.
pub fn dir_key(dir: &str) -> String {
    let slashes = dir.replace('\\', "/");
    let trimmed = slashes.trim_end_matches('/');
    let key = if trimmed.is_empty() { "/" } else { trimmed };
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key.to_owned()
    }
}

pub struct History {
    /// The folder with the files. `None` = only in memory (for tests, or when the folder cannot be made).
    folder: Option<PathBuf>,
    limits: Limits,
    commands: Vec<CommandRecord>,
    dirs: HashMap<String, DirState>,
    /// Lines in the files now (to know when to compact).
    command_lines: usize,
    dir_lines: usize,
    /// How much of each file we read (to read only the new lines from other windows).
    command_offset: u64,
    dir_offset: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct DirState {
    dir: String,
    count: u32,
    last: u64,
    pinned: bool,
}

/// A line in `commands.jsonl`.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum CommandLine {
    Run(CommandRecord),
    Forget { forget: String },
}

/// A line in `dirs.jsonl`.
#[derive(Serialize, Deserialize)]
struct DirLine {
    op: DirOp,
    dir: String,
    time: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    count: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DirOp {
    Visit,
    Pin,
    Unpin,
    Forget,
    /// The folded state of one folder (written by a compaction).
    State,
}

impl History {
    /// A history that is not saved.
    pub fn in_memory(limits: Limits) -> Self {
        Self {
            folder: None,
            limits,
            commands: Vec::new(),
            dirs: HashMap::new(),
            command_lines: 0,
            dir_lines: 0,
            command_offset: 0,
            dir_offset: 0,
        }
    }

    /// Loads the files in `folder` (makes the folder when it is not there).
    /// Returns the history and the problems (bad lines), for the log.
    pub fn open(folder: &Path, limits: Limits) -> io::Result<(Self, Vec<String>)> {
        std::fs::create_dir_all(folder)?;
        let mut history = Self::in_memory(limits);
        history.folder = Some(folder.to_owned());
        let problems = history.refresh();
        Ok((history, problems))
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Reads the lines that other fterm windows added since the last read.
    pub fn refresh(&mut self) -> Vec<String> {
        let Some(folder) = self.folder.clone() else {
            return Vec::new();
        };
        let mut problems = Vec::new();
        // A file that got shorter was compacted by another window: read all of it again.
        let commands = folder.join(COMMANDS_FILE);
        if file_len(&commands) < self.command_offset {
            self.commands.clear();
            self.command_offset = 0;
            self.command_lines = 0;
        }
        let dirs = folder.join(DIRS_FILE);
        if file_len(&dirs) < self.dir_offset {
            self.dirs.clear();
            self.dir_offset = 0;
            self.dir_lines = 0;
        }
        let mut new_commands = Vec::new();
        self.command_offset =
            read_lines(
                &commands,
                self.command_offset,
                |n, line| match serde_json::from_str::<CommandLine>(line) {
                    Ok(line) => new_commands.push(line),
                    Err(err) => problems.push(format!("{COMMANDS_FILE}: line {n}: {err}")),
                },
            );
        self.command_lines += new_commands.len();
        for line in new_commands {
            self.apply_command(line);
        }
        let mut new_dirs = Vec::new();
        self.dir_offset = read_lines(
            &dirs,
            self.dir_offset,
            |n, line| match serde_json::from_str::<DirLine>(line) {
                Ok(line) => new_dirs.push(line),
                Err(err) => problems.push(format!("{DIRS_FILE}: line {n}: {err}")),
            },
        );
        self.dir_lines += new_dirs.len();
        for line in new_dirs {
            self.apply_dir(line);
        }
        problems
    }

    pub fn add_command(&mut self, record: CommandRecord) -> io::Result<()> {
        self.write_command(CommandLine::Run(record))
    }

    /// Removes all runs of this command text.
    pub fn forget_command(&mut self, cmd: &str) -> io::Result<()> {
        self.write_command(CommandLine::Forget {
            forget: cmd.to_owned(),
        })
    }

    pub fn visit_dir(&mut self, dir: &str, time: u64) -> io::Result<()> {
        self.write_dir(DirLine::new(DirOp::Visit, dir, time))
    }

    pub fn pin_dir(&mut self, dir: &str, pinned: bool, time: u64) -> io::Result<()> {
        let op = if pinned { DirOp::Pin } else { DirOp::Unpin };
        self.write_dir(DirLine::new(op, dir, time))
    }

    pub fn forget_dir(&mut self, dir: &str, time: u64) -> io::Result<()> {
        self.write_dir(DirLine::new(DirOp::Forget, dir, time))
    }

    /// The commands, newest first, one entry for each text.
    pub fn commands(&self, filter: &CommandFilter) -> Vec<CommandEntry> {
        let cwd = filter.cwd.as_deref().map(dir_key);
        let mut entries: Vec<CommandEntry> = Vec::new();
        let mut index: HashMap<&str, usize> = HashMap::new();
        for run in self.commands.iter().rev() {
            if let Some(cwd) = &cwd
                && run.cwd.as_deref().map(dir_key).as_ref() != Some(cwd)
            {
                continue;
            }
            match index.get(run.cmd.as_str()) {
                Some(&i) => entries[i].count += 1,
                None => {
                    index.insert(&run.cmd, entries.len());
                    entries.push(CommandEntry {
                        cmd: run.cmd.clone(),
                        last: run.start,
                        count: 1,
                        exit: run.exit,
                        cwd: run.cwd.clone(),
                    });
                }
            }
        }
        entries.retain(|e| !filter.only_ok || e.exit == Some(0));
        entries.truncate(self.limits.commands);
        entries
    }

    /// The folders: pinned first, then by frecency (at `now`, unix ms).
    pub fn dirs(&self, now: u64) -> Vec<DirEntry> {
        let mut list: Vec<DirEntry> = self
            .dirs
            .values()
            .map(|d| DirEntry {
                dir: d.dir.clone(),
                count: d.count,
                last: d.last,
                pinned: d.pinned,
                score: frecency(d.count, d.last, now),
            })
            .collect();
        list.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.score.total_cmp(&a.score))
                .then(b.last.cmp(&a.last))
                .then(a.dir.cmp(&b.dir))
        });
        // Pinned folders always stay; the others up to the limit.
        let pinned = list.iter().filter(|d| d.pinned).count();
        list.truncate(pinned + self.limits.dirs);
        list
    }
}

impl History {
    fn apply_command(&mut self, line: CommandLine) {
        match line {
            CommandLine::Run(record) => self.commands.push(record),
            CommandLine::Forget { forget } => self.commands.retain(|r| r.cmd != forget),
        }
    }

    fn apply_dir(&mut self, line: DirLine) {
        let key = dir_key(&line.dir);
        if line.op == DirOp::Forget {
            self.dirs.remove(&key);
            return;
        }
        let state = self.dirs.entry(key).or_insert_with(|| DirState {
            dir: line.dir.clone(),
            count: 0,
            last: 0,
            pinned: false,
        });
        match line.op {
            DirOp::Visit => {
                state.count += 1;
                if line.time >= state.last {
                    state.last = line.time;
                    // Keep the newest spelling of the folder.
                    state.dir = line.dir;
                }
            }
            DirOp::Pin => state.pinned = true,
            DirOp::Unpin => state.pinned = false,
            DirOp::State => {
                state.count = line.count.unwrap_or(0);
                state.last = line.time;
                state.pinned = line.pinned;
            }
            DirOp::Forget => {}
        }
    }

    /// Writes one command line to the file and reads the new lines (ours and other windows').
    /// Without a file, the line goes only to the memory.
    fn write_command(&mut self, line: CommandLine) -> io::Result<()> {
        let Some(folder) = self.folder.clone() else {
            self.apply_command(line);
            self.command_lines += 1;
            if self.command_lines > 2 * self.limits.commands {
                self.keep_newest_commands();
                self.command_lines = self.commands.len();
            }
            return Ok(());
        };
        let text = serde_json::to_string(&line).map_err(io::Error::other)?;
        let result = append_line(&folder.join(COMMANDS_FILE), &text);
        if result.is_err() {
            self.apply_command(line);
        }
        self.refresh();
        if self.command_lines > 2 * self.limits.commands {
            self.compact_commands(&folder)?;
        }
        result
    }

    fn write_dir(&mut self, line: DirLine) -> io::Result<()> {
        let Some(folder) = self.folder.clone() else {
            self.apply_dir(line);
            self.dir_lines += 1;
            if self.dir_lines > 2 * self.limits.dirs {
                self.keep_best_dirs();
                self.dir_lines = self.dirs.len();
            }
            return Ok(());
        };
        let text = serde_json::to_string(&line).map_err(io::Error::other)?;
        let result = append_line(&folder.join(DIRS_FILE), &text);
        if result.is_err() {
            self.apply_dir(line);
        }
        self.refresh();
        if self.dir_lines > 2 * self.limits.dirs {
            self.compact_dirs(&folder)?;
        }
        result
    }

    /// Keeps the runs of the newest `limits.commands` texts.
    fn keep_newest_commands(&mut self) {
        let keep: std::collections::HashSet<String> = self
            .commands(&CommandFilter::default())
            .into_iter()
            .map(|e| e.cmd)
            .collect();
        self.commands.retain(|r| keep.contains(&r.cmd));
        // One text can have many runs: keep at most the limit of runs too.
        let extra = self.commands.len().saturating_sub(self.limits.commands);
        self.commands.drain(..extra);
    }

    /// Keeps the pinned folders and the best `limits.dirs` others.
    fn keep_best_dirs(&mut self) {
        let now = self.dirs.values().map(|d| d.last).max().unwrap_or(0);
        let keep: std::collections::HashSet<String> = self
            .dirs(now)
            .into_iter()
            .filter(|d| !d.pinned)
            .take(self.limits.dirs)
            .map(|d| dir_key(&d.dir))
            .collect();
        self.dirs.retain(|key, d| d.pinned || keep.contains(key));
    }

    fn compact_commands(&mut self, folder: &Path) -> io::Result<()> {
        self.keep_newest_commands();
        let lines: Vec<String> = self
            .commands
            .iter()
            .map(|r| serde_json::to_string(r).map_err(io::Error::other))
            .collect::<io::Result<_>>()?;
        let path = folder.join(COMMANDS_FILE);
        replace_file(&path, &lines)?;
        self.command_lines = lines.len();
        self.command_offset = file_len(&path);
        Ok(())
    }

    fn compact_dirs(&mut self, folder: &Path) -> io::Result<()> {
        self.keep_best_dirs();
        let mut states: Vec<&DirState> = self.dirs.values().collect();
        states.sort_by_key(|d| d.last);
        let lines: Vec<String> = states
            .into_iter()
            .map(|d| {
                serde_json::to_string(&DirLine {
                    op: DirOp::State,
                    dir: d.dir.clone(),
                    time: d.last,
                    count: Some(d.count),
                    pinned: d.pinned,
                })
                .map_err(io::Error::other)
            })
            .collect::<io::Result<_>>()?;
        let path = folder.join(DIRS_FILE);
        replace_file(&path, &lines)?;
        self.dir_lines = lines.len();
        self.dir_offset = file_len(&path);
        Ok(())
    }
}

impl DirLine {
    fn new(op: DirOp, dir: &str, time: u64) -> Self {
        Self {
            op,
            dir: dir.to_owned(),
            time,
            count: None,
            pinned: false,
        }
    }
}

/// Often and recently used folders win (like zoxide): the count times a weight for the age.
fn frecency(count: u32, last: u64, now: u64) -> f64 {
    const HOUR: u64 = 3_600_000;
    let age = now.saturating_sub(last);
    let weight = if age < HOUR {
        4.0
    } else if age < 24 * HOUR {
        2.0
    } else if age < 7 * 24 * HOUR {
        0.5
    } else {
        0.25
    };
    f64::from(count) * weight
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

/// Reads the full lines after `offset` and gives each one (with its number) to `each`.
/// Returns the new offset: the end of the last full line (a line that is still being written waits).
fn read_lines(path: &Path, offset: u64, mut each: impl FnMut(usize, &str)) -> u64 {
    let Ok(mut file) = File::open(path) else {
        return offset;
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return offset;
    }
    let mut reader = BufReader::new(file);
    let mut done = offset;
    let mut buf = Vec::new();
    let mut n = 0;
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(len) => {
                if buf.last() != Some(&b'\n') {
                    break;
                }
                done += len as u64;
                n += 1;
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim();
                if !line.is_empty() {
                    each(n, line);
                }
            }
        }
    }
    done
}

/// One `write` call for the line and its end, so lines of many windows do not mix.
fn append_line(path: &Path, line: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(format!("{line}\n").as_bytes())
}

/// Writes `lines` to a temp file and puts it in the place of `path`.
fn replace_file(path: &Path, lines: &[String]) -> io::Result<()> {
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 3_600_000;
    const DAY: u64 = 24 * HOUR;

    /// A fresh folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fterm-history-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn run(cmd: &str, cwd: &str, exit: i32, start: u64) -> CommandRecord {
        CommandRecord {
            cmd: cmd.into(),
            cwd: Some(cwd.into()),
            exit: Some(exit),
            start,
            took_ms: 10,
            shell: Some("pwsh".into()),
        }
    }

    fn texts(list: &[CommandEntry]) -> Vec<&str> {
        list.iter().map(|e| e.cmd.as_str()).collect()
    }

    fn dir_names(list: &[DirEntry]) -> Vec<&str> {
        list.iter().map(|e| e.dir.as_str()).collect()
    }

    #[test]
    fn what_to_save() {
        assert!(should_save("cargo test", true));
        assert!(!should_save("", true));
        assert!(!should_save("   ", false));
        assert!(
            !should_save(" secret --token x", true),
            "a space at the start = do not save"
        );
        assert!(should_save(" ls", false));
        assert!(!should_save(&"x".repeat(MAX_COMMAND + 1), true));
    }

    #[test]
    fn folder_keys() {
        assert_eq!(dir_key("C:\\work\\fterm\\"), dir_key("C:/work/fterm"));
        assert_eq!(dir_key("/home/me/"), dir_key("/home/me"));
        assert_eq!(dir_key("/"), "/", "the root stays");
        if cfg!(windows) {
            assert_eq!(dir_key("C:/Work"), dir_key("c:/work"));
        }
    }

    #[test]
    fn commands_are_newest_first_and_the_same_text_is_one_entry() {
        let mut h = History::in_memory(Limits::default());
        h.add_command(run("cargo build", "C:/a", 1, 100)).unwrap();
        h.add_command(run("git status", "C:/a", 0, 200)).unwrap();
        h.add_command(run("cargo build", "C:/b", 0, 300)).unwrap();
        let list = h.commands(&CommandFilter::default());
        assert_eq!(texts(&list), ["cargo build", "git status"]);
        let build = &list[0];
        assert_eq!((build.count, build.last, build.exit), (2, 300, Some(0)));
        assert_eq!(
            build.cwd.as_deref(),
            Some("C:/b"),
            "the folder of the last run"
        );
    }

    #[test]
    fn command_filters() {
        let mut h = History::in_memory(Limits::default());
        h.add_command(run("make", "C:/a", 2, 100)).unwrap();
        h.add_command(run("ls", "C:/a", 0, 200)).unwrap();
        h.add_command(run("dir", "C:/b", 0, 300)).unwrap();
        let only_a = CommandFilter {
            cwd: Some("c:\\a\\".into()),
            only_ok: false,
        };
        let a = h.commands(&only_a);
        if cfg!(windows) {
            assert_eq!(texts(&a), ["ls", "make"], "the folder is compared as a key");
        }
        let ok = h.commands(&CommandFilter {
            cwd: None,
            only_ok: true,
        });
        assert_eq!(texts(&ok), ["dir", "ls"]);
    }

    #[test]
    fn forget_a_command() {
        let mut h = History::in_memory(Limits::default());
        h.add_command(run("rm -rf build", "C:/a", 0, 100)).unwrap();
        h.add_command(run("ls", "C:/a", 0, 200)).unwrap();
        h.forget_command("rm -rf build").unwrap();
        assert_eq!(texts(&h.commands(&CommandFilter::default())), ["ls"]);
    }

    #[test]
    fn the_history_is_saved_and_loaded_again() {
        let dir = folder("save");
        {
            let (mut h, problems) = History::open(&dir, Limits::default()).unwrap();
            assert!(problems.is_empty());
            h.add_command(run("cargo test", "C:/a", 0, 100)).unwrap();
            h.add_command(run("git push", "C:/a", 1, 200)).unwrap();
            h.forget_command("git push").unwrap();
            h.visit_dir("C:/a", 100).unwrap();
            h.pin_dir("C:/b", true, 150).unwrap();
        }
        let (h, problems) = History::open(&dir, Limits::default()).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            texts(&h.commands(&CommandFilter::default())),
            ["cargo test"]
        );
        let dirs = h.dirs(200);
        assert_eq!(dir_names(&dirs), ["C:/b", "C:/a"], "pinned first");
        assert!(dirs[0].pinned);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_lines_are_skipped() {
        let dir = folder("bad");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(COMMANDS_FILE),
            "{\"cmd\":\"ls\",\"start\":1}\nnot json\n{\"cmd\":\"pwd\",\"start\":2}\n",
        )
        .unwrap();
        let (h, problems) = History::open(&dir, Limits::default()).unwrap();
        assert_eq!(texts(&h.commands(&CommandFilter::default())), ["pwd", "ls"]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_line_that_is_not_ended_waits() {
        use std::io::Write as _;
        let dir = folder("half");
        let (mut h, _) = History::open(&dir, Limits::default()).unwrap();
        let path = dir.join(COMMANDS_FILE);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"{\"cmd\":\"ls\",\"start\":1}\n{\"cmd\":\"pw")
            .unwrap();
        assert!(h.refresh().is_empty(), "no error for the half line");
        assert_eq!(texts(&h.commands(&CommandFilter::default())), ["ls"]);
        file.write_all(b"d\",\"start\":2}\n").unwrap();
        assert!(h.refresh().is_empty());
        assert_eq!(texts(&h.commands(&CommandFilter::default())), ["pwd", "ls"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_windows_write_the_same_files() {
        let dir = folder("two");
        let (mut a, _) = History::open(&dir, Limits::default()).unwrap();
        let (mut b, _) = History::open(&dir, Limits::default()).unwrap();
        a.add_command(run("from a", "C:/a", 0, 100)).unwrap();
        b.add_command(run("from b", "C:/a", 0, 200)).unwrap();
        b.visit_dir("C:/b", 200).unwrap();
        // `a` sees the lines of `b` after a refresh.
        assert!(a.refresh().is_empty());
        assert_eq!(
            texts(&a.commands(&CommandFilter::default())),
            ["from b", "from a"]
        );
        assert_eq!(dir_names(&a.dirs(300)), ["C:/b"]);
        // Its own lines are not read two times.
        assert!(a.refresh().is_empty());
        assert_eq!(a.commands(&CommandFilter::default()).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compaction_keeps_the_newest() {
        let dir = folder("compact");
        let limits = Limits {
            commands: 5,
            dirs: 3,
        };
        let (mut h, _) = History::open(&dir, limits).unwrap();
        for i in 0..12 {
            h.add_command(run(&format!("cmd {i}"), "C:/a", 0, 100 + i))
                .unwrap();
            h.visit_dir(&format!("C:/d{i}"), 100 + i).unwrap();
        }
        h.pin_dir("C:/d0", true, 200).unwrap();
        let lines = |name: &str| {
            std::fs::read_to_string(dir.join(name))
                .unwrap()
                .lines()
                .count()
        };
        assert!(lines(COMMANDS_FILE) <= 10, "the file is compacted");
        assert!(lines(DIRS_FILE) <= 6 + 1);
        let (h2, problems) = History::open(&dir, limits).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let list = h2.commands(&CommandFilter::default());
        assert_eq!(list.len(), 5);
        assert_eq!(list[0].cmd, "cmd 11");
        let dirs = h2.dirs(300);
        assert!(dirs.len() <= 4, "the limit, and the pinned one");
        assert_eq!(dirs[0].dir, "C:/d0", "a pinned folder is never dropped");
        assert_eq!(dirs[1].dir, "C:/d11");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frecency_order() {
        let mut h = History::in_memory(Limits::default());
        let now = 30 * DAY;
        // Often, but long ago.
        for i in 0..10 {
            h.visit_dir("C:/old", 10 * DAY + i).unwrap();
        }
        // Three times, today: 3 x 2 = 6.
        h.visit_dir("C:/today", now - HOUR * 3).unwrap();
        h.visit_dir("C:/today", now - HOUR * 2).unwrap();
        h.visit_dir("C:/today", now - HOUR).unwrap();
        // One time, a minute ago: 1 x 4 = 4. The old one: 10 x 0.25 = 2.5.
        h.visit_dir("C:/now", now - 60_000).unwrap();
        assert_eq!(dir_names(&h.dirs(now)), ["C:/today", "C:/now", "C:/old"]);
        // The same folder with other slashes is the same folder.
        h.visit_dir("C:\\now\\", now).unwrap();
        assert_eq!(h.dirs(now).len(), 3);
    }

    #[test]
    fn pin_unpin_and_forget_folders() {
        let mut h = History::in_memory(Limits::default());
        h.visit_dir("C:/a", 100).unwrap();
        h.visit_dir("C:/b", 100).unwrap();
        h.pin_dir("C:/b", true, 100).unwrap();
        assert!(h.dirs(200)[0].pinned);
        h.pin_dir("C:/b", false, 100).unwrap();
        assert!(h.dirs(200).iter().all(|d| !d.pinned));
        h.forget_dir("C:/a", 100).unwrap();
        assert_eq!(dir_names(&h.dirs(200)), ["C:/b"]);
        // A visit after forget brings it back, with a new count.
        h.visit_dir("C:/a", 300).unwrap();
        let a = h.dirs(400).into_iter().find(|d| d.dir == "C:/a").unwrap();
        assert_eq!(a.count, 1);
    }
}
