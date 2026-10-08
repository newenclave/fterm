//! The window title (like WezTerm): `[tab/tabs] title`, and the agents in other tabs that need you.

/// What the window title can show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TitleInfo {
    /// The active tab (from 1) and how many tabs there are.
    pub tab: usize,
    pub tabs: usize,
    /// The title of the active tab.
    pub title: String,
    /// The agent state in the active tab (`working`, `waiting`, ...), if there is one.
    pub agent: Option<String>,
    /// Agents in the other tabs that wait for you, and that failed.
    pub waiting: usize,
    pub failed: usize,
}

/// The default title: `[2/5] claude — ⏳ 1 waiting`. With one tab there is no `[1/1]`.
pub fn default_title(info: &TitleInfo) -> String {
    let mut title = if info.title.is_empty() {
        "fterm".to_owned()
    } else {
        info.title.clone()
    };
    if info.tabs > 1 {
        title = format!("[{}/{}] {title}", info.tab, info.tabs);
    }
    let mut status = Vec::new();
    if info.waiting > 0 {
        status.push(fterm_config::tr!("title.waiting", n = info.waiting));
    }
    if info.failed > 0 {
        status.push(fterm_config::tr!("title.failed", n = info.failed));
    }
    if !status.is_empty() {
        title = format!("{title} — {}", status.join(" · "));
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(tab: usize, tabs: usize, title: &str) -> TitleInfo {
        TitleInfo {
            tab,
            tabs,
            title: title.into(),
            ..TitleInfo::default()
        }
    }

    #[test]
    fn the_tab_number_like_wezterm() {
        assert_eq!(
            default_title(&info(6, 7, "setup-fterm")),
            "[6/7] setup-fterm"
        );
        assert_eq!(
            default_title(&info(1, 1, "powershell")),
            "powershell",
            "one tab: no number"
        );
        assert_eq!(default_title(&info(1, 1, "")), "fterm", "no title");
    }

    #[test]
    fn agents_in_other_tabs() {
        let mut i = info(2, 3, "claude");
        i.waiting = 1;
        assert_eq!(default_title(&i), "[2/3] claude — ⏳ 1 waiting");
        i.failed = 2;
        assert_eq!(
            default_title(&i),
            "[2/3] claude — ⏳ 1 waiting · ✗ 2 failed"
        );
        i.waiting = 0;
        assert_eq!(default_title(&i), "[2/3] claude — ✗ 2 failed");
    }
}
