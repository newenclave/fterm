//! The API methods: the app side of `fterm-api` (Phase 7).

use fterm_api::RpcError;
use fterm_api::discovery::{Instance, instances_dir, register};
use fterm_api::server::{ClientId, Server};
use fterm_api::transport::socket_name;
use fterm_term::input::{lines_text, total_lines};
use serde_json::{Value, json};

use super::*;
use crate::access::{Check, Verdict, check};
use crate::api::{ApiRequest, Bridge, METHODS, bool_param, pane_param, place_param, str_param};
use crate::inbox::MAX_MESSAGE;
use crate::waits::{Happening, WaitKind, matches, parse_kind};

/// Who an API client is (from `hello`).
#[derive(Clone, Debug, Default)]
pub(super) struct ApiClient {
    pub name: String,
    /// The pane where the client runs (`FTERM_PANE_ID` of `ftermctl`).
    pub pane: Option<PaneId>,
    /// What the user said: may it use other panes (`None` = not asked yet).
    pub said: Option<bool>,
}

/// A `wait_for` that waits.
pub(super) struct Wait {
    pub client: ClientId,
    pub pane: PaneId,
    pub kind: WaitKind,
    pub deadline: Instant,
    pub reply: std::sync::mpsc::Sender<Result<Value, RpcError>>,
}

/// Requests of one client that wait for the user's answer (may it use other panes?).
pub(super) struct ApiAsk {
    pub client: ClientId,
    pub requests: Vec<ApiRequest>,
}

/// The answer to the access question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AccessAnswer {
    Allow,
    Always,
    Deny,
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
        if self.running.is_none() {
            let _ = request.reply.send(Err(RpcError::new(
                RpcError::INTERNAL,
                "the window is not open yet",
            )));
            return;
        }
        match self.api_access(&request) {
            Verdict::Allow => {}
            Verdict::Deny(why) => {
                let _ = request
                    .reply
                    .send(Err(RpcError::new(RpcError::DENIED, why)));
                return;
            }
            Verdict::Ask => return self.ask_access(request),
        }
        if request.method == "wait_for" {
            return self.api_wait_for(request);
        }
        let result =
            self.api_dispatch(event_loop, request.client, &request.method, &request.params);
        let _ = request.reply.send(result);
    }

    /// May this request run now? Reading or typing into a pane that is not the client's own needs a yes.
    fn api_access(&self, request: &ApiRequest) -> Verdict {
        let gated = matches!(
            request.method.as_str(),
            "send_text" | "get_text" | "close" | "wait_for" | "read_messages" | "spawn"
        );
        if !gated {
            return Verdict::Allow;
        }
        let client = self.api_clients.get(&request.client);
        let running = self.running.as_ref().expect("checked in api_call");
        let (own, remote) = if request.method == "spawn" {
            // A new pane is new power: it needs the same yes as a pane of somebody else.
            (false, true)
        } else {
            match self.target_pane(request.client, &request.params) {
                // A bad pane id: let the method give the error.
                Err(_) => return Verdict::Allow,
                Ok(pane) => {
                    let p = &running.panes[&pane];
                    let own = client.and_then(|c| c.pane) == Some(pane)
                        || p.opened_by == Some(request.client);
                    (own, p.remote)
                }
            }
        };
        let name = client.map_or("client", |c| c.name.as_str());
        check(&Check {
            own,
            remote,
            said: client.and_then(|c| c.said),
            name,
            always: &self.api_always,
            ask: self.config.config.api.ask,
        })
    }

    /// Keeps the request and asks the user (one question for each client).
    fn ask_access(&mut self, request: ApiRequest) {
        if let Some(ask) = self
            .api_questions
            .iter_mut()
            .find(|a| a.client == request.client)
        {
            ask.requests.push(request);
            return;
        }
        self.api_questions.push_back(ApiAsk {
            client: request.client,
            requests: vec![request],
        });
        if let Some(running) = &self.running {
            running.window.request_redraw();
            running
                .window
                .request_user_attention(Some(winit::window::UserAttentionType::Informational));
        }
    }

    /// The lines of the access question that is open now.
    pub(super) fn access_question_lines(&self) -> Option<Vec<String>> {
        let ask = self.api_questions.front()?;
        let client = self.api_clients.get(&ask.client);
        let name = client.map_or("A program", |c| c.name.as_str());
        let tab = client.and_then(|c| c.pane).and_then(|pane| {
            let running = self.running.as_ref()?;
            running
                .mux
                .tabs()
                .iter()
                .position(|t| t.layout.contains(pane))
                .map(|i| i + 1)
        });
        Some(crate::access::question(name, tab))
    }

    /// The user answered the access question: run or refuse the requests that waited.
    pub(super) fn answer_access(&mut self, event_loop: &ActiveEventLoop, answer: AccessAnswer) {
        let Some(ask) = self.api_questions.pop_front() else {
            return;
        };
        if let Some(client) = self.api_clients.get_mut(&ask.client) {
            client.said = Some(answer != AccessAnswer::Deny);
            if answer == AccessAnswer::Always {
                self.api_always.insert(client.name.clone());
            }
        }
        for request in ask.requests {
            self.api_call(event_loop, request);
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// A client went away: forget it, its waits, and its questions.
    pub(super) fn api_client_gone(&mut self, client: ClientId) {
        self.api_clients.remove(&client);
        self.waits.retain(|w| w.client != client);
        self.api_questions.retain(|a| a.client != client);
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    fn api_wait_for(&mut self, request: ApiRequest) {
        let result = (|| {
            let pane = self.target_pane(request.client, &request.params)?;
            let event = str_param(&request.params, "event")?
                .ok_or_else(|| RpcError::invalid_params("give an `event`"))?;
            let kind = parse_kind(&event, str_param(&request.params, "pattern")?)
                .map_err(RpcError::invalid_params)?;
            let ms = request
                .params
                .get("timeout_ms")
                .and_then(Value::as_f64)
                .unwrap_or(30_000.0)
                .clamp(0.0, 3_600_000.0);
            Ok((
                pane,
                kind,
                Instant::now() + Duration::from_millis(ms as u64),
            ))
        })();
        let (pane, kind, deadline) = match result {
            Ok(wait) => wait,
            Err(err) => {
                let _ = request.reply.send(Err(err));
                return;
            }
        };
        // A text that is on the screen already: no need to wait.
        if let WaitKind::Text(pattern) = &kind {
            let running = self.running.as_ref().expect("checked in api_call");
            let screen = running.panes[&pane].session.screen_text();
            if screen.contains(pattern.as_str()) {
                let _ = request
                    .reply
                    .send(Ok(json!({ "event": "text", "pane": pane.0 })));
                return;
            }
        }
        self.waits.push(Wait {
            client: request.client,
            pane,
            kind,
            deadline,
            reply: request.reply,
        });
    }

    /// Something happened in a pane: answer the waits that waited for it.
    pub(super) fn resolve_waits(&mut self, pane: PaneId, happening: Happening, data: Value) {
        if self.waits.is_empty() {
            return;
        }
        let (done, waiting): (Vec<Wait>, Vec<Wait>) = std::mem::take(&mut self.waits)
            .into_iter()
            .partition(|w| w.pane == pane && matches(&w.kind, happening));
        self.waits = waiting;
        for wait in done {
            let _ = wait.reply.send(Ok(data.clone()));
        }
    }

    /// The screen of a pane changed: check the waits for a text.
    pub(super) fn check_text_waits(&mut self, pane: PaneId) {
        if !self
            .waits
            .iter()
            .any(|w| w.pane == pane && matches!(w.kind, WaitKind::Text(_)))
        {
            return;
        }
        let Some(screen) = self
            .running
            .as_ref()
            .and_then(|r| r.panes.get(&pane))
            .map(|p| p.session.screen_text())
        else {
            return;
        };
        self.resolve_waits(
            pane,
            Happening::Screen(&screen),
            json!({ "event": "text", "pane": pane.0 }),
        );
    }

    /// Waits whose time is over get a timeout. Returns when the next one ends.
    pub(super) fn expire_waits(&mut self, now: Instant) -> Option<Instant> {
        let (over, waiting): (Vec<Wait>, Vec<Wait>) = std::mem::take(&mut self.waits)
            .into_iter()
            .partition(|w| w.deadline <= now);
        self.waits = waiting;
        for wait in over {
            let _ = wait
                .reply
                .send(Err(RpcError::new(RpcError::TIMEOUT, "the time is over")));
        }
        self.waits.iter().map(|w| w.deadline).min()
    }

    /// A pane closed: its inbox goes, and its waits get an error.
    pub(super) fn api_pane_closed(&mut self, pane: PaneId) {
        self.inbox.remove(pane);
        let (gone, waiting): (Vec<Wait>, Vec<Wait>) = std::mem::take(&mut self.waits)
            .into_iter()
            .partition(|w| w.pane == pane);
        self.waits = waiting;
        for wait in gone {
            let _ = wait
                .reply
                .send(Err(RpcError::new(RpcError::NOT_FOUND, "the pane closed")));
        }
        self.api_event("pane_closed", json!({ "pane": pane.0 }));
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
            "spawn" => self.api_spawn(client, params),
            "scene_open" => self.api_scene_open(client, params),
            "scene_draw" => self.api_scene_draw(client, params),
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
            "send_message" => self.api_send_message(client, params),
            "read_messages" => {
                let pane = self.target_pane(client, params)?;
                let unread_only = bool_param(params, "unread_only")?.unwrap_or(true);
                let mark_read = bool_param(params, "mark_read")?.unwrap_or(true);
                let messages: Vec<Value> = self
                    .inbox
                    .read(pane, unread_only, mark_read)
                    .into_iter()
                    .map(|m| {
                        json!({
                            "id": m.id,
                            "from": m.from.map(|p| p.0),
                            "from_name": m.from_name,
                            "text": m.text,
                            "time": m.time,
                            "read": m.read,
                        })
                    })
                    .collect();
                if let Some(running) = &self.running {
                    running.window.request_redraw();
                }
                Ok(json!({ "pane": pane.0, "messages": messages }))
            }
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
        self.api_clients.insert(
            client,
            ApiClient {
                name,
                pane,
                said: None,
            },
        );
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
            "remote": p.remote,
            "messages": self.inbox.unread(pane),
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

    fn api_spawn(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
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
        if let Some(p) = pane.and_then(|id| self.running.as_mut()?.panes.get_mut(&id)) {
            p.opened_by = Some(client);
        }
        Ok(json!({ "pane": pane.map(|p| p.0) }))
    }

    /// `scene_open`: a Braille scene next to a pane (Phase 11). The client may draw in it at once.
    fn api_scene_open(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let direction = crate::api::scene_place(params)?;
        if let Some(next_to) = pane_param(params, "pane")?.map(PaneId) {
            let running = self.running.as_mut().expect("checked in api_call");
            if !running.panes.contains_key(&next_to) {
                return Err(not_found(&format!("pane {}", next_to.0)));
            }
            running.mux.focus(next_to);
        }
        self.split_with(direction, |app, size| Ok(app.spawn_scene(size)))
            .map_err(|err| RpcError::new(RpcError::INTERNAL, format!("{err:#}")))?;
        let pane = self
            .running
            .as_ref()
            .and_then(|r| r.mux.active_pane())
            .ok_or_else(|| RpcError::new(RpcError::INTERNAL, "no pane"))?;
        if let Some(p) = self.running.as_mut().and_then(|r| r.panes.get_mut(&pane)) {
            p.opened_by = Some(client);
        }
        Ok(self.scene_size(pane))
    }

    /// `scene_draw`: drawing commands for a scene pane (see `fterm_scene::Op`).
    fn api_scene_draw(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let pane = self.target_pane(client, params)?;
        let ops = params
            .get("ops")
            .ok_or_else(|| RpcError::invalid_params("`ops` is missing: a command or a list"))?;
        let ops = fterm_scene::parse_ops(ops).map_err(RpcError::invalid_params)?;
        let running = self.running.as_ref().expect("checked in api_call");
        let p = &running.panes[&pane];
        let Some(scene) = &p.scene else {
            return Err(RpcError::invalid_params(format!(
                "pane {} is not a scene (open one with scene_open)",
                pane.0
            )));
        };
        {
            let mut canvas = scene.lock().unwrap();
            fterm_scene::apply(&mut canvas, &ops).map_err(RpcError::invalid_params)?;
            p.session.feed(&fterm_scene::render(&canvas));
        }
        running.window.request_redraw();
        Ok(self.scene_size(pane))
    }

    /// The pane and the size of its scene, in cells and in dots.
    fn scene_size(&self, pane: PaneId) -> Value {
        let size = self
            .running
            .as_ref()
            .and_then(|r| r.panes.get(&pane))
            .and_then(|p| p.scene.as_ref())
            .map(|s| {
                let c = s.lock().unwrap();
                (c.cols(), c.rows(), c.width(), c.height(), c.aspect())
            })
            .unwrap_or((0, 0, 0, 0, 1.0));
        json!({
            "pane": pane.0,
            "cols": size.0,
            "rows": size.1,
            "width": size.2,
            "height": size.3,
            "aspect": size.4,
        })
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

impl App {
    fn api_send_message(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let to = pane_param(params, "to")?
            .map(PaneId)
            .ok_or_else(|| RpcError::invalid_params("give `to` (a pane id)"))?;
        let text = str_param(params, "text")?.unwrap_or_default();
        if text.trim().is_empty() {
            return Err(RpcError::invalid_params("the message is empty"));
        }
        if text.len() > MAX_MESSAGE {
            return Err(RpcError::invalid_params(format!(
                "the message is longer than {MAX_MESSAGE} bytes"
            )));
        }
        let running = self.running.as_ref().expect("checked in api_call");
        if !running.panes.contains_key(&to) {
            return Err(not_found(&format!("pane {}", to.0)));
        }
        let sender = self.api_clients.get(&client).cloned().unwrap_or_default();
        let from_name = if sender.name.is_empty() {
            "a program".to_owned()
        } else {
            sender.name.clone()
        };
        let id = self
            .inbox
            .send(to, sender.pane, &from_name, &text, now_ms());
        // The user sees it too (a toast and the count in the Agents panel).
        let tab = running
            .mux
            .tabs()
            .iter()
            .position(|t| t.layout.contains(to))
            .and_then(|i| running.tab_titles().get(i).cloned())
            .unwrap_or_default();
        let first_line: String = text
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(80)
            .collect();
        self.notify(
            Some(to),
            &format!("Message for {tab}"),
            &format!("{from_name}: {first_line}"),
            Level::Info,
            Source::Api,
        );
        let data = json!({
            "event": "message",
            "id": id,
            "to": to.0,
            "from": sender.pane.map(|p| p.0),
            "from_name": from_name,
            "text": text,
        });
        self.api_event("message", data.clone());
        self.resolve_waits(to, Happening::Message, data);
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
        Ok(json!({ "id": id }))
    }
}
