//! The history popups: commands (Alt+F8) and folders (Alt+F12), in the style of the command palette.
//! Pure: the rows, the filter, the selection, and the bytes that go to the shell.

use fterm_history::{CommandEntry, DirEntry};

use crate::palette::{VISIBLE_ROWS, score};
use crate::panels::short_ago;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupKind {
    Commands,
    Dirs,
    /// Saved sessions (Phase 9b).
    Sessions,
    /// The themes (built-in ones and files).
    Themes,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PopupRow {
    /// The command or the folder.
    pub text: String,
    /// The second column: the time, the exit code, a pin.
    pub hint: String,
    /// A failed command or a folder that is not there any more (red / grey).
    pub bad: bool,
    /// What the row stands for when it is not `text` (the file of a session).
    pub key: String,
}

pub struct HistoryPopup {
    pub kind: PopupKind,
    rows: Vec<PopupRow>,
    query: String,
    /// Rows that match the query, best first.
    shown: Vec<usize>,
    selected: usize,
    top: usize,
    /// Commands: only this folder / only exit code 0.
    pub only_here: bool,
    pub only_ok: bool,
    /// The folder of the pane (for "only this folder").
    pub here: Option<String>,
    /// The text that was typed at the prompt when the popup opened (Enter replaces it).
    pub typed: String,
}

impl HistoryPopup {
    /// A popup with these rows. The query starts with the typed text, so `git` + Alt+F8 shows git commands.
    pub fn new(kind: PopupKind, rows: Vec<PopupRow>, typed: String, here: Option<String>) -> Self {
        let mut popup = Self {
            kind,
            rows,
            query: typed.trim().to_owned(),
            shown: Vec::new(),
            selected: 0,
            top: 0,
            only_here: false,
            only_ok: false,
            here,
            typed,
        };
        popup.refilter();
        popup
    }

    /// New rows (after a filter changed), with the same query.
    pub fn set_rows(&mut self, rows: Vec<PopupRow>) {
        self.rows = rows;
        self.refilter();
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn type_text(&mut self, text: &str) {
        self.query.extend(text.chars().filter(|c| !c.is_control()));
        self.refilter();
    }

    pub fn backspace(&mut self) {
        if self.query.pop().is_some() {
            self.refilter();
        }
    }

    pub fn move_selection(&mut self, step: i32) {
        if self.shown.is_empty() {
            return;
        }
        let last = self.shown.len() - 1;
        self.selected = (self.selected as i64 + i64::from(step)).clamp(0, last as i64) as usize;
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + VISIBLE_ROWS {
            self.top = self.selected + 1 - VISIBLE_ROWS;
        }
    }

    pub fn selected(&self) -> Option<&PopupRow> {
        self.shown.get(self.selected).map(|&i| &self.rows[i])
    }

    /// The rows to draw now: (row, is it selected).
    pub fn visible(&self) -> Vec<(&PopupRow, bool)> {
        self.shown
            .iter()
            .enumerate()
            .skip(self.top)
            .take(VISIBLE_ROWS)
            .map(|(n, &i)| (&self.rows[i], n == self.selected))
            .collect()
    }

    /// The text on the right of the input line: what the popup shows.
    pub fn title(&self) -> String {
        match self.kind {
            PopupKind::Dirs => "Folders".to_owned(),
            PopupKind::Sessions => "Sessions".to_owned(),
            PopupKind::Themes => "Themes".to_owned(),
            PopupKind::Commands => {
                let mut title = "Commands".to_owned();
                if self.only_here {
                    title.push_str(" · this folder");
                }
                if self.only_ok {
                    title.push_str(" · exit 0");
                }
                title
            }
        }
    }

    /// The key hints under the list.
    pub fn footer(&self) -> &'static str {
        match self.kind {
            PopupKind::Commands => {
                "Enter put · Shift+Enter run · Ctrl+D folder · Ctrl+G exit 0 · Ctrl+C copy · Del forget"
            }
            PopupKind::Dirs => {
                "Enter cd · Shift+Enter tab · Ctrl+Enter split · Ctrl+P pin · Del forget"
            }
            PopupKind::Sessions => {
                "Enter restore · Del forget · Save session as… is in the palette"
            }
            PopupKind::Themes => "Enter use · the themes folder is next to fterm.lua",
        }
    }
}

/// The rows for the commands popup.
pub fn command_rows(entries: &[CommandEntry], now: u64) -> Vec<PopupRow> {
    entries
        .iter()
        .map(|e| {
            let ago = ago(now, e.last);
            let bad = e.exit.is_some_and(|code| code != 0);
            let hint = match e.exit {
                Some(code) if code != 0 => format!("exit {code} · {ago}"),
                _ => ago,
            };
            PopupRow {
                text: e.cmd.clone(),
                hint,
                bad,
                ..PopupRow::default()
            }
        })
        .collect()
}

/// The rows for the folders popup. `exists` says if a folder is still there.
pub fn dir_rows(entries: &[DirEntry], now: u64, exists: impl Fn(&str) -> bool) -> Vec<PopupRow> {
    entries
        .iter()
        .map(|e| {
            let there = exists(&e.dir);
            let hint = if !there {
                "not found".to_owned()
            } else if e.pinned {
                format!("★ {}", ago(now, e.last))
            } else {
                ago(now, e.last)
            };
            PopupRow {
                text: shown_dir(&e.dir),
                hint,
                bad: !there,
                ..PopupRow::default()
            }
        })
        .collect()
}

/// Where a pane lives: Windows, or a WSL distro (its folders are Linux paths). The history popups
/// and hints show only the folders and commands of the pane's world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum World {
    Windows,
    Wsl(String),
}

impl World {
    /// The folder belongs to this world.
    pub fn has(&self, dir: &str) -> bool {
        dir.starts_with('/') == matches!(self, World::Wsl(_))
    }

    /// A command (with the folder where it ran) belongs to this world.
    pub fn has_command(&self, cwd: Option<&str>) -> bool {
        cwd.is_none_or(|dir| self.has(dir))
    }

    /// The path that Windows can check (`\\wsl.localhost\<distro>\...` for a Linux folder).
    pub fn host_path(&self, dir: &str) -> String {
        match self {
            World::Wsl(distro) if dir.starts_with('/') => {
                // `/mnt/c/...` is the C: drive.
                if let Some(rest) = dir.strip_prefix("/mnt/") {
                    let (drive, tail) = rest.split_once('/').unwrap_or((rest, ""));
                    if drive.len() == 1 && drive.chars().all(|c| c.is_ascii_alphabetic()) {
                        return format!(
                            r"{}:\{}",
                            drive.to_ascii_uppercase(),
                            tail.replace('/', "\\")
                        );
                    }
                }
                format!(r"\\wsl.localhost\{distro}{}", dir.replace('/', "\\"))
            }
            _ => dir.to_owned(),
        }
    }
}

/// The bytes that put `text` into the prompt in place of the typed text:
/// End (go to the end of the line), Backspace for each typed char, then the text, and Enter with `run`.
pub fn replace_input(typed: &str, text: &str, run: bool) -> Vec<u8> {
    let mut out = b"\x1b[F".to_vec();
    out.extend(std::iter::repeat_n(0x7f, typed.chars().count()));
    out.extend_from_slice(text.as_bytes());
    if run {
        out.push(b'\r');
    }
    out
}

/// The command that goes to `dir` in this shell (by the program name).
pub fn cd_command(program: &str, dir: &str) -> String {
    let file = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    let name = file.strip_suffix(".exe").unwrap_or(&file);
    match name {
        "powershell" | "pwsh" => format!("Set-Location -LiteralPath '{}'", dir.replace('\'', "''")),
        "cmd" => format!("cd /d \"{}\"", dir.replace('/', "\\")),
        _ => format!("cd -- '{}'", dir.replace('\'', "'\\''")),
    }
}

fn ago(now: u64, then: u64) -> String {
    short_ago(std::time::Duration::from_millis(now.saturating_sub(then)))
}

/// A folder for people: with `\` on Windows.
pub fn shown_dir(dir: &str) -> String {
    // A Linux folder (WSL) keeps its `/`.
    if cfg!(windows) && !dir.starts_with('/') {
        dir.replace('/', "\\")
    } else {
        dir.to_owned()
    }
}

impl HistoryPopup {
    fn refilter(&mut self) {
        let mut scored: Vec<(i32, usize)> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| score(&self.query, &row.text).map(|s| (s, i)))
            .collect();
        // Stable: the history order stays for the same score.
        scored.sort_by_key(|(s, _)| -s);
        self.shown = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
        self.top = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    #[test]
    fn each_folder_has_its_world() {
        let windows = World::Windows;
        let ubuntu = World::Wsl("Ubuntu".into());
        assert!(windows.has("C:/work") && windows.has(r"\\wsl$\Ubuntu\home"));
        assert!(!windows.has("/home/me"));
        assert!(ubuntu.has("/home/me") && !ubuntu.has("C:/work"));
        // A command without a folder is for every world.
        assert!(windows.has_command(None) && ubuntu.has_command(None));
        assert!(ubuntu.has_command(Some("/tmp")) && !ubuntu.has_command(Some("C:/x")));
    }

    #[test]
    fn a_linux_folder_is_checked_through_windows() {
        let ubuntu = World::Wsl("Ubuntu".into());
        assert_eq!(
            ubuntu.host_path("/home/me"),
            r"\\wsl.localhost\Ubuntu\home\me"
        );
        assert_eq!(World::Windows.host_path("C:/work"), "C:/work");
        // The Windows drives in WSL are not in \\wsl.localhost: check them on the drive.
        assert_eq!(ubuntu.host_path("/mnt/c/work/fterm"), r"C:\work\fterm");
        assert_eq!(ubuntu.host_path("/mnt/d"), r"D:\");
        assert_eq!(
            ubuntu.host_path("/mnt/wsl"),
            r"\\wsl.localhost\Ubuntu\mnt\wsl"
        );
    }

    #[test]
    fn a_linux_folder_keeps_its_slashes() {
        assert_eq!(shown_dir("/home/me/src"), "/home/me/src");
    }

    fn row(text: &str) -> PopupRow {
        PopupRow {
            text: text.into(),
            hint: String::new(),
            bad: false,
            ..PopupRow::default()
        }
    }

    fn popup(rows: &[&str], typed: &str) -> HistoryPopup {
        HistoryPopup::new(
            PopupKind::Commands,
            rows.iter().map(|r| row(r)).collect(),
            typed.into(),
            None,
        )
    }

    fn shown(p: &HistoryPopup) -> Vec<&str> {
        p.visible()
            .into_iter()
            .map(|(r, _)| r.text.as_str())
            .collect()
    }

    #[test]
    fn the_query_starts_with_the_typed_text() {
        let p = popup(&["cargo test", "git status", "git push"], "git");
        assert_eq!(p.query(), "git");
        assert_eq!(
            shown(&p),
            ["git status", "git push"],
            "the history order for the same score"
        );
        assert_eq!(p.selected().unwrap().text, "git status");
    }

    #[test]
    fn no_query_keeps_the_history_order() {
        let p = popup(&["c", "b", "a"], "");
        assert_eq!(shown(&p), ["c", "b", "a"]);
    }

    #[test]
    fn typing_and_backspace_filter_again() {
        let mut p = popup(&["cargo build", "cargo test", "ls"], "");
        p.type_text("ct");
        assert_eq!(shown(&p), ["cargo test"]);
        p.backspace();
        p.backspace();
        assert_eq!(shown(&p).len(), 3);
        p.type_text("zzz");
        assert!(p.selected().is_none());
    }

    #[test]
    fn the_selection_stays_inside_and_on_the_screen() {
        let names: Vec<String> = (0..30).map(|i| format!("cmd {i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut p = popup(&refs, "");
        p.move_selection(-1);
        assert_eq!(p.selected().unwrap().text, "cmd 0");
        p.move_selection(20);
        assert_eq!(p.selected().unwrap().text, "cmd 20");
        let visible = p.visible();
        assert_eq!(visible.len(), VISIBLE_ROWS);
        assert!(visible.iter().any(|(r, s)| *s && r.text == "cmd 20"));
        p.move_selection(100);
        assert_eq!(p.selected().unwrap().text, "cmd 29");
    }

    #[test]
    fn new_rows_keep_the_query() {
        let mut p = popup(&["git a", "ls"], "git");
        p.set_rows(vec![row("git b"), row("git c"), row("pwd")]);
        assert_eq!(p.query(), "git");
        assert_eq!(shown(&p), ["git b", "git c"]);
    }

    #[test]
    fn titles_show_the_filters() {
        let mut p = popup(&[], "");
        assert_eq!(p.title(), "Commands");
        p.only_here = true;
        p.only_ok = true;
        assert_eq!(p.title(), "Commands · this folder · exit 0");
        let d = HistoryPopup::new(PopupKind::Dirs, vec![], String::new(), None);
        assert_eq!(d.title(), "Folders");
        let sessions = HistoryPopup::new(PopupKind::Sessions, vec![], String::new(), None);
        assert_eq!(sessions.title(), "Sessions");
        assert!(sessions.footer().contains("Enter restore"));
        let themes = HistoryPopup::new(PopupKind::Themes, vec![], String::new(), None);
        assert_eq!(themes.title(), "Themes");
        assert!(themes.footer().contains("Enter use"));
        assert!(p.footer().contains("Shift+Enter"));
        assert!(d.footer().contains("Ctrl+P"));
    }

    #[test]
    fn command_rows_show_the_time_and_a_bad_exit_code() {
        let now = 100 * MIN;
        let entries = [
            CommandEntry {
                cmd: "make".into(),
                last: now - 5 * MIN,
                count: 3,
                exit: Some(2),
                cwd: Some("C:/a".into()),
            },
            CommandEntry {
                cmd: "ls".into(),
                last: now,
                count: 1,
                exit: Some(0),
                cwd: None,
            },
        ];
        let rows = command_rows(&entries, now);
        assert_eq!(rows[0].text, "make");
        assert!(rows[0].bad);
        assert_eq!(rows[0].hint, "exit 2 · 5 min");
        assert!(!rows[1].bad);
        assert_eq!(rows[1].hint, "now");
    }

    #[test]
    fn dir_rows_show_pins_and_missing_folders() {
        let now = 100 * MIN;
        let entries = [
            DirEntry {
                dir: "C:/work".into(),
                count: 5,
                last: now - 2 * MIN,
                pinned: true,
                score: 1.0,
            },
            DirEntry {
                dir: "C:/gone".into(),
                count: 1,
                last: now,
                pinned: false,
                score: 1.0,
            },
        ];
        let rows = dir_rows(&entries, now, |d| d != "C:/gone");
        assert_eq!(rows[0].hint, "★ 2 min");
        assert!(!rows[0].bad);
        assert_eq!(rows[1].hint, "not found");
        assert!(rows[1].bad);
        if cfg!(windows) {
            assert_eq!(rows[0].text, "C:\\work", "Windows folders with `\\`");
        }
    }

    #[test]
    fn replace_the_typed_text() {
        assert_eq!(replace_input("", "ls", false), b"\x1b[Fls");
        assert_eq!(
            replace_input("gi", "git status", true),
            b"\x1b[F\x7f\x7fgit status\r"
        );
        // Backspace for each char, not each byte.
        assert_eq!(replace_input("日本", "x", false), b"\x1b[F\x7f\x7fx");
    }

    #[test]
    fn cd_for_each_shell() {
        assert_eq!(
            cd_command("powershell.exe", "C:/Program Files/it's"),
            "Set-Location -LiteralPath 'C:/Program Files/it''s'"
        );
        assert_eq!(
            cd_command("pwsh", "C:/a"),
            "Set-Location -LiteralPath 'C:/a'"
        );
        assert_eq!(
            cd_command("C:\\Windows\\cmd.exe", "C:/a b"),
            "cd /d \"C:\\a b\""
        );
        assert_eq!(
            cd_command("bash", "/home/me/it's"),
            "cd -- '/home/me/it'\\''s'"
        );
        assert_eq!(cd_command("/bin/zsh", "/tmp"), "cd -- '/tmp'");
    }
}
