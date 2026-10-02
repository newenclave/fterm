//! Where fterm keeps its data: the history, the sessions, and the shell scripts. With `data_dir` in
//! the config they are all in one folder (a portable fterm); else in the folders of the user.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// `data_dir` of the config, set at start.
static DATA_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn set_data_dir(dir: Option<PathBuf>) {
    *DATA_DIR.write().unwrap() = dir;
}

pub fn data_dir() -> Option<PathBuf> {
    DATA_DIR.read().unwrap().clone()
}

/// `data_dir` as written in the config: a relative path is from the folder of the config file,
/// and `~` is the home folder.
pub fn resolve(config_file: &Path, data_dir: &str, home: Option<&Path>) -> PathBuf {
    let dir = fterm_config::profiles::expand_home(data_dir, home);
    if dir.is_absolute() {
        return dir;
    }
    config_file
        .parent()
        .map_or_else(|| dir.clone(), |folder| folder.join(&dir))
}

/// One folder of fterm: the env var (for tests and scripts), else `sub` in `data_dir`, else the
/// usual folder.
pub fn folder(
    env: Option<PathBuf>,
    data_dir: Option<&Path>,
    sub: &str,
    usual: Option<PathBuf>,
) -> Option<PathBuf> {
    env.or_else(|| data_dir.map(|dir| dir.join(sub))).or(usual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_data_folder_is_next_to_the_config() {
        // Full paths look different on Windows and on unix.
        let (folder, full, home) = if cfg!(windows) {
            ("D:/tools/fterm", "E:/fterm-data", "C:/Users/me")
        } else {
            ("/opt/fterm", "/srv/fterm-data", "/home/me")
        };
        let folder = Path::new(folder);
        let config = folder.join("fterm.lua");
        assert_eq!(resolve(&config, "data", None), folder.join("data"));
        assert_eq!(resolve(&config, full, None), PathBuf::from(full));
        let home = Path::new(home);
        assert_eq!(resolve(&config, "~/fterm", Some(home)), home.join("fterm"));
        assert_eq!(resolve(&config, ".", None), folder.join("."));
    }

    #[test]
    fn the_env_var_then_the_data_folder_then_the_usual_one() {
        let usual = Some(PathBuf::from("C:/Users/me/AppData/Roaming/fterm/history"));
        let data = Path::new("D:/tools/fterm/data");
        assert_eq!(
            folder(
                Some(PathBuf::from("T:/test")),
                Some(data),
                "history",
                usual.clone()
            ),
            Some(PathBuf::from("T:/test"))
        );
        assert_eq!(
            folder(None, Some(data), "history", usual.clone()),
            Some(data.join("history"))
        );
        assert_eq!(folder(None, None, "history", usual.clone()), usual);
    }
}
