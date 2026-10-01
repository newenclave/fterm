//! Restore the session (Phase 9b): what fterm saves about its window, and how a layout comes back.

use std::path::{Path, PathBuf};

use fterm_mux::{Direction, Layout, PaneId};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedSession {
    pub version: u32,
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
}
