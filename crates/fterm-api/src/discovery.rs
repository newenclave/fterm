//! How a client finds its fterm window.
//!
//! Every pane has `FTERM_SOCKET` (the socket of its window). Outside of fterm, a client reads the files in
//! [`instances_dir`]: each window writes `<pid>.json` with its socket, and deletes it at the end.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The env var with the socket of the window (in every pane).
pub const SOCKET_ENV: &str = "FTERM_SOCKET";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    pub pid: u32,
    pub socket: String,
    /// When the window started (unix ms).
    pub started: u64,
}

/// `%LOCALAPPDATA%\fterm\instances` on Windows, `$XDG_RUNTIME_DIR/fterm/instances` (or a temp folder) elsewhere.
pub fn instances_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("fterm").join("instances")
}

/// The file of one window. It is deleted when this is dropped.
pub struct Registration {
    path: PathBuf,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn register(dir: &Path, instance: &Instance) -> io::Result<Registration> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", instance.pid));
    let text = serde_json::to_string(instance).map_err(io::Error::other)?;
    std::fs::write(&path, text)?;
    Ok(Registration { path })
}

/// All windows that wrote a file, newest first. Files of windows that are not alive are deleted.
pub fn instances(dir: &Path) -> Vec<Instance> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut alive = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let instance = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Instance>(&text).ok());
        match instance {
            Some(instance) if crate::transport::connect(&instance.socket).is_ok() => {
                alive.push(instance)
            }
            // A window that crashed, or a broken file.
            _ => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    alive.sort_by(|a, b| b.started.cmp(&a.started));
    alive
}

/// The socket to use: `FTERM_SOCKET` when it is set, else the window with this pid, else the newest window.
pub fn find(dir: &Path, env_socket: Option<&str>, pid: Option<u32>) -> Option<String> {
    if let Some(socket) = env_socket.filter(|s| !s.is_empty()) {
        return Some(socket.to_owned());
    }
    let list = instances(dir);
    match pid {
        Some(pid) => list.into_iter().find(|i| i.pid == pid).map(|i| i.socket),
        None => list.into_iter().next().map(|i| i.socket),
    }
}
