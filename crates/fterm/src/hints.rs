//! Grey hints while you type (like Far Manager and fish): the rest of a command from the history.

use fterm_history::CommandEntry;

/// The rest of the newest command that starts with `typed`. Commands from this folder (`here`) come first,
/// then all commands. `None` when nothing fits or nothing is typed.
pub fn pick_hint(typed: &str, here: &[CommandEntry], all: &[CommandEntry]) -> Option<String> {
    if typed.trim().is_empty() {
        return None;
    }
    here.iter()
        .chain(all)
        .find(|e| e.cmd.len() > typed.len() && e.cmd.starts_with(typed))
        .map(|e| e.cmd[typed.len()..].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(cmd: &str) -> CommandEntry {
        CommandEntry {
            cmd: cmd.into(),
            last: 0,
            count: 1,
            exit: Some(0),
            cwd: None,
        }
    }

    #[test]
    fn the_rest_of_the_newest_command() {
        let all = [entry("git status"), entry("git push"), entry("ls")];
        assert_eq!(pick_hint("git", &[], &all).as_deref(), Some(" status"));
        assert_eq!(pick_hint("git p", &[], &all).as_deref(), Some("ush"));
    }

    #[test]
    fn this_folder_first() {
        let here = [entry("cargo test -p fterm")];
        let all = [entry("cargo build"), entry("cargo test -p fterm")];
        assert_eq!(
            pick_hint("cargo", &here, &all).as_deref(),
            Some(" test -p fterm")
        );
        assert_eq!(pick_hint("cargo b", &here, &all).as_deref(), Some("uild"));
    }

    #[test]
    fn no_hint() {
        let all = [entry("git status")];
        assert_eq!(pick_hint("", &[], &all), None, "nothing typed");
        assert_eq!(pick_hint("   ", &[], &all), None);
        assert_eq!(
            pick_hint("git status", &[], &all),
            None,
            "all of it is typed"
        );
        assert_eq!(pick_hint("svn", &[], &all), None);
        assert_eq!(
            pick_hint("GIT", &[], &all),
            None,
            "the case must be the same"
        );
    }
}
