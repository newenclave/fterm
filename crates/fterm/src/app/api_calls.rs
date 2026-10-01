//! The API methods: the app side of `fterm-api` (Phase 7).

use fterm_api::RpcError;
use fterm_api::discovery::{Instance, instances_dir, register};
use fterm_api::server::{ClientId, Server};
use fterm_api::transport::socket_name;
use fterm_term::input::{lines_text, total_lines};
use serde_json::{Value, json};

use super::*;
use crate::api::{ApiRequest, Bridge, METHODS, bool_param, pane_param, place_param, str_param};

/// Who an API client is (from `hello`).
#[derive(Clone, Debug, Default)]
pub(super) struct ApiClient {
    pub name: String,
    /// The pane where the client runs (`FTERM_PANE_ID` of `ftermctl`).
    pub pane: Option<PaneId>,
}

/// The longest history that `get_text` gives.
const MAX_LINES: usize = 10_000;

fn not_found(what: &str) -> RpcError {
    RpcError::new(RpcError::NOT_FOUND, format!("no {what}"))
}

impl App {
    /// Starts the API server for this window and writes the instance file.
    pub(super) fn start_api(&mut self) {
        if !self.config.config.api.enabled {
            return;
        }
        let name = socket_name(std::process::id());
        match Server::start(&name, std::sync::Arc::new(Bridge::new(self.proxy.clone()))) {
            Ok(server) => {
                let instance = Instance {
                    pid: std::process::id(),
                    socket: name.clone(),
                    started: now_ms(),
                };
                match register(&instances_dir(), &instance) {
                    Ok(registration) => self.api_registration = Some(registration),
                    Err(err) => tracing::warn!("cannot write the instance file: {err}"),
                }
                tracing::info!(socket = %name, "API server started");
                self.api_server = Some(server);
            }
            Err(err) => tracing::warn!(socket = %name, "cannot start the API server: {err}"),
        }
    }

    /// The socket of this window (for `FTERM_SOCKET` in new panes).
    pub(super) fn api_socket(&self) -> Option<String> {
        self.api_server.as_ref().map(|s| s.name().to_owned())
    }

    /// Sends an event to the API clients that asked for it.
    pub(super) fn api_event(&self, event: &str, params: Value) {
        if let Some(server) = &self.api_server {
            server.broadcast(event, params);
        }
    }

    pub(super) fn api_call(&mut self, event_loop: &ActiveEventLoop, request: ApiRequest) {
        let name = self
            .api_clients
            .get(&request.client)
            .map(|c| c.name.as_str());
        tracing::debug!(client = request.client, ?name, method = %request.method, "API call");
        let result = if self.running.is_none() {
            Err(RpcError::new(
                RpcError::INTERNAL,
                "the window is not open yet",
            ))
        } else {
            self.api_dispatch(event_loop, request.client, &request.method, &request.params)
        };
        let _ = request.reply.send(result);
    }

    fn api_dispatch(
        &mut self,
        event_loop: &ActiveEventLoop,
        client: ClientId,
        method: &str,
        params: &Value,
    ) -> Result<Value, RpcError> {
        match method {
            "hello" => self.api_hello(client, params),
            "list" => Ok(self.api_list()),
            "spawn" => self.api_spawn(params),
            "send_text" => self.api_send_text(client, params),
            "get_text" => self.api_get_text(client, params),
            "focus" => {
                let pane = self.target_pane(client, params)?;
                self.go_to_pane(pane);
                Ok(json!({}))
            }
            "close" => self.api_close(event_loop, client, params),
            "zoom" => {
                let pane = self.target_pane(client, params)?;
                self.go_to_pane(pane);
                if let Some(running) = &mut self.running {
                    running.mux.toggle_zoom();
                }
                self.tab_changed();
                Ok(json!({}))
            }
            "set_title" => self.api_set_title(client, params),
            "notify" => {
                let title = str_param(params, "title")?.unwrap_or_default();
                let body = str_param(params, "body")?.unwrap_or_default();
                let level = match str_param(params, "level")? {
                    Some(name) => Level::from_name(&name)
                        .ok_or_else(|| RpcError::invalid_params(format!("no level `{name}`")))?,
                    None => Level::Info,
                };
                if title.is_empty() && body.is_empty() {
                    return Err(RpcError::invalid_params("give a `title` or a `body`"));
                }
                let pane = self.api_clients.get(&client).and_then(|c| c.pane);
                self.notify(pane, &title, &body, level, Source::Api);
                Ok(json!({}))
            }
            "panel" => {
                let running = self.running.as_mut().expect("checked in api_call");
                match str_param(params, "name")?.as_deref() {
                    None => running.dock.open = false,
                    Some(name) => {
                        let kind = PanelKind::from_name(name).ok_or_else(|| {
                            RpcError::invalid_params(format!("no panel `{name}`"))
                        })?;
                        running.dock.open = true;
                        running.dock.active = kind;
                    }
                }
                running.dock.focused = false;
                self.dock_changed();
                Ok(json!({}))
            }
            other => Err(RpcError::no_method(other)),
        }
    }

    fn api_hello(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let name = str_param(params, "name")?.unwrap_or_else(|| "client".to_owned());
        let pane = pane_param(params, "pane")?.map(PaneId).filter(|p| {
            self.running
                .as_ref()
                .is_some_and(|r| r.panes.contains_key(p))
        });
        self.api_clients.insert(client, ApiClient { name, pane });
        Ok(json!({
            "api_version": fterm_api::API_VERSION,
            "fterm": env!("CARGO_PKG_VERSION"),
            "pid": std::process::id(),
            "methods": METHODS,
            "events": crate::api::EVENTS,
            "pane": pane.map(|p| p.0),
        }))
    }

    /// The pane of the request: `pane` in the params, else the client's own pane, else the active pane.
    fn target_pane(&self, client: ClientId, params: &Value) -> Result<PaneId, RpcError> {
        let running = self.running.as_ref().expect("checked in api_call");
        let pane = match pane_param(params, "pane")? {
            Some(id) => PaneId(id),
            None => self
                .api_clients
                .get(&client)
                .and_then(|c| c.pane)
                .or_else(|| running.mux.active_pane())
                .ok_or_else(|| not_found("pane"))?,
        };
        if running.panes.contains_key(&pane) {
            Ok(pane)
        } else {
            Err(not_found(&format!("pane {}", pane.0)))
        }
    }

    pub(super) fn pane_json(&self, pane: PaneId, tab: usize) -> Value {
        let running = self.running.as_ref().expect("checked in api_call");
        let Some(p) = running.panes.get(&pane) else {
            return Value::Null;
        };
        let size = p.session.grid_size();
        json!({
            "id": pane.0,
            "tab": tab,
            "program": display_name(p.session.program()),
            "title": p.app_title,
            "cwd": p.shell.cwd,
            "active": running.mux.active_pane() == Some(pane),
            "running": p.shell.is_running(),
            "at_prompt": p.shell.at_prompt(),
            "agent": p.agent.as_ref().map(|a| json!({ "state": a.kind.name(), "message": a.message })),
            "last_command": p.last_command.as_ref().map(|c| json!({
                "command": c.command,
                "exit": c.exit,
                "took_ms": c.took_ms,
            })),
            "size": { "columns": size.columns, "rows": size.rows },
        })
    }

    fn api_list(&self) -> Value {
        let running = self.running.as_ref().expect("checked in api_call");
        let titles = running.tab_titles();
        let tabs: Vec<Value> = running
            .mux
            .tabs()
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let panes: Vec<Value> = tab
                    .layout
                    .panes()
                    .into_iter()
                    .map(|pane| self.pane_json(pane, i + 1))
                    .collect();
                json!({
                    "tab": i + 1,
                    "id": tab.id.0,
                    "title": titles.get(i),
                    "active": i == running.mux.active_index(),
                    "zoomed": tab.zoomed.is_some(),
                    "panes": panes,
                })
            })
            .collect();
        json!({
            "pid": std::process::id(),
            "active_pane": running.mux.active_pane().map(|p| p.0),
            "tabs": tabs,
        })
    }

    fn api_spawn(&mut self, params: &Value) -> Result<Value, RpcError> {
        let place = place_param(params)?;
        let profile = str_param(params, "profile")?;
        if let Some(cwd) = str_param(params, "cwd")? {
            self.spawn_cwd = Some(cwd);
        }
        // A split goes next to this pane (else next to the active one).
        if let Some(next_to) = pane_param(params, "pane")?.map(PaneId) {
            let running = self.running.as_mut().expect("checked in api_call");
            if !running.panes.contains_key(&next_to) {
                return Err(not_found(&format!("pane {}", next_to.0)));
            }
            running.mux.focus(next_to);
        }
        let result = match place {
            SpawnWhere::Tab => self.new_tab(profile.as_deref()).map(|_| ()),
            SpawnWhere::SplitRight => self.split(Direction::Right, profile.as_deref()),
            SpawnWhere::SplitDown => self.split(Direction::Down, profile.as_deref()),
        };
        self.spawn_cwd = None;
        result.map_err(|err| RpcError::new(RpcError::INTERNAL, format!("{err:#}")))?;
        let pane = self.running.as_ref().and_then(|r| r.mux.active_pane());
        Ok(json!({ "pane": pane.map(|p| p.0) }))
    }

    fn api_send_text(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let pane = self.target_pane(client, params)?;
        let text = str_param(params, "text")?.unwrap_or_default();
        let enter = bool_param(params, "enter")?.unwrap_or(false);
        let running = self.running.as_ref().expect("checked in api_call");
        let session = &running.panes[&pane].session;
        let mut bytes = text.into_bytes();
        if enter {
            bytes.push(b'\r');
        }
        session.with_term_mut(|term, _| term.scroll_display(Scroll::Bottom));
        session.write(bytes);
        Ok(json!({}))
    }

    fn api_get_text(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let pane = self.target_pane(client, params)?;
        let what = str_param(params, "what")?.unwrap_or_else(|| "screen".to_owned());
        let lines = params
            .get("lines")
            .and_then(Value::as_u64)
            .map_or(200, |n| n as usize)
            .min(MAX_LINES);
        let running = self.running.as_ref().expect("checked in api_call");
        let p = &running.panes[&pane];
        let session = &p.session;
        let text = match what.as_str() {
            "screen" => session.screen_text(),
            "last_output" => {
                let range = session.with_term(|term| {
                    let total = total_lines(term);
                    p.shell.last_output(total).map(|(start, end)| {
                        lines_text(term, start.max(end.saturating_sub(lines)), end)
                    })
                });
                let Some(text) = range else {
                    return Err(RpcError::new(
                        RpcError::NOT_FOUND,
                        "no command output yet (it needs shell integration)",
                    ));
                };
                let last = p.last_command.as_ref();
                return Ok(json!({
                    "pane": pane.0,
                    "text": text.trim_end(),
                    "running": p.shell.is_running(),
                    "command": last.and_then(|c| c.command.clone()),
                    "exit": last.and_then(|c| c.exit),
                }));
            }
            "history" => session.with_term(|term| {
                let total = total_lines(term);
                lines_text(term, total.saturating_sub(lines), total)
            }),
            other => {
                return Err(RpcError::invalid_params(format!(
                    "`what` must be \"screen\", \"history\", or \"last_output\", got `{other}`"
                )));
            }
        };
        Ok(json!({ "pane": pane.0, "text": text.trim_end() }))
    }

    fn api_close(
        &mut self,
        event_loop: &ActiveEventLoop,
        client: ClientId,
        params: &Value,
    ) -> Result<Value, RpcError> {
        let pane = self.target_pane(client, params)?;
        let force = bool_param(params, "force")?.unwrap_or(false);
        let running = self.running.as_ref().expect("checked in api_call");
        let programs = Self::pane_programs(running, pane);
        if !programs.is_empty() && !force {
            return Err(RpcError::new(
                RpcError::DENIED,
                format!(
                    "{} runs in this pane; use force = true to close it",
                    programs.join(", ")
                ),
            ));
        }
        if !self.close_pane_now(pane) {
            event_loop.exit();
        }
        Ok(json!({}))
    }

    fn api_set_title(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let pane = self.target_pane(client, params)?;
        let title = str_param(params, "title")?.unwrap_or_default();
        let running = self.running.as_mut().expect("checked in api_call");
        let tab = running
            .mux
            .tabs()
            .iter()
            .find(|t| t.layout.contains(pane))
            .map(|t| t.id)
            .ok_or_else(|| not_found("tab"))?;
        running.mux.rename(tab, &title);
        running.window.request_redraw();
        self.update_window_title();
        Ok(json!({}))
    }
}
