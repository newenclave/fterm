//! Mux panes: their programs live in the mux server (`fterm --mux-server`), so they go on when the window closes.
//! The window keeps its own copy of each Term (`Session::remote`): on attach it gets a snapshot, then the output.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fterm_server::client::{Handlers, MuxClient};
use fterm_term::session::{RemoteFeed, SessionOptions};

use super::*;
use crate::session_state::SavedSession;

/// Makes the remote session of server pane `rid`: `first` (a snapshot) goes into its Term before the output.
pub(super) fn remote_session(
    client: &Arc<MuxClient>,
    rid: u64,
    options: &SessionOptions,
    size: GridSize,
    first: Option<&[u8]>,
    on_event: impl Fn(TermEvent) + Send + Sync + 'static,
) -> Session {
    let (session, feed) = Session::remote(options, size, client.link(rid), on_event);
    let feed: Arc<RemoteFeed> = Arc::new(feed);
    if let Some(bytes) = first {
        feed.output(bytes);
    }
    let (output, exit) = (feed.clone(), feed);
    client.set_handlers(
        rid,
        Handlers {
            output: Box::new(move |bytes| output.output(bytes)),
            exit: Box::new(move || exit.exit()),
        },
    );
    session
}

impl App {
    /// New panes of this profile live in the mux server.
    pub(super) fn mux_wanted(&self, profile: Option<&str>) -> bool {
        self.profile(profile)
            .and_then(|p| p.mux)
            .unwrap_or(self.config.config.mux)
    }

    /// The connection to the mux server. `start` = start the server when it is not there.
    pub(super) fn mux_client(&mut self, start: bool) -> Option<Arc<MuxClient>> {
        if let Some(client) = &self.mux {
            return Some(client.clone());
        }
        let name = fterm_server::socket_name();
        let proxy = self.proxy.clone();
        let connect = || {
            let proxy = proxy.clone();
            MuxClient::connect(&name, move || {
                let _ = proxy.send_event(UserEvent::MuxGone);
            })
        };
        let client = match connect() {
            Ok(client) => client,
            Err(_) if start => {
                if let Err(err) = start_server() {
                    tracing::warn!("cannot start the mux server: {err}");
                    return None;
                }
                // It needs a moment to listen.
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    match connect() {
                        Ok(client) => break client,
                        Err(_) if Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        Err(err) => {
                            tracing::warn!("cannot reach the mux server: {err}");
                            return None;
                        }
                    }
                }
            }
            Err(_) => return None,
        };
        tracing::info!(%name, "connected to the mux server");
        self.mux = Some(client.clone());
        Some(client)
    }

    /// Starts a program in the mux server for a new pane. Returns the session and the server pane id.
    pub(super) fn spawn_mux_session(
        &mut self,
        id: PaneId,
        options: &SessionOptions,
        size: GridSize,
    ) -> anyhow::Result<(Session, u64)> {
        let client = self
            .mux_client(true)
            .ok_or_else(|| anyhow::anyhow!("the mux server does not run"))?;
        let spawn = fterm_server::proto::Spawn {
            program: options.program.clone(),
            args: options.args.clone(),
            cwd: options.cwd.as_ref().map(|p| p.display().to_string()),
            env: options.env.clone(),
            scrollback: options.scrollback,
        };
        let (rid, _) = client.spawn(spawn, size).map_err(anyhow::Error::msg)?;
        let proxy = self.proxy.clone();
        let session = remote_session(&client, rid, options, size, None, move |event| {
            let _ = proxy.send_event(UserEvent::Term(id, event));
        });
        Ok((session, rid))
    }

    /// Shows server pane `rid` in a new pane of this window (not in a tab yet). The pane gets its snapshot and
    /// what the server knows of it (the title, the folder, the prompt, the agent).
    pub(super) fn attach_mux_pane(&mut self, rid: u64, profile: Option<String>) -> Option<PaneId> {
        let client = self.mux_client(false)?;
        let (snapshot, meta, program, server_size) = match client.attach(rid) {
            Ok(answer) => answer,
            Err(err) => {
                tracing::warn!(pane = rid, "cannot attach: {err}");
                return None;
            }
        };
        let running = self.running.as_mut()?;
        let id = running.mux.new_pane_id();
        let options = SessionOptions {
            program: Some(program),
            scrollback: self.config.config.scrollback,
            ..SessionOptions::default()
        };
        let proxy = self.proxy.clone();
        let session = remote_session(
            &client,
            rid,
            &options,
            server_size,
            Some(&snapshot),
            move |event| {
                let _ = proxy.send_event(UserEvent::Term(id, event));
            },
        );
        running.panes.insert(
            id,
            Pane {
                session,
                app_title: (!meta.title.is_empty()).then(|| meta.title.clone()),
                shell: ShellState::default(),
                agent: None,
                agent_hooks: false,
                last_command: None,
                remote: true,
                opened_by: None,
                profile,
                rerun: None,
                intro_lines: 0,
                scene: None,
                review: None,
                harmonize: true,
                palette_changes: None,
                mux: Some(rid),
            },
        );
        // What the output does not show again: the same events as for a local pane.
        for event in meta.osc {
            let _ = self
                .proxy
                .send_event(UserEvent::Term(id, TermEvent::Osc(event)));
        }
        tracing::info!(pane = id.0, server_pane = rid, "attached a mux pane");
        Some(id)
    }

    /// At start: the panes of the mux server that no window shows come back, in the layout of the last window
    /// (its other panes start again, like a restored session). True when something came back.
    pub(super) fn reattach_mux(&mut self) -> bool {
        let Some(client) = self.mux_client(false) else {
            return false;
        };
        let free: Vec<u64> = match client.list() {
            Ok(panes) => panes
                .into_iter()
                .filter(|p| !p.attached)
                .map(|p| p.pane)
                .collect(),
            Err(_) => return false,
        };
        if free.is_empty() {
            return false;
        }
        self.mux_free = free.clone();
        let layout = client
            .layout()
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_value::<SavedSession>(v).ok());
        if let Some(saved) = layout {
            self.restore_session(saved);
        }
        // Panes that the layout does not have: each in its own tab.
        for rid in std::mem::take(&mut self.mux_free) {
            if let Some(id) = self.attach_mux_pane(rid, None)
                && let Some(running) = &mut self.running
            {
                running.mux.add_tab(fterm_mux::Layout::Pane(id), id, None);
            }
        }
        self.tab_changed();
        let n = free.len();
        self.notify(
            None,
            &trn!("toast.mux_back", n),
            "",
            Level::Info,
            Source::App,
        );
        true
    }

    /// At the end of the window: the server keeps the layout, for the next window.
    pub(super) fn save_mux_layout(&mut self) {
        let has_mux = self
            .running
            .as_ref()
            .is_some_and(|r| r.panes.values().any(|p| p.mux.is_some()));
        if !has_mux {
            return;
        }
        let (Some(client), Some(session)) = (self.mux.clone(), self.capture_session()) else {
            return;
        };
        if let Ok(value) = serde_json::to_value(&session) {
            client.set_layout(value);
        }
    }

    /// The window closes because the user closed its last tab: the mux panes of that tab end (the others go on).
    pub(super) fn end_mux_panes_on_exit(&mut self) {
        let ending = std::mem::take(&mut self.mux_end_on_exit);
        let Some(client) = self.mux.clone() else {
            return;
        };
        if ending.is_empty() {
            return;
        }
        for rid in &ending {
            client.close(*rid);
        }
        // An answer comes after the server did the closes before it: then the window may go.
        let _ = client.list();
        if let Some(running) = &mut self.running {
            for p in running.panes.values_mut() {
                if p.mux.is_some_and(|rid| ending.contains(&rid)) {
                    p.mux = None;
                }
            }
        }
    }

    /// `stop_mux`: asks first, with the number of panes that end.
    pub(super) fn ask_stop_mux(&mut self) {
        let Some(client) = self.mux_client(false) else {
            return self.notify(None, &tr!("toast.no_mux"), "", Level::Info, Source::App);
        };
        let n = client.list().map_or(0, |p| p.len());
        self.mux_stop_question = Some(vec![
            tr!("box.mux_stop"),
            trn!("box.mux_stop_panes", n),
            String::new(),
            tr!("box.mux_stop_keys"),
        ]);
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Keys of the stop question: Enter stops the server, Esc does nothing.
    pub(super) fn mux_stop_key(&mut self, event: &KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => {
                self.mux_stop_question = None;
                if let Some(client) = self.mux.clone() {
                    match client.stop() {
                        Ok(n) => {
                            // The server goes on purpose: no "the server stopped" warning after it.
                            self.mux = None;
                            self.notify(
                                None,
                                &tr!("toast.mux_stopped"),
                                &trn!("box.mux_stop_panes", n),
                                Level::Info,
                                Source::App,
                            );
                        }
                        Err(err) => self.notify(
                            None,
                            &tr!("toast.mux_stop_error"),
                            &err,
                            Level::Error,
                            Source::App,
                        ),
                    }
                }
            }
            Key::Named(NamedKey::Escape) => self.mux_stop_question = None,
            _ => return,
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// The user closed a mux pane: its program ends in the server too.
    pub(super) fn close_mux_pane(&self, rid: Option<u64>) {
        if let (Some(rid), Some(client)) = (rid, &self.mux) {
            client.close(rid);
        }
    }

    /// The server stopped: its panes got their end already (the client calls their exit).
    pub(super) fn mux_gone(&mut self) {
        if self.mux.take().is_some() {
            self.notify(
                None,
                &tr!("toast.mux_gone"),
                "",
                Level::Warning,
                Source::App,
            );
        }
    }
}

/// Starts `fterm --mux-server` in the background (with no window and no console).
fn start_server() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("--mux-server")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NO_WINDOW: no console, so it does not end with the console of this window. Not
        // CREATE_NEW_PROCESS_GROUP: that turns Ctrl+C off for the server and every program in its panes.
        command.creation_flags(0x0000_0008 | 0x0800_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own session: closing the terminal of fterm does not end it.
        command.process_group(0);
    }
    command.spawn().map(|_| ())
}
