//! The environment of new panes.

/// `PATH` with `dir` at the end (when it is not there yet), so programs that come with fterm
/// (`ftermctl`) work in every pane. At the end, so fterm never hides a program of the user.
pub fn path_with(path: Option<&str>, dir: &str, windows: bool) -> String {
    let separator = if windows { ';' } else { ':' };
    let same = |a: &str| {
        if windows {
            a.trim_end_matches(['\\', '/'])
                .eq_ignore_ascii_case(dir.trim_end_matches(['\\', '/']))
        } else {
            a.trim_end_matches('/') == dir.trim_end_matches('/')
        }
    };
    match path.filter(|p| !p.is_empty()) {
        None => dir.to_owned(),
        Some(path) if path.split(separator).any(same) => path.to_owned(),
        Some(path) => format!("{path}{separator}{dir}"),
    }
}

/// `WSLENV` with the fterm vars, so they go into WSL panes too (`ftermctl.exe` and the hooks need them).
/// `/u` = only from Windows to WSL; the values are not paths, so they do not change on the way.
pub fn wslenv_with(wslenv: Option<&str>) -> String {
    let mut parts: Vec<String> = wslenv
        .unwrap_or("")
        .split(':')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    for name in ["TERM_PROGRAM", "FTERM_PANE_ID", "FTERM_SOCKET"] {
        let there = parts.iter().any(|p| {
            p.split('/')
                .next()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        });
        if !there {
            parts.push(format!("{name}/u"));
        }
    }
    parts.join(":")
}

/// `FTERM_RECORD` (a folder): the recording of a pane. The pid keeps two windows apart.
pub fn record_file(dir: &std::path::Path, pid: u32, pane: u64, unix_ms: u64) -> std::path::PathBuf {
    dir.join(format!("fterm-{pid}-pane-{pane}-{unix_ms}.cast"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_pane_has_its_own_recording() {
        let dir = std::path::Path::new("rec");
        assert_eq!(
            record_file(dir, 4321, 3, 1_790_000_000_123),
            dir.join("fterm-4321-pane-3-1790000000123.cast")
        );
    }

    #[test]
    fn the_fterm_vars_go_into_wsl() {
        let ours = "TERM_PROGRAM/u:FTERM_PANE_ID/u:FTERM_SOCKET/u";
        assert_eq!(wslenv_with(None), ours);
        assert_eq!(wslenv_with(Some("")), ours);
        // The user's vars stay first.
        assert_eq!(wslenv_with(Some("GOPATH/l")), format!("GOPATH/l:{ours}"));
        // A var that is there already (with any flags, any case) is not added again.
        assert_eq!(
            wslenv_with(Some("fterm_socket/p:USERPROFILE/pu")),
            "fterm_socket/p:USERPROFILE/pu:TERM_PROGRAM/u:FTERM_PANE_ID/u"
        );
    }

    #[test]
    fn the_folder_goes_to_the_end() {
        assert_eq!(
            path_with(Some(r"C:\Windows;C:\bin"), r"C:\fterm", true),
            r"C:\Windows;C:\bin;C:\fterm"
        );
        assert_eq!(
            path_with(Some("/usr/bin"), "/opt/fterm", false),
            "/usr/bin:/opt/fterm"
        );
        assert_eq!(path_with(None, r"C:\fterm", true), r"C:\fterm");
        assert_eq!(path_with(Some(""), r"C:\fterm", true), r"C:\fterm");
    }

    #[test]
    fn not_two_times() {
        assert_eq!(
            path_with(Some(r"C:\a;C:\FTERM\"), r"C:\fterm", true),
            r"C:\a;C:\FTERM\",
            "Windows: no case, no end slash"
        );
        assert_eq!(
            path_with(Some("/opt/fterm:/usr/bin"), "/opt/fterm", false),
            "/opt/fterm:/usr/bin"
        );
        assert_eq!(
            path_with(Some("/opt/FTERM"), "/opt/fterm", false),
            "/opt/FTERM:/opt/fterm",
            "Unix: case matters"
        );
    }
}
