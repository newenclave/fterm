//! Past conversations of agents: our own file (`agents.jsonl` next to the command history, written from the hooks)
//! and the conversation files of Claude Code (`<config>/projects/<folder>/<id>.jsonl`, read only). The Agents
//! panel lists them for a folder, and Enter brings one back with `claude --resume <id>`.

use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// The most bytes read from a conversation file of Claude to find its first prompt.
const PEEK: u64 = 64 * 1024;

/// One conversation in our file. A newer line for the same id takes the place of the older one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub cwd: String,
    /// The first prompt (100 chars).
    pub title: String,
    pub started_ms: u64,
    pub last_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

/// Adds a record at the end of the file (many windows may write: one line at a time).
pub fn append(file: &Path, record: &Record) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_string(record).map_err(std::io::Error::other)?;
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?
        .write_all(line.as_bytes())
}

/// The conversations of the file, the newest first: the last line of each id, with its first start.
pub fn load(file: &Path) -> Vec<Record> {
    let Ok(f) = std::fs::File::open(file) else {
        return Vec::new();
    };
    let mut by_id: HashMap<String, Record> = HashMap::new();
    for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
        let Ok(r) = serde_json::from_str::<Record>(&line) else {
            continue;
        };
        match by_id.get_mut(&r.id) {
            Some(old) => {
                let started = old.started_ms.min(r.started_ms);
                let title = if r.title.is_empty() {
                    old.title.clone()
                } else {
                    r.title.clone()
                };
                *old = Record {
                    started_ms: started,
                    title,
                    ..r
                };
            }
            None => {
                by_id.insert(r.id.clone(), r);
            }
        }
    }
    let mut out: Vec<Record> = by_id.into_values().collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.last_ms));
    out
}

/// The folder name that Claude Code gives a project: each char that is not a letter or a digit is `-`.
pub fn claude_folder_name(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The folder of the conversations of Claude Code for `cwd`: `CLAUDE_CONFIG_DIR` (else `~/.claude`)`/projects/...`.
pub fn claude_project_dir(
    config_dir: Option<&Path>,
    home: Option<&Path>,
    cwd: &str,
) -> Option<PathBuf> {
    let base = match config_dir {
        Some(dir) => dir.to_path_buf(),
        None => home?.join(".claude"),
    };
    Some(base.join("projects").join(claude_folder_name(cwd)))
}

/// A conversation file of Claude Code.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeFile {
    pub id: String,
    pub modified: SystemTime,
    /// The first prompt, when the file shows it (its format is not documented: best effort).
    pub title: Option<String>,
}

/// The conversation files in `dir`, the newest first. Only names that are good ids.
pub fn claude_files(dir: &Path) -> Vec<ClaudeFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<ClaudeFile> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_owned();
            if !crate::session_state::valid_session_id(&id) {
                return None;
            }
            let modified = e.metadata().ok()?.modified().ok()?;
            Some(ClaudeFile {
                title: first_prompt(&path),
                id,
                modified,
            })
        })
        .collect();
    out.sort_by_key(|f| std::cmp::Reverse(f.modified));
    out
}

/// The first text of the user in a conversation file of Claude Code: a line with `"type": "user"` and its
/// `message.content` (a text, or a list of parts with `text`).
fn first_prompt(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(PEEK)
        .read_to_string(&mut text)
        .ok();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("user") {
            continue;
        }
        let content = v.pointer("/message/content")?;
        let found = match content {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Array(parts) => parts
                .iter()
                .find_map(|p| p.get("text").and_then(|t| t.as_str()).map(str::to_owned)),
            _ => None,
        };
        if let Some(t) = found.filter(|t| !t.trim().is_empty() && !t.trim_start().starts_with('<'))
        {
            return Some(crate::agent_info::clean(&t, 100));
        }
    }
    None
}

/// One row of the list: the id, its title, when (ms since 1970), and the cost when we know it.
#[derive(Clone, Debug, PartialEq)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub when_ms: u64,
    pub cost: Option<f64>,
    /// From our file (else only from the folder of Claude).
    pub ours: bool,
}

/// The conversations of a folder: ours first (the newest first), then the files of Claude that we do not know.
pub fn conversations(ours: &[Record], cwd: &str, files: &[ClaudeFile]) -> Vec<Conversation> {
    let same = |a: &str| {
        a.trim_end_matches(['/', '\\'])
            .eq_ignore_ascii_case(cwd.trim_end_matches(['/', '\\']))
    };
    let mut out: Vec<Conversation> = ours
        .iter()
        .filter(|r| same(&r.cwd.replace('\\', "/")) || same(&r.cwd))
        .map(|r| Conversation {
            id: r.id.clone(),
            title: r.title.clone(),
            when_ms: r.last_ms,
            cost: r.cost,
            ours: true,
        })
        .collect();
    for f in files {
        if out.iter().any(|c| c.id == f.id) {
            continue;
        }
        let when_ms = f
            .modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        out.push(Conversation {
            id: f.id.clone(),
            title: f.title.clone().unwrap_or_default(),
            when_ms,
            cost: None,
            ours: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fterm-agents-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(id: &str, cwd: &str, title: &str, last: u64) -> Record {
        Record {
            id: id.into(),
            cwd: cwd.into(),
            title: title.into(),
            started_ms: last,
            last_ms: last,
            cost: None,
        }
    }

    #[test]
    fn our_file_keeps_the_last_line_of_each_conversation() {
        let file = temp("ours").join("agents.jsonl");
        append(&file, &record("a", "C:/w", "Fix the login", 100)).unwrap();
        append(&file, &record("b", "C:/w", "Add tests", 200)).unwrap();
        append(
            &file,
            &Record {
                cost: Some(0.5),
                title: String::new(),
                ..record("a", "C:/w", "", 300)
            },
        )
        .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        let all = load(&file);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "a", "the newest first");
        assert_eq!(
            all[0].title, "Fix the login",
            "an empty title keeps the old one"
        );
        assert_eq!(
            (all[0].started_ms, all[0].last_ms, all[0].cost),
            (100, 300, Some(0.5))
        );
        assert!(load(Path::new("no/such/file")).is_empty());
    }

    #[test]
    fn the_folder_of_claude_for_a_project() {
        assert_eq!(
            claude_folder_name("C:/work/github/fterm"),
            "C--work-github-fterm"
        );
        assert_eq!(
            claude_folder_name(r"C:\work\github\fterm"),
            "C--work-github-fterm"
        );
        assert_eq!(claude_folder_name("/home/a/my.app"), "-home-a-my-app");
        let home = Path::new("/home/a");
        assert_eq!(
            claude_project_dir(None, Some(home), "/w"),
            Some(home.join(".claude").join("projects").join("-w"))
        );
        assert_eq!(
            claude_project_dir(Some(Path::new("/cfg")), Some(home), "/w"),
            Some(Path::new("/cfg").join("projects").join("-w"))
        );
    }

    #[test]
    fn conversation_files_of_claude_with_their_first_prompt() {
        let dir = temp("claude");
        std::fs::write(
            dir.join("11111111-aaaa.jsonl"),
            concat!(
                r#"{"type":"summary","summary":"x"}"#, "\n",
                r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#, "\n",
                r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Make the build\nfaster"}]}}"#, "\n",
            ),
        )
        .unwrap();
        std::fs::write(dir.join("22222222-bbbb.jsonl"), "garbage\n").unwrap();
        std::fs::write(dir.join("bad name!.jsonl"), "").unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        let files = claude_files(&dir);
        let mut ids: Vec<&str> = files.iter().map(|f| f.id.as_str()).collect();
        ids.sort();
        assert_eq!(
            ids,
            ["11111111-aaaa", "22222222-bbbb"],
            "only good ids of .jsonl files"
        );
        let first = files.iter().find(|f| f.id == "11111111-aaaa").unwrap();
        assert_eq!(
            first.title.as_deref(),
            Some("Make the build faster"),
            "not the command, the prompt"
        );
        assert_eq!(
            files
                .iter()
                .find(|f| f.id == "22222222-bbbb")
                .unwrap()
                .title,
            None
        );
        assert!(claude_files(&dir.join("missing")).is_empty());
    }

    #[test]
    fn ours_first_then_the_files_we_do_not_know() {
        let ours = [
            record("a", "C:/w", "Fix the login", 300),
            record("x", "C:/other", "Elsewhere", 400),
        ];
        let files = [
            ClaudeFile {
                id: "a".into(),
                modified: SystemTime::UNIX_EPOCH,
                title: None,
            },
            ClaudeFile {
                id: "old".into(),
                modified: SystemTime::UNIX_EPOCH,
                title: Some("Old one".into()),
            },
        ];
        let list = conversations(&ours, "C:/w/", &files);
        let ids: Vec<&str> = list.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["a", "old"], "only this folder, and no twice");
        assert!(list[0].ours && !list[1].ours);
        assert_eq!(list[1].title, "Old one");
    }
}
