//! Past conversations of agents: our file (from the hooks), the files of Claude Code, and their list (H in the
//! Agents panel); Enter brings one back in a new tab with `claude --resume <id>`.

use super::*;
use crate::agent_history::{self, Record};
use crate::history_popup::{HistoryPopup, PopupKind, PopupRow};

impl App {
    /// Our file of conversations: `agents.jsonl` next to the command history.
    fn agents_file(&self) -> Option<std::path::PathBuf> {
        crate::paths::folder(
            std::env::var_os("FTERM_HISTORY_DIR").map(std::path::PathBuf::from),
            crate::paths::data_dir().as_deref(),
            "history",
            fterm_history::default_folder(),
        )
        .map(|dir| dir.join("agents.jsonl"))
    }

    /// The conversation of `pane` took a step that counts (a prompt, a stop): it goes into our file.
    pub(super) fn remember_conversation(&self, pane: PaneId) {
        if !self.config.config.agents.history {
            return;
        }
        let Some(p) = self.running.as_ref().and_then(|r| r.panes.get(&pane)) else {
            return;
        };
        let (Some(id), Some(cwd), Some(file)) =
            (&p.agent_session, &p.shell.cwd, self.agents_file())
        else {
            return;
        };
        let now = fterm_history::now_ms();
        let record = Record {
            id: id.clone(),
            cwd: cwd.clone(),
            title: p
                .agent_info
                .first_prompt
                .as_deref()
                .map(|t| crate::agent_info::clean(t, 100))
                .unwrap_or_default(),
            started_ms: now,
            last_ms: now,
            cost: p.agent_info.usage.as_ref().and_then(|u| u.cost_usd),
        };
        if let Err(err) = agent_history::append(&file, &record) {
            tracing::warn!("cannot write {}: {err}", file.display());
        }
    }

    /// `H` in the Agents panel: the past conversations of the folder of `pane` (else of the active pane).
    pub(super) fn open_conversations(&mut self, pane: Option<PaneId>) {
        let cwd = self.running.as_ref().and_then(|r| {
            pane.and_then(|p| r.panes.get(&p))
                .or_else(|| r.active_pane())
                .and_then(|p| p.shell.cwd.clone())
        });
        let Some(cwd) = cwd else {
            return self.notify(None, &tr!("toast.no_folder"), "", Level::Info, Source::App);
        };
        if let Some(running) = &mut self.running {
            running.dock.focused = false;
        }
        self.palette = None;
        self.history_popup = Some(HistoryPopup::new(
            PopupKind::Conversations,
            Vec::new(),
            String::new(),
            Some(cwd),
        ));
        self.refresh_history_popup();
    }

    /// The rows of the list: ours first, then the files of Claude; `key` is the id.
    pub(super) fn conversation_rows(&self, cwd: Option<&str>) -> Vec<PopupRow> {
        let Some(cwd) = cwd else {
            return Vec::new();
        };
        let ours = self
            .agents_file()
            .filter(|_| self.config.config.agents.history)
            .map(|f| agent_history::load(&f))
            .unwrap_or_default();
        let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(std::path::PathBuf::from);
        let home = fterm_config::profiles::home_dir();
        let files = agent_history::claude_project_dir(config_dir.as_deref(), home.as_deref(), cwd)
            .map(|dir| agent_history::claude_files(&dir))
            .unwrap_or_default();
        let now = fterm_history::now_ms();
        agent_history::conversations(&ours, cwd, &files)
            .into_iter()
            .map(|c| {
                let mut hint = vec![crate::panels::long_ago(
                    Duration::from_millis(now.saturating_sub(c.when_ms)),
                    false,
                )];
                if let Some(cost) = c.cost {
                    hint.push(format!("${cost:.2}"));
                }
                if !c.ours {
                    hint.push("Claude".to_owned());
                }
                PopupRow {
                    text: if c.title.is_empty() {
                        tr!(
                            "popup.no_title",
                            id = c.id.chars().take(8).collect::<String>()
                        )
                    } else {
                        c.title
                    },
                    hint: hint.join(" · "),
                    key: c.id,
                    ..Default::default()
                }
            })
            .collect()
    }

    /// Enter in the list: a new tab in that folder, and `claude --resume <id>` runs in it.
    pub(super) fn resume_conversation(&mut self, id: &str, cwd: Option<String>) {
        if !crate::session_state::valid_session_id(id) {
            return;
        }
        self.spawn_cwd = cwd;
        let opened = self.new_tab(None);
        self.spawn_cwd = None;
        match opened {
            Ok(pane) => {
                if let Some(p) = self.running.as_mut().and_then(|r| r.panes.get_mut(&pane)) {
                    p.rerun = Some((format!("claude --resume {id}"), true));
                    p.agent_session = Some(id.to_owned());
                }
            }
            Err(err) => self.notify(
                None,
                &tr!("toast.cannot_start"),
                &format!("{err:#}"),
                Level::Error,
                Source::App,
            ),
        }
    }
}
