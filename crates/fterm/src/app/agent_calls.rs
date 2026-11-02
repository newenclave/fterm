//! Past conversations of agents: our file (from the hooks), the files of Claude Code, and their list (H in the
//! Agents panel); Enter brings one back in a new tab with `claude --resume <id>`.

use super::*;
use crate::agent_history::{self, Record};
use crate::history_popup::{HistoryPopup, PopupKind, PopupRow};

impl App {
    /// The agent of `pane` took a step: `on_agent_event` of the config sees it (and may call `fterm.*`), and
    /// `on_agent_row` makes its row again.
    pub(super) fn agent_changed(
        &mut self,
        event_loop: &ActiveEventLoop,
        pane: PaneId,
        event: Option<&crate::agent_info::AgentEvent>,
    ) {
        use crate::agent_info::AgentEvent;
        use fterm_config::load::{AgentEventIn, AgentRowIn};
        if let Some(event) = event {
            let usage = self
                .running
                .as_ref()
                .and_then(|r| r.panes.get(&pane))
                .and_then(|p| p.agent_info.usage.clone());
            let mut input = AgentEventIn {
                pane: pane.0,
                cost: usage.as_ref().and_then(|u| u.cost_usd),
                context: usage.as_ref().and_then(|u| u.context_pct),
                ..Default::default()
            };
            match event {
                AgentEvent::Prompt(text) => {
                    input.kind = "prompt".into();
                    input.text = Some(text.clone());
                }
                AgentEvent::Tool {
                    name,
                    file,
                    command,
                    ..
                } => {
                    input.kind = "tool".into();
                    input.tool = Some(name.clone());
                    input.file = file.clone();
                    input.command = command.clone();
                }
                AgentEvent::Question(text) => {
                    input.kind = "question".into();
                    input.text = Some(text.clone());
                }
                AgentEvent::Done(text) => {
                    input.kind = "done".into();
                    input.text = text.clone();
                }
                AgentEvent::Usage(_) => input.kind = "usage".into(),
            }
            match self.config.on_agent_event(&input) {
                Ok(calls) => {
                    for call in calls {
                        self.run_api_call(event_loop, call);
                    }
                }
                Err(err) => tracing::warn!("on_agent_event: {err}"),
            }
        }
        let Some(running) = &self.running else {
            return;
        };
        let Some(p) = running.panes.get(&pane) else {
            return;
        };
        let info = &p.agent_info;
        let (done, total, current) = info.todo_progress().unwrap_or((0, 0, None));
        let input = AgentRowIn {
            pane: pane.0,
            name: running.pane_tab_title(pane).unwrap_or_default(),
            state: p
                .agent
                .as_ref()
                .map_or("idle", |a| a.kind.name())
                .to_owned(),
            question: info.question.clone(),
            todos_done: done,
            todos_total: total,
            todo_current: current.map(str::to_owned),
            cost: info.usage.as_ref().and_then(|u| u.cost_usd),
            context: info.usage.as_ref().and_then(|u| u.context_pct),
            files: info.files.len(),
            last: info.last_step().map(str::to_owned),
        };
        let row = match self.config.on_agent_row(&input) {
            Ok(row) => row,
            Err(err) => {
                tracing::warn!("on_agent_row: {err}");
                None
            }
        };
        if let Some(p) = self.running.as_mut().and_then(|r| r.panes.get_mut(&pane)) {
            p.agent_row = row;
        }
    }

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
