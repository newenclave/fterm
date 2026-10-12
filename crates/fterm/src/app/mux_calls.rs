//! Mux panes: their programs live in a mux server, so they go on when the window closes. The server is the local one
//! (`fterm --mux-server`) or one on another machine (`remotes`, over `ssh ... ftermctl mux proxy`). The window
//! keeps its own copy of each Term (`Session::remote`): on attach it gets a snapshot, then the output.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fterm_config::load::Remote;
use fterm_server::client::{Handlers, MuxClient};
use fterm_term::session::{RemoteFeed, SessionOptions};

use super::*;
use crate::session_state::SavedSession;

/// The name of the local mux server in `App::muxes`.
pub(super) const LOCAL: &str = "";

/// A pane whose program lives in a mux server: the server (`LOCAL` or the name of a remote) and its pane id there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MuxPane {
    pub remote: String,
    pub id: u64,
}

impl MuxPane {
    pub fn is_local(&self) -> bool {
        self.remote == LOCAL
    }
}

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

/// How long a remote may take to answer `Hello` (ssh may ask the network, then start ftermctl there).
const HELLO: Duration = Duration::from_secs(20);

impl App {
    /// New panes of this profile live in the local mux server.
    pub(super) fn mux_wanted(&self, profile: Option<&str>) -> bool {
        self.profile(profile)
            .and_then(|p| p.mux)
            .unwrap_or(self.config.config.mux)
    }

    /// A remote by its name: one of the config, or a host from "Connect over SSH…".
    fn remote_named(&self, name: &str) -> Option<Remote> {
        self.config
            .config
            .remotes
            .iter()
            .chain(&self.adhoc_remotes)
            .find(|r| r.name == name)
            .cloned()
    }

    /// The connection to the mux server `remote` (`LOCAL` or a remote name). For the local server `start` = start
    /// it when it is not there; a remote is connected when it is not yet (with a toast on an error).
    pub(super) fn mux_client(&mut self, remote: &str, start: bool) -> Option<Arc<MuxClient>> {
        if let Some(client) = self.muxes.get(remote) {
            return Some(client.clone());
        }
        let proxy = self.proxy.clone();
        let gone_name = remote.to_owned();
        let gone = move || {
            let _ = proxy.send_event(UserEvent::MuxGone(gone_name.clone()));
        };
        let client = if remote == LOCAL {
            self.connect_local(start, gone)?
        } else {
            let r = self.remote_named(remote)?;
            let command = r.connect_command();
            tracing::info!(?command, "connecting to {remote}");
            let connected = MuxClient::connect_command(&command, &[], gone)
                .map_err(|err| err.to_string())
                .and_then(|client| client.hello(HELLO).map(|_| client));
            match connected {
                Ok(client) => client,
                Err(err) => {
                    self.notify(
                        None,
                        &tr!("toast.remote_error", remote = remote),
                        &err,
                        Level::Error,
                        Source::App,
                    );
                    return None;
                }
            }
        };
        self.muxes.insert(remote.to_owned(), client.clone());
        Some(client)
    }

    fn connect_local(
        &self,
        start: bool,
        gone: impl Fn() + Send + Clone + 'static,
    ) -> Option<Arc<MuxClient>> {
        let name = fterm_server::socket_name();
        let connect = || MuxClient::connect(&name, gone.clone());
        match connect() {
            Ok(client) => Some(client),
            Err(_) if start => {
                if let Err(err) = start_server() {
                    tracing::warn!("cannot start the mux server: {err}");
                    return None;
                }
                // It needs a moment to listen.
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    match connect() {
                        Ok(client) => break Some(client),
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
            Err(_) => None,
        }
    }

    /// Starts a program in the mux server `remote` for a new pane. A remote pane gets no local folder and no
    /// local environment: the server starts it in the home of its user.
    pub(super) fn spawn_mux_session(
        &mut self,
        remote: &str,
        id: PaneId,
        options: &SessionOptions,
        size: GridSize,
    ) -> anyhow::Result<(Session, MuxPane)> {
        let client = self
            .mux_client(remote, true)
            .ok_or_else(|| anyhow::anyhow!("no connection to the mux server {remote:?}"))?;
        let local = remote == LOCAL;
        let spawn = fterm_server::proto::Spawn {
            program: options.program.clone(),
            args: options.args.clone(),
            cwd: if local {
                options.cwd.as_ref().map(|p| p.display().to_string())
            } else {
                None
            },
            env: if local {
                options.env.clone()
            } else {
                Vec::new()
            },
            scrollback: options.scrollback,
        };
        let (rid, _) = client.spawn(spawn, size).map_err(anyhow::Error::msg)?;
        let proxy = self.proxy.clone();
        let session = remote_session(&client, rid, options, size, None, move |event| {
            let _ = proxy.send_event(UserEvent::Term(id, event));
        });
        Ok((
            session,
            MuxPane {
                remote: remote.to_owned(),
                id: rid,
            },
        ))
    }

    /// Shows pane `rid` of the server `remote` in a new pane of this window (not in a tab yet). The pane gets its
    /// snapshot and what the server knows of it (the title, the folder, the prompt, the agent).
    pub(super) fn attach_mux_pane(
        &mut self,
        remote: &str,
        rid: u64,
        profile: Option<String>,
    ) -> Option<PaneId> {
        let client = self.mux_client(remote, false)?;
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
                mux: Some(MuxPane {
                    remote: remote.to_owned(),
                    id: rid,
                }),
            },
        );
        // What the output does not show again: the same events as for a local pane.
        for event in meta.osc {
            let _ = self
                .proxy
                .send_event(UserEvent::Term(id, TermEvent::Osc(event)));
        }
        tracing::info!(pane = id.0, server_pane = rid, %remote, "attached a mux pane");
        Some(id)
    }

    /// The panes of server `remote` that no window shows.
    fn free_panes(client: &MuxClient) -> Vec<u64> {
        client.list().map_or(Vec::new(), |panes| {
            panes
                .into_iter()
                .filter(|p| !p.attached)
                .map(|p| p.pane)
                .collect()
        })
    }

    /// At start: the panes of the local mux server that no window shows come back, in the layout of the last
    /// window (its other panes start again, like a restored session). True when something came back.
    pub(super) fn reattach_mux(&mut self) -> bool {
        let Some(client) = self.mux_client(LOCAL, false) else {
            return false;
        };
        let free = Self::free_panes(&client);
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
            if let Some(id) = self.attach_mux_pane(LOCAL, rid, None)
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

    /// "Connect: <remote>": connects, and its panes that no window shows come back in a workspace named like the
    /// remote. With none, a new tab opens there.
    pub(super) fn connect_remote(&mut self, name: &str) {
        let Some(client) = self.mux_client(name, false) else {
            return;
        };
        let free = Self::free_panes(&client);
        if free.is_empty() {
            self.spawn_workspace = Some(name.to_owned());
            self.spawn_remote = Some(name.to_owned());
            self.spawn(None, SpawnWhere::Tab);
            return;
        }
        let mut shown = 0;
        for rid in &free {
            if let Some(id) = self.attach_mux_pane(name, *rid, None)
                && let Some(running) = &mut self.running
            {
                running
                    .mux
                    .add_tab_in(name, fterm_mux::Layout::Pane(id), id, None);
                shown += 1;
            }
        }
        if let Some(running) = &mut self.running {
            running.mux.switch_workspace(name);
            running.resize_all_panes();
        }
        self.tab_changed();
        self.notify(
            None,
            &trn!("toast.remote_back", shown, remote = name),
            "",
            Level::Info,
            Source::App,
        );
    }

    /// `connect_ssh`: a box that asks for `user@host`.
    pub(super) fn start_ssh_prompt(&mut self) {
        self.ssh_prompt = Some(crate::ai_chat::InputBox::default());
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    pub(super) fn ssh_prompt_lines(&self) -> Option<Vec<String>> {
        let input = self.ssh_prompt.as_ref()?;
        Some(vec![
            tr!("box.ssh"),
            format!("{}▏", input.text),
            String::new(),
            tr!("box.ssh_keys"),
        ])
    }

    pub(super) fn ssh_prompt_key(&mut self, event: &KeyEvent) {
        let ctrl = self.mods.control_key();
        let Some(input) = &mut self.ssh_prompt else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.ssh_prompt = None,
            Key::Named(NamedKey::Enter) => {
                let host = input.take().trim().to_owned();
                self.ssh_prompt = None;
                if !host.is_empty() {
                    if self.remote_named(&host).is_none() {
                        self.adhoc_remotes.push(Remote::ssh(&host));
                    }
                    self.connect_remote(&host);
                }
            }
            Key::Named(NamedKey::Backspace) => input.backspace(),
            _ => {
                if let Some(text) = event.text.as_deref()
                    && !ctrl
                    && !self.mods.alt_key()
                {
                    let text: String = text.chars().filter(|c| !c.is_control()).collect();
                    input.insert(&text);
                }
            }
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// At the end of the window: the local server keeps the layout, for the next window.
    pub(super) fn save_mux_layout(&mut self) {
        let has_local = self.running.as_ref().is_some_and(|r| {
            r.panes
                .values()
                .any(|p| p.mux.as_ref().is_some_and(MuxPane::is_local))
        });
        if !has_local {
            return;
        }
        let (Some(client), Some(session)) =
            (self.muxes.get(LOCAL).cloned(), self.capture_session())
        else {
            return;
        };
        if let Ok(value) = serde_json::to_value(&session) {
            client.set_layout(value);
        }
    }

    /// The window closes because the user closed its last tab: the mux panes of that tab end (the others go on).
    pub(super) fn end_mux_panes_on_exit(&mut self) {
        let ending = std::mem::take(&mut self.mux_end_on_exit);
        if ending.is_empty() {
            return;
        }
        for pane in &ending {
            self.close_mux_pane(Some(pane.clone()));
        }
        // An answer comes after the server did the closes before it: then the window may go.
        let servers: Vec<String> = ending.iter().map(|p| p.remote.clone()).collect();
        for remote in servers {
            if let Some(client) = self.muxes.get(&remote) {
                let _ = client.list();
            }
        }
        if let Some(running) = &mut self.running {
            for p in running.panes.values_mut() {
                if p.mux.as_ref().is_some_and(|m| ending.contains(m)) {
                    p.mux = None;
                }
            }
        }
    }

    /// `stop_mux`: asks first, with the number of panes that end.
    pub(super) fn ask_stop_mux(&mut self) {
        let Some(client) = self.mux_client(LOCAL, false) else {
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
                if let Some(client) = self.muxes.get(LOCAL).cloned() {
                    match client.stop() {
                        Ok(n) => {
                            // The server goes on purpose: no "the server stopped" warning after it.
                            self.muxes.remove(LOCAL);
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

    /// The user closed a mux pane: its program ends in its server too.
    pub(super) fn close_mux_pane(&self, pane: Option<MuxPane>) {
        if let Some(pane) = pane
            && let Some(client) = self.muxes.get(&pane.remote)
        {
            client.close(pane.id);
        }
    }

    /// The connection to a server ended: its panes got their end already (the client calls their exit).
    pub(super) fn mux_gone(&mut self, remote: &str) {
        if self.muxes.remove(remote).is_none() {
            return;
        }
        let (title, body) = if remote == LOCAL {
            (tr!("toast.mux_gone"), String::new())
        } else {
            (
                tr!("toast.remote_gone", remote = remote),
                tr!("toast.remote_gone_body", remote = remote),
            )
        };
        self.notify(None, &title, &body, Level::Warning, Source::App);
    }
}

/// Starts `fterm --mux-server` in the background (with no window and no console).
fn start_server() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    fterm_server::spawn_detached(&exe, &["--mux-server"])
}
