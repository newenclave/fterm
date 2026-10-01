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

#[cfg(test)]
mod tests {
    use super::*;

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
