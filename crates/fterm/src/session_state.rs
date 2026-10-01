//! Restore the session (Phase 9b): what fterm saves about its window, and how a layout comes back.

use std::path::{Path, PathBuf};

use fterm_mux::{Direction, Layout, PaneId};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedSession {
    pub version: u32,
    /// The name of a session that the user saved ("Save session as…").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// When it was saved (unix ms).
    pub saved: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<SavedWindow>,
    pub active_tab: usize,
    pub tabs: Vec<SavedTab>,
    #[serde(default)]
    pub dock: SavedDock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub maximized: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The active pane: its number in the order of `Layout::panes`.
    pub active: usize,
    pub layout: SavedLayout,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedLayout {
    Pane(SavedPane),
    Split {
        /// `right` or `down`.
        direction: String,
        ratio: f32,
        first: Box<SavedLayout>,
        second: Box<SavedLayout>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPane {
    /// The profile that started the pane (`None` = the default profile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The program of the pane (for the question text), for example `powershell`.
    #[serde(default)]
    pub program: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedDock {
    #[serde(default)]
    pub open: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f32>,
}

/// The tree of a tab, with what `pane` says about each pane.
pub fn save_layout(layout: &Layout, pane: &dyn Fn(PaneId) -> SavedPane) -> SavedLayout {
    match layout {
        Layout::Pane(id) => SavedLayout::Pane(pane(*id)),
        Layout::Split {
            direction,
            ratio,
            first,
            second,
        } => SavedLayout::Split {
            direction: match direction {
                Direction::Right => "right".to_owned(),
                Direction::Down => "down".to_owned(),
            },
            ratio: *ratio,
            first: Box::new(save_layout(first, pane)),
            second: Box::new(save_layout(second, pane)),
        },
    }
}

/// Builds the tree again. `spawn` starts one pane and gives its id (`None` = it could not start;
/// then that part of the tree goes away and the other side takes its place).
pub fn restore_layout(
    saved: &SavedLayout,
    spawn: &mut dyn FnMut(&SavedPane) -> Option<PaneId>,
) -> Option<Layout> {
    match saved {
        SavedLayout::Pane(p) => spawn(p).map(Layout::Pane),
        SavedLayout::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let first = restore_layout(first, spawn);
            let second = restore_layout(second, spawn);
            match (first, second) {
                (Some(first), Some(second)) => Some(Layout::Split {
                    direction: if direction == "down" {
                        Direction::Down
                    } else {
                        Direction::Right
                    },
                    ratio: ratio.clamp(fterm_mux::layout::MIN_RATIO, fterm_mux::layout::MAX_RATIO),
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (one, None) | (None, one) => one,
            }
        }
    }
}

impl SavedSession {
    pub fn pane_count(&self) -> usize {
        fn count(l: &SavedLayout) -> usize {
            match l {
                SavedLayout::Pane(_) => 1,
                SavedLayout::Split { first, second, .. } => count(first) + count(second),
            }
        }
        self.tabs.iter().map(|t| count(&t.layout)).sum()
    }

    /// The text of the question: "4 tabs, 7 panes, saved 10 min ago".
    pub fn describe(&self, now: u64) -> String {
        let plural = |n: usize, word: &str| {
            if n == 1 {
                format!("1 {word}")
            } else {
                format!("{n} {word}s")
            }
        };
        let age = crate::panels::short_ago(std::time::Duration::from_millis(
            now.saturating_sub(self.saved),
        ));
        let when = if age == "now" {
            "saved now".to_owned()
        } else {
            format!("saved {age} ago")
        };
        format!(
            "{}, {}, {when}",
            plural(self.tabs.len(), "tab"),
            plural(self.pane_count(), "pane")
        )
    }
}

/// `%LOCALAPPDATA%\fterm\session.json` (Linux and macOS: `~/.local/state/fterm/session.json`).
/// `FTERM_SESSION_FILE` wins (for tests).
pub fn default_path() -> Option<PathBuf> {
    if let Some(file) = std::env::var_os("FTERM_SESSION_FILE") {
        return Some(PathBuf::from(file));
    }
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")));
    Some(base?.join("fterm").join("session.json"))
}

/// Writes the file (a temp file, then rename, so a crash never leaves half a file).
pub fn save(path: &Path, session: &SavedSession) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(session).map_err(std::io::Error::other)?;
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// Reads the file. A missing, broken, or too new file gives `None`.
pub fn load(path: &Path) -> Option<SavedSession> {
    let text = std::fs::read_to_string(path).ok()?;
    let session: SavedSession = serde_json::from_str(&text).ok()?;
    (session.version <= VERSION && !session.tabs.is_empty()).then_some(session)
}

/// How many closed sessions are kept (like "recently closed" in a browser).
pub const KEEP_CLOSED: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A window that closed (or crashed).
    Closed,
    /// Saved with a name by the user (never deleted by fterm).
    Named,
}

/// One saved session in the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub kind: EntryKind,
    pub session: SavedSession,
}

/// `%LOCALAPPDATA%\fterm\sessions` (Linux and macOS: `~/.local/state/fterm/sessions`).
/// `FTERM_SESSION_DIR` wins (for tests).
pub fn sessions_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FTERM_SESSION_DIR") {
        return Some(PathBuf::from(dir));
    }
    Some(default_path()?.parent()?.join("sessions"))
}

/// The file that the window with this pid saves to while it runs.
pub fn live_path(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("live-{pid}.json"))
}

/// The window closed: its live file becomes a closed session. Only the newest `KEEP_CLOSED` stay.
pub fn close_live(dir: &Path, pid: u32) {
    let live = live_path(dir, pid);
    let Some(session) = load(&live) else {
        let _ = std::fs::remove_file(&live);
        return;
    };
    let closed = dir.join(format!("closed-{}-{pid}.json", session.saved));
    if std::fs::rename(&live, &closed).is_err() {
        return;
    }
    // Only the newest closed sessions stay.
    let mut closed: Vec<Entry> = list(dir)
        .into_iter()
        .filter(|e| e.kind == EntryKind::Closed)
        .collect();
    if closed.len() > KEEP_CLOSED {
        for old in closed.drain(KEEP_CLOSED..) {
            let _ = std::fs::remove_file(old.path);
        }
    }
}

/// Live files of windows that are not alive any more (a crash, a reboot) become closed sessions.
pub fn adopt_dead(dir: &Path, alive: &[u32]) {
    let Ok(files) = std::fs::read_dir(dir) else {
        return;
    };
    for file in files.flatten() {
        let name = file.file_name().to_string_lossy().into_owned();
        let pid = name
            .strip_prefix("live-")
            .and_then(|rest| rest.strip_suffix(".json"))
            .and_then(|pid| pid.parse::<u32>().ok());
        if let Some(pid) = pid
            && !alive.contains(&pid)
        {
            close_live(dir, pid);
        }
    }
}

/// Saves the session with a name. The same name takes the place of the old one.
pub fn save_named(dir: &Path, name: &str, session: &SavedSession) -> std::io::Result<PathBuf> {
    // A file name from the name: letters, digits, and `-`; the real name is in the file.
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "session" } else { slug };
    let path = dir.join(format!("named-{slug}.json"));
    let named = SavedSession {
        name: Some(name.trim().to_owned()),
        ..session.clone()
    };
    save(&path, &named)?;
    Ok(path)
}

/// All saved sessions: named ones first (by name), then closed ones (newest first). Live files are not in it.
pub fn list(dir: &Path) -> Vec<Entry> {
    let Ok(files) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Entry> = files
        .flatten()
        .filter_map(|file| {
            let name = file.file_name().to_string_lossy().into_owned();
            let kind = if name.starts_with("closed-") {
                EntryKind::Closed
            } else if name.starts_with("named-") {
                EntryKind::Named
            } else {
                return None;
            };
            let path = file.path();
            let session = load(&path)?;
            Some(Entry {
                path,
                kind,
                session,
            })
        })
        .collect();
    out.sort_by(|a, b| match (a.kind, b.kind) {
        (EntryKind::Named, EntryKind::Closed) => std::cmp::Ordering::Less,
        (EntryKind::Closed, EntryKind::Named) => std::cmp::Ordering::Greater,
        (EntryKind::Named, EntryKind::Named) => a.session.name.cmp(&b.session.name),
        (EntryKind::Closed, EntryKind::Closed) => b.session.saved.cmp(&a.session.saved),
    });
    out
}

/// The text of a session in the list: its name (★), or its folders; and "2 tabs, 3 panes · 10 min".
pub fn entry_text(entry: &Entry, now: u64) -> (String, String) {
    fn folders(l: &SavedLayout, out: &mut Vec<String>) {
        match l {
            SavedLayout::Pane(p) => {
                let dir = crate::history_popup::shown_dir(p.cwd.as_deref().unwrap_or("~"));
                if !out.contains(&dir) {
                    out.push(dir);
                }
            }
            SavedLayout::Split { first, second, .. } => {
                folders(first, out);
                folders(second, out);
            }
        }
    }
    let s = &entry.session;
    let text = match &s.name {
        Some(name) => format!("★ {name}"),
        None => {
            let mut all = Vec::new();
            for tab in &s.tabs {
                folders(&tab.layout, &mut all);
            }
            let mut text = all.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
            if all.len() > 2 {
                text.push_str(&format!(" +{}", all.len() - 2));
            }
            text
        }
    };
    let plural = |n: usize, word: &str| {
        if n == 1 {
            format!("1 {word}")
        } else {
            format!("{n} {word}s")
        }
    };
    let age = crate::panels::short_ago(std::time::Duration::from_millis(
        now.saturating_sub(s.saved),
    ));
    let hint = format!(
        "{}, {} · {age}",
        plural(s.tabs.len(), "tab"),
        plural(s.pane_count(), "pane")
    );
    (text, hint)
}

/// The newest closed session (for "Restore the last session?").
pub fn newest_closed(dir: &Path) -> Option<Entry> {
    list(dir).into_iter().find(|e| e.kind == EntryKind::Closed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(n: u64) -> SavedPane {
        SavedPane {
            profile: Some(format!("p{n}")),
            cwd: Some(format!("C:/d{n}")),
            program: "powershell".into(),
        }
    }

    fn tree() -> Layout {
        // [1 | [2 / 3]]
        Layout::Split {
            direction: Direction::Right,
            ratio: 0.3,
            first: Box::new(Layout::Pane(PaneId(1))),
            second: Box::new(Layout::Split {
                direction: Direction::Down,
                ratio: 0.6,
                first: Box::new(Layout::Pane(PaneId(2))),
                second: Box::new(Layout::Pane(PaneId(3))),
            }),
        }
    }

    #[test]
    fn a_layout_goes_and_comes_back() {
        let saved = save_layout(&tree(), &|id| pane(id.0));
        let mut next = 10;
        let mut started = Vec::new();
        let back = restore_layout(&saved, &mut |p| {
            started.push(p.cwd.clone().unwrap());
            next += 1;
            Some(PaneId(next))
        })
        .unwrap();
        assert_eq!(
            started,
            ["C:/d1", "C:/d2", "C:/d3"],
            "in the order of the panes"
        );
        let expected = Layout::Split {
            direction: Direction::Right,
            ratio: 0.3,
            first: Box::new(Layout::Pane(PaneId(11))),
            second: Box::new(Layout::Split {
                direction: Direction::Down,
                ratio: 0.6,
                first: Box::new(Layout::Pane(PaneId(12))),
                second: Box::new(Layout::Pane(PaneId(13))),
            }),
        };
        assert_eq!(back, expected);
    }

    #[test]
    fn a_pane_that_cannot_start_leaves_its_place_to_the_other_side() {
        let saved = save_layout(&tree(), &|id| pane(id.0));
        let back = restore_layout(&saved, &mut |p| {
            (p.cwd.as_deref() != Some("C:/d2")).then_some(PaneId(7))
        })
        .unwrap();
        // [1 | 3]: the split of 2 and 3 became only 3.
        assert_eq!(back.panes().len(), 2);
        assert!(matches!(
            back,
            Layout::Split {
                direction: Direction::Right,
                ..
            }
        ));
        assert_eq!(
            restore_layout(&saved, &mut |_| None),
            None,
            "nothing started"
        );
    }

    fn session() -> SavedSession {
        SavedSession {
            version: VERSION,
            name: None,
            saved: 1_000_000,
            window: Some(SavedWindow {
                width: 1200,
                height: 700,
                x: 10,
                y: 20,
                maximized: false,
            }),
            active_tab: 1,
            tabs: vec![
                SavedTab {
                    title: None,
                    active: 0,
                    layout: SavedLayout::Pane(pane(1)),
                },
                SavedTab {
                    title: Some("build".into()),
                    active: 2,
                    layout: save_layout(&tree(), &|id| pane(id.0)),
                },
            ],
            dock: SavedDock {
                open: true,
                panel: Some("ai".into()),
                ratio: Some(0.4),
            },
        }
    }

    #[test]
    fn the_question_text() {
        let s = session();
        assert_eq!(s.pane_count(), 4);
        assert_eq!(
            s.describe(1_000_000 + 10 * 60_000),
            "2 tabs, 4 panes, saved 10 min ago"
        );
        let one = SavedSession {
            tabs: vec![s.tabs[0].clone()],
            ..s
        };
        assert_eq!(one.describe(1_000_000 + 2_000), "1 tab, 1 pane, saved now");
    }

    #[test]
    fn the_file() {
        let dir = std::env::temp_dir().join(format!("fterm-session-{}", std::process::id()));
        let path = dir.join("session.json");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&path), None, "no file");
        save(&path, &session()).unwrap();
        assert_eq!(load(&path), Some(session()));
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), None, "a broken file");
        let newer = SavedSession {
            version: VERSION + 1,
            ..session()
        };
        save(&path, &newer).unwrap();
        assert_eq!(load(&path), None, "a file from a newer fterm");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fterm-sessions-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn at(saved: u64) -> SavedSession {
        SavedSession { saved, ..session() }
    }

    #[test]
    fn a_closed_window_goes_to_the_list() {
        let dir = temp("close");
        save(&live_path(&dir, 42), &at(1000)).unwrap();
        assert!(list(&dir).is_empty(), "a live window is not in the list");
        close_live(&dir, 42);
        assert!(!live_path(&dir, 42).exists());
        let entries = list(&dir);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::Closed);
        assert_eq!(entries[0].session.saved, 1000);
        assert_eq!(newest_closed(&dir).unwrap().session.saved, 1000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_newest_closed_sessions_stay() {
        let dir = temp("keep");
        for i in 0..(KEEP_CLOSED as u64 + 5) {
            save(&live_path(&dir, 7), &at(1000 + i)).unwrap();
            close_live(&dir, 7);
        }
        let entries = list(&dir);
        assert_eq!(entries.len(), KEEP_CLOSED);
        assert_eq!(
            entries[0].session.saved,
            1000 + KEEP_CLOSED as u64 + 4,
            "newest first"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_crashed_window_is_not_lost() {
        let dir = temp("crash");
        save(&live_path(&dir, 100), &at(5000)).unwrap();
        save(&live_path(&dir, 200), &at(6000)).unwrap();
        adopt_dead(&dir, &[200]);
        let entries = list(&dir);
        assert_eq!(
            entries.len(),
            1,
            "pid 100 is dead: its session is closed now"
        );
        assert_eq!(entries[0].session.saved, 5000);
        assert!(
            live_path(&dir, 200).exists(),
            "pid 200 is alive: it stays live"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn named_sessions_come_first_and_stay() {
        let dir = temp("named");
        save(&live_path(&dir, 1), &at(9000)).unwrap();
        close_live(&dir, 1);
        save_named(&dir, "my project", &at(100)).unwrap();
        save_named(&dir, "Backend: api/db", &at(200)).unwrap();
        let entries = list(&dir);
        let names: Vec<Option<&str>> = entries.iter().map(|e| e.session.name.as_deref()).collect();
        assert_eq!(names, [Some("Backend: api/db"), Some("my project"), None]);
        assert_eq!(entries[0].kind, EntryKind::Named);
        // The same name again: it takes the place of the old one.
        save_named(&dir, "my project", &at(300)).unwrap();
        assert_eq!(list(&dir).len(), 3);
        // Many closed windows never push a named session out.
        for i in 0..(KEEP_CLOSED as u64 + 3) {
            save(&live_path(&dir, 2), &at(10_000 + i)).unwrap();
            close_live(&dir, 2);
        }
        assert_eq!(
            list(&dir)
                .iter()
                .filter(|e| e.kind == EntryKind::Named)
                .count(),
            2
        );
        assert_eq!(newest_closed(&dir).unwrap().session.name, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_text_of_an_entry() {
        let closed = Entry {
            path: PathBuf::from("closed-1.json"),
            kind: EntryKind::Closed,
            session: at(1_000_000),
        };
        let (text, hint) = entry_text(&closed, 1_000_000 + 5 * 60_000);
        // The folders of the panes (each one time), short.
        assert_eq!(
            text,
            if cfg!(windows) {
                "C:\\d1, C:\\d2 +1"
            } else {
                "C:/d1, C:/d2 +1"
            }
        );
        assert_eq!(hint, "2 tabs, 4 panes · 5 min");
        let named = Entry {
            kind: EntryKind::Named,
            session: SavedSession {
                name: Some("my project".into()),
                ..at(1_000_000)
            },
            ..closed
        };
        assert_eq!(entry_text(&named, 1_000_000).0, "★ my project");
    }
}
