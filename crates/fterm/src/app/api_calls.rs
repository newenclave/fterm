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
#[derive(Clone, Debug)]
pub(super) struct ApiClient {
    pub name: String,
    /// The pane where the client runs, as its processes show it (`None`: not in a pane of this window, or not known).
    pub pane: Option<PaneId>,
    /// The pane that the client says it runs in (`FTERM_PANE_ID`): only the default target of a call that asks.
    pub hint: Option<PaneId>,
    /// The program of its pane that the client belongs to (for "Always").
    pub program: Option<crate::identity::Program>,
    /// What the user said: may it use other panes (`None` = not asked yet).
    pub said: Option<bool>,
    /// A `hello` came: the name and the pane stay (a pane may come later, but never change).
    pub hello: bool,
    /// An event hook of the config, not a program.
    pub hook: bool,
}

/// The event hooks of the config whose calls can act on panes.
const HOOKS: [&str; 2] = ["on_agent", "on_agent_event"];

/// How long the question of a hook stays (no call waits for it).
const HOOK_QUESTION: Duration = Duration::from_secs(120);

/// The client id of an event hook of the config: far above the ids of the server, which count from 0.
pub(super) fn hook_client(hook: &str) -> ClientId {
    let i = HOOKS.iter().position(|h| *h == hook).unwrap_or(HOOKS.len());
    u64::MAX - i as u64
}

impl Default for ApiClient {
    fn default() -> Self {
        Self {
            name: "client".to_owned(),
            pane: None,
            hint: None,
            program: None,
            said: None,
            hello: false,
            hook: false,
        }
    }
}

impl ApiClient {
    /// The record of an event hook of the config.
    pub fn for_hook(hook: &str) -> Self {
        Self {
            name: hook.to_owned(),
            hello: true,
            hook: true,
            ..Self::default()
        }
    }

    /// The key of an "Always" for this client: its checked pane and its program there.
    pub fn always_key(&self) -> Option<crate::access::AlwaysKey> {
        let program = self.program.as_ref()?;
        Some(crate::access::AlwaysKey {
            pane: self.pane?.0,
            pid: program.pid,
            start: program.start,
        })
    }

    /// Who a "No" of the user counts for: the checked pane, or all clients with no pane.
    pub fn bucket(&self) -> crate::access::Bucket {
        if self.hook {
            return crate::access::Bucket::Hooks;
        }
        self.pane.map_or(crate::access::Bucket::NoPane, |p| {
            crate::access::Bucket::Pane(p.0)
        })
    }

    /// A `hello`: the first one gives the name, the hint, and the pane that the processes show. A later one may
    /// only add a pane that was not known (a mux pane whose shell came late); it never changes the pane or the
    /// name, and it keeps the answer of the user.
    pub fn hello(
        &mut self,
        name: String,
        hint: Option<PaneId>,
        checked: Option<(PaneId, crate::identity::Program)>,
    ) {
        if !self.hello {
            self.hello = true;
            self.name = name;
            self.hint = hint;
        } else if self.pane.is_some() {
            return;
        }
        if let Some((pane, program)) = checked {
            self.pane = Some(pane);
            self.program = Some(program);
        }
    }
}

/// A `wait_for` that waits.
pub(super) struct Wait {
    pub client: ClientId,
    pub pane: PaneId,
    pub kind: WaitKind,
    pub deadline: Instant,
    pub reply: std::sync::mpsc::Sender<Result<Value, RpcError>>,
}

/// An `ai_ask` that waits for the end of the answer.
pub(super) struct AiWait {
    pub client: ClientId,
    /// The request id of the AI chat.
    pub id: u64,
    pub deadline: Instant,
    pub reply: std::sync::mpsc::Sender<Result<Value, RpcError>>,
}

/// A `screenshot` that waits for the next frame.
pub(super) struct PendingShot {
    pub pane: PaneId,
    pub path: std::path::PathBuf,
    pub reply: std::sync::mpsc::Sender<Result<Value, RpcError>>,
}

/// Requests of one client that wait for the user's answer (may it use other panes?).
pub(super) struct ApiAsk {
    /// A number of its own: answer mode is for one question.
    pub id: u64,
    pub client: ClientId,
    pub requests: Vec<ApiRequest>,
    /// The question of an event hook: no call waits for it (the answer counts from the next event); it goes then.
    pub until: Option<Instant>,
}

/// The error of a call that the user said no to a moment ago (an agent should ask the user, not try again).
const REFUSED: &str =
    "the user said no to this pane a moment ago: ask the user before you try again";

/// The key that starts answering the access question: Ctrl+Shift+Enter.
pub(super) fn is_answer_chord(key: &Key, ctrl: bool, shift: bool, alt: bool) -> bool {
    ctrl && shift && !alt && matches!(key, Key::Named(NamedKey::Enter))
}

/// A key while the access question is answered: digits by the place of the key (any layout), letters by the
/// letter of the layout; a modifier alone or a repeat does nothing; with Ctrl or Alt it is another key.
pub(super) fn answer_key(
    key: &Key,
    physical: PhysicalKey,
    repeat: bool,
    ctrl: bool,
    alt: bool,
) -> crate::access::AnswerKey {
    use crate::access::AnswerKey as A;
    let modifier_only = matches!(
        key,
        Key::Named(
            NamedKey::Shift
                | NamedKey::Control
                | NamedKey::Alt
                | NamedKey::AltGraph
                | NamedKey::Super
                | NamedKey::Meta
                | NamedKey::Hyper
                | NamedKey::Fn
                | NamedKey::CapsLock
                | NamedKey::NumLock
        )
    );
    if repeat || modifier_only {
        return A::Ignore;
    }
    if ctrl || alt {
        return A::Other;
    }
    match physical {
        PhysicalKey::Code(KeyCode::Digit1 | KeyCode::Numpad1) => return A::Allow,
        PhysicalKey::Code(KeyCode::Digit2 | KeyCode::Numpad2) => return A::Always,
        PhysicalKey::Code(KeyCode::Digit3 | KeyCode::Numpad3) => return A::No,
        _ => {}
    }
    match key {
        Key::Named(NamedKey::Enter) => A::Confirm,
        Key::Named(NamedKey::Escape) => A::No,
        Key::Character(c) => match c.to_lowercase().as_str() {
            "y" => A::Allow,
            "a" => A::Always,
            "n" => A::No,
            _ => A::Other,
        },
        _ => A::Other,
    }
}

/// The access question as it is drawn now.
pub(super) struct AccessBox {
    /// Changes when the question in front changes, or what fterm says of its client: answer mode ends, and a
    /// click waits again (a button must not move under the mouse).
    pub key: u64,
    pub lines: crate::access::QuestionLines,
    pub buttons: Vec<(crate::access::Button, String)>,
    /// "Always" is offered (the client's pane is checked).
    pub always_ok: bool,
    pub answering: bool,
    /// The warning about a guard of the config (not an access question).
    pub warning: bool,
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
        if request.method == "spawn"
            && request.params.get("place").and_then(Value::as_str) == Some("overlay")
            && request.params.get("wait").and_then(Value::as_bool) == Some(true)
        {
            return self.api_overlay_wait(request);
        }
        if request.method == "screenshot" {
            return self.api_screenshot(request);
        }
        if request.method == "ai_ask" {
            return self.api_ai_ask(request);
        }
        if request.method == "review" {
            return self.api_review(event_loop, request);
        }
        if request.method == "form" {
            return self.api_form(event_loop, request);
        }
        if request.method == "show_image" && crate::api::picture_wait(&request.params) {
            return self.api_show_image_wait(request);
        }
        if request.method == "hello" {
            let result = self.api_hello(request.client, &request.params, request.chain.as_deref());
            let _ = request.reply.send(result);
            return;
        }
        let result =
            self.api_dispatch(event_loop, request.client, &request.method, &request.params);
        let _ = request.reply.send(result);
    }

    /// May this request run now? Reading or typing into a pane that is not the client's own needs a yes.
    fn api_access(&self, request: &ApiRequest) -> Verdict {
        if request.method == "ai_ask" && !self.config.config.ai.api_access {
            return Verdict::Deny(
                "questions to the AI panel use the user's key: set ai = { api_access = true } in fterm.lua",
            );
        }
        if !crate::access::gated(&request.method, &request.params) {
            return Verdict::Allow;
        }
        let client = self.api_clients.get(&request.client);
        let running = self.running.as_ref().expect("checked in api_call");
        let (own, remote) = if request.method == "spawn" || request.method.starts_with("ai_") {
            // A new pane is new power, and the AI chat has the text of the panes:
            // they need the same yes as a pane of somebody else.
            (false, true)
        } else {
            match self.target_pane(request.client, &request.params) {
                // A bad pane id: let the method give the error.
                Err(_) => return Verdict::Allow,
                Ok(pane) => {
                    let p = &running.panes[&pane];
                    let own = crate::access::owns(
                        &request.method,
                        client.and_then(|c| c.pane) == Some(pane),
                        p.opened_by == Some(request.client),
                    );
                    (own, p.remote)
                }
            }
        };
        let bucket = client.map_or(crate::access::Bucket::NoPane, ApiClient::bucket);
        check(&Check {
            own,
            remote,
            said: client.and_then(|c| c.said),
            always: self
                .api_always
                .allows(client.and_then(ApiClient::always_key)),
            refused: self.api_refused.refused(bucket, Instant::now()),
            ask: self.config.config.api.ask,
            may_ask: crate::access::may_ask(&request.method),
        })
    }

    /// Questions whose calls no longer wait (their clients gave up): they go. Gives when to look again.
    pub(super) fn expire_api_questions(&mut self, now: Instant) -> Option<Instant> {
        let before = self.api_questions.len();
        self.api_questions.retain(|q| {
            q.until.is_some_and(|until| now < until)
                || q.requests
                    .iter()
                    .any(|r| crate::api::still_waits(r.asked, &r.method, &r.params, now))
        });
        if self.api_questions.len() != before
            && let Some(running) = &self.running
        {
            running.window.request_redraw();
        }
        let calls = self
            .api_questions
            .iter()
            .flat_map(|q| &q.requests)
            .map(|r| r.asked + crate::api::timeout_for(&r.method, &r.params));
        let hooks = self.api_questions.iter().filter_map(|q| q.until);
        calls.chain(hooks).min()
    }

    /// Calls of a Lua event hook (`on_agent`, `on_agent_event`). It runs on events that programs make (an agent, or
    /// any program that prints an OSC), not on a key of the user: calls that act on panes need a yes, as an API
    /// client with no pane. Before the yes they do not run (later, the pane in front could be another one); the yes
    /// counts for the next events, until the config changes. A toast, a tab color, or the clipboard always run.
    pub(super) fn run_hook_calls(
        &mut self,
        event_loop: &ActiveEventLoop,
        hook: &str,
        calls: Vec<ApiCall>,
    ) {
        let gated = calls.iter().any(crate::access::hook_call_gated);
        let verdict = if gated {
            self.hook_access(hook, &calls)
        } else {
            Verdict::Allow
        };
        match verdict {
            Verdict::Allow => {}
            Verdict::Ask => self.ask_hook(hook),
            Verdict::Deny(why) => tracing::info!(hook, why, "calls of a hook do not run"),
        }
        for call in calls {
            if verdict == Verdict::Allow || !crate::access::hook_call_gated(&call) {
                self.run_api_call(event_loop, call);
            }
        }
    }

    fn hook_access(&self, hook: &str, calls: &[ApiCall]) -> Verdict {
        let record = self.api_clients.get(&hook_client(hook));
        // Typing goes into the pane in front: a pane with no remote control takes none.
        let types = calls
            .iter()
            .any(|c| matches!(c, ApiCall::SendText(_) | ApiCall::CopyMode(_)));
        let remote = !types
            || self.running.as_ref().is_none_or(|r| {
                r.mux
                    .active_pane()
                    .and_then(|pane| r.panes.get(&pane))
                    .is_none_or(|p| p.remote)
            });
        check(&Check {
            own: false,
            remote,
            said: record.and_then(|c| c.said),
            always: false,
            refused: self
                .api_refused
                .refused(crate::access::Bucket::Hooks, Instant::now()),
            ask: self.config.config.api.ask,
            may_ask: true,
        })
    }

    /// The question for a hook (one at a time).
    fn ask_hook(&mut self, hook: &str) {
        let client = hook_client(hook);
        self.api_clients
            .entry(client)
            .or_insert_with(|| ApiClient::for_hook(hook));
        if self.api_questions.iter().any(|q| q.client == client) {
            return;
        }
        self.next_question += 1;
        self.api_questions.push_back(ApiAsk {
            id: self.next_question,
            client,
            requests: Vec::new(),
            until: Some(Instant::now() + HOOK_QUESTION),
        });
        if let Some(running) = &self.running {
            running.window.request_redraw();
            running
                .window
                .request_user_attention(Some(winit::window::UserAttentionType::Informational));
        }
    }

    /// The config changed: what the user said for its hooks counts no more (it is new code).
    pub(super) fn forget_hooks(&mut self) {
        self.api_clients.retain(|_, c| !c.hook);
        self.api_questions.retain(|q| q.until.is_none());
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
        self.next_question += 1;
        self.api_questions.push_back(ApiAsk {
            id: self.next_question,
            client: request.client,
            requests: vec![request],
            until: None,
        });
        if let Some(running) = &self.running {
            running.window.request_redraw();
            running
                .window
                .request_user_attention(Some(winit::window::UserAttentionType::Informational));
        }
    }

    /// The access question in front, as it is drawn now. The warning about a guard comes first.
    pub(super) fn access_box(&self, now: Instant) -> Option<AccessBox> {
        use std::hash::{Hash, Hasher};
        if let Some(config) = self.guard_warning {
            let answering = self.answer_mode.active(now);
            let (lines, buttons) =
                crate::guards::warning_box(self.guard.confirmed, config, answering);
            let mut hasher = std::hash::DefaultHasher::new();
            ("guard", config).hash(&mut hasher);
            return Some(AccessBox {
                key: hasher.finish(),
                lines,
                buttons,
                always_ok: false,
                answering,
                warning: true,
            });
        }
        let ask = self.api_questions.front()?;
        let client = self.api_clients.get(&ask.client);
        let name = client.map_or("client", |c| c.name.as_str());
        // What fterm checked: the tab of the client's pane and its program.
        let hook = client.is_some_and(|c| c.hook);
        let place = client
            .filter(|_| !hook)
            .and_then(|c| {
                let program = c.program.as_ref()?;
                let (tab, _) = self.running.as_ref()?.pane_tab_number(c.pane?)?;
                Some(crate::access::Place::Pane {
                    tab,
                    program: program.name.as_str(),
                })
            })
            .unwrap_or(if hook {
                crate::access::Place::Config
            } else {
                crate::access::Place::Outside
            });
        let answering = self.answer_mode.active(now);
        let (lines, buttons) = crate::access::question_box(
            &crate::access::Asker { name, place },
            answering,
            answering && self.answer_mode.always_chosen(),
        );
        let mut hasher = std::hash::DefaultHasher::new();
        (ask.id, name, place).hash(&mut hasher);
        Some(AccessBox {
            key: hasher.finish(),
            lines,
            buttons,
            always_ok: matches!(place, crate::access::Place::Pane { .. }),
            answering,
            warning: false,
        })
    }

    /// The config changed (or fterm starts): a looser guard keeps the confirmed value and waits for a yes.
    pub(super) fn apply_guards(&mut self) {
        let config = crate::guards::Guards {
            ask: self.config.config.api.ask,
            ai_access: self.config.config.ai.api_access,
        };
        let decision = crate::guards::decide(&self.guard, config);
        if decision.state != self.guard {
            self.guard = decision.state;
            self.save_guards();
        }
        self.config.config.api.ask = decision.effective.ask;
        self.config.config.ai.api_access = decision.effective.ai_access;
        if let Some(loose) = decision.warn {
            tracing::warn!(
                ?loose,
                "the config turns a guard of the API off: it waits for a yes"
            );
        }
        self.guard_warning = decision.warn;
        self.ask_redraw();
    }

    /// The user answered the warning about a guard.
    fn answer_guard(&mut self, turn_off: bool) {
        self.answer_mode.end();
        let Some(config) = self.guard_warning.take() else {
            return;
        };
        self.guard = crate::guards::answer(&self.guard, config, turn_off);
        self.save_guards();
        if turn_off {
            self.config.config.api.ask = config.ask;
            self.config.config.ai.api_access = config.ai_access;
        }
        self.ask_redraw();
    }

    fn save_guards(&self) {
        if let Some(file) = super::guard_file()
            && let Err(err) = crate::guards::save(&file, &self.guard)
        {
            tracing::warn!(file = %file.display(), "cannot save the guards: {err}");
        }
    }

    /// An answer to the box in front: the warning about a guard, or the access question.
    fn answer_front(&mut self, event_loop: &ActiveEventLoop, answer: crate::access::Answer) {
        if self.guard_warning.is_some() {
            self.answer_guard(answer == crate::access::Answer::Allow);
        } else {
            self.answer_access(event_loop, answer);
        }
    }

    /// Tells answer mode which question is in front now (another one ends it). Before each key and click, and
    /// in `about_to_wait`.
    pub(super) fn sync_answer_mode(&mut self, now: Instant) {
        let key = self.access_box(now).map(|b| b.key);
        self.answer_mode.front(key, now);
    }

    /// A key: in answer mode it answers (and is not typed anywhere); the chord starts answer mode. `false`: the
    /// key goes on.
    pub(super) fn access_question_key(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: &KeyEvent,
    ) -> bool {
        let now = Instant::now();
        self.sync_answer_mode(now);
        if self.answer_mode.active(now) {
            let always_ok = self.access_box(now).is_some_and(|b| b.always_ok);
            let mut key = answer_key(
                &event.logical_key,
                event.physical_key,
                event.repeat,
                self.mods.control_key(),
                self.mods.alt_key(),
            );
            // Only a click turns a guard off.
            if self.guard_warning.is_some()
                && matches!(
                    key,
                    crate::access::AnswerKey::Allow
                        | crate::access::AnswerKey::Always
                        | crate::access::AnswerKey::Confirm
                )
            {
                key = crate::access::AnswerKey::Ignore;
            }
            match self.answer_mode.key(key, always_ok, now) {
                Some(answer) => self.answer_front(event_loop, answer),
                None => self.ask_redraw(),
            }
            return true;
        }
        // The AI panel has its own Ctrl+Shift+Enter: there a click answers.
        let dock = self.running.as_ref().is_some_and(|r| r.dock.focused);
        if (!self.api_questions.is_empty() || self.guard_warning.is_some())
            && !dock
            && !event.repeat
            && is_answer_chord(
                &event.logical_key,
                self.mods.control_key(),
                self.mods.shift_key(),
                self.mods.alt_key(),
            )
        {
            self.answer_mode.start(now);
            self.ask_redraw();
            return true;
        }
        false
    }

    /// The left mouse button: a click on a button of the access question answers it (a moment after the box came);
    /// a press outside the box ends answer mode and goes on. `true`: the box took it.
    pub(super) fn access_question_mouse(
        &mut self,
        event_loop: &ActiveEventLoop,
        state: ElementState,
    ) -> bool {
        let now = Instant::now();
        self.sync_answer_mode(now);
        let Some(question) = self.access_box(now) else {
            return false;
        };
        let Some(running) = &self.running else {
            return false;
        };
        let texts: Vec<String> = question.lines.iter().map(|(t, _)| t.clone()).collect();
        let labels: Vec<String> = question.buttons.iter().map(|(_, t)| t.clone()).collect();
        let layout = fterm_render::overlay::question_layout(
            &texts,
            &labels,
            running.tab_area(),
            running.renderer.cell(),
        );
        let (x, y) = (self.mouse.position.0 as f32, self.mouse.position.1 as f32);
        let hit = layout
            .buttons
            .iter()
            .zip(&question.buttons)
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, (button, _))| *button);
        let inside = layout.frame.contains(x, y);
        match state {
            ElementState::Pressed => {
                if hit.is_some() || !inside {
                    let was = self.answer_mode.active(now);
                    self.answer_mode.press(hit, now);
                    if was != self.answer_mode.active(now) {
                        self.ask_redraw();
                    }
                }
                inside
            }
            ElementState::Released => {
                let ours = self.answer_mode.pressed();
                if let Some(answer) = self.answer_mode.release(hit) {
                    self.answer_front(event_loop, answer);
                }
                ours || inside
            }
        }
    }

    fn ask_redraw(&self) {
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// The user answered the access question: run or refuse the requests that waited.
    pub(super) fn answer_access(
        &mut self,
        event_loop: &ActiveEventLoop,
        answer: crate::access::Answer,
    ) {
        use crate::access::Answer;
        self.answer_mode.end();
        let Some(ask) = self.api_questions.pop_front() else {
            return;
        };
        let now = Instant::now();
        // A client with no `hello` gets a record too: its answer stays.
        let record = self.api_clients.entry(ask.client).or_default();
        record.said = Some(answer != Answer::No);
        let (key, bucket) = (record.always_key(), record.bucket());
        match answer {
            Answer::Always => {
                if let Some(key) = key {
                    self.api_always.allow(key);
                }
            }
            Answer::No => {
                // For a minute no new question of that pane (or of the clients with no pane), and the questions
                // of it that wait now are refused too.
                self.api_refused.note(bucket, now);
                let clients = &self.api_clients;
                let (refused, waiting): (Vec<ApiAsk>, Vec<ApiAsk>) =
                    std::mem::take(&mut self.api_questions)
                        .into_iter()
                        .partition(|q| {
                            clients
                                .get(&q.client)
                                .map_or(crate::access::Bucket::NoPane, ApiClient::bucket)
                                == bucket
                        });
                self.api_questions = waiting.into();
                for q in refused {
                    for r in q.requests {
                        let _ = r.reply.send(Err(RpcError::new(RpcError::DENIED, REFUSED)));
                    }
                }
            }
            Answer::Allow => {}
        }
        for request in ask.requests {
            // A call whose client stopped waiting while the question was open does not run now (the client may
            // have tried again, and text would be typed twice).
            if crate::api::still_waits(request.asked, &request.method, &request.params, now) {
                self.api_call(event_loop, request);
            }
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// A client went away: forget it, its waits, and its questions.
    pub(super) fn api_client_gone(&mut self, client: ClientId) {
        self.api_clients.remove(&client);
        self.waits.retain(|w| w.client != client);
        self.ai_waits.retain(|w| w.client != client);
        self.api_questions.retain(|a| a.client != client);
        self.dialog_client_gone(client);
        // Its plan questions too: R would open a Review that nobody waits for.
        self.plan_questions.retain(|q| q.request.client != client);
        self.picture_client_gone(client);
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

    /// `ai_ask`: a question in the AI panel. With `wait`, the answer comes when the AI has finished.
    fn api_ai_ask(&mut self, request: ApiRequest) {
        let result = (|| {
            let text = str_param(&request.params, "text")?
                .filter(|t| !t.trim().is_empty())
                .ok_or_else(|| RpcError::invalid_params("give a `text`"))?;
            let context = match pane_param(&request.params, "pane")? {
                Some(_) => self.pane_context_of(self.target_pane(request.client, &request.params)?),
                None => Vec::new(),
            };
            let wait = bool_param(&request.params, "wait")?.unwrap_or(false);
            if self.ai.running.is_some() {
                return Err(RpcError::invalid_params(
                    "an answer is running: wait for it (the ai_answer event) or stop it (ai_stop)",
                ));
            }
            let from = self
                .api_clients
                .get(&request.client)
                .map_or_else(|| "client".to_owned(), |c| c.name.clone());
            let id = self
                .ai_submit(&text, context, Some(&from))
                .map_err(|refused| {
                    let why = refused.why.map_or_else(
                        || "the question was not sent".to_owned(),
                        |(title, body, _)| format!("{title}: {body}"),
                    );
                    RpcError::new(RpcError::DENIED, why)
                })?;
            Ok((id, wait))
        })();
        match result {
            Err(err) => {
                let _ = request.reply.send(Err(err));
            }
            Ok((id, false)) => {
                let _ = request.reply.send(Ok(json!({ "id": id })));
            }
            // The request ended at once (for example, no model in the config).
            Ok((id, true)) if self.ai.running != Some(id) => {
                let _ = request.reply.send(Ok(self.ai_answer_json(id)));
            }
            Ok((id, true)) => {
                let ms = request
                    .params
                    .get("timeout_ms")
                    .and_then(Value::as_f64)
                    .unwrap_or(crate::api::AI_TIMEOUT_MS)
                    .clamp(0.0, 3_600_000.0);
                self.ai_waits.push(AiWait {
                    client: request.client,
                    id,
                    deadline: Instant::now() + Duration::from_millis(ms as u64),
                    reply: request.reply,
                });
            }
        }
    }

    /// The end of AI answer `id`, as the API gives it.
    fn ai_answer_json(&self, id: u64) -> Value {
        let (text, error) = self.ai.last_answer();
        let mut data = json!({ "event": "ai_answer", "id": id, "text": text });
        if let Some(error) = error {
            data["error"] = json!(error);
        }
        data
    }

    /// AI answer `id` has ended: the `ai_answer` event, and the `ai_ask` calls that wait for it.
    pub(super) fn ai_answered(&mut self, id: u64) {
        let data = self.ai_answer_json(id);
        self.api_event("ai_answer", data.clone());
        let (done, waiting): (Vec<AiWait>, Vec<AiWait>) = std::mem::take(&mut self.ai_waits)
            .into_iter()
            .partition(|w| w.id == id);
        self.ai_waits = waiting;
        for wait in done {
            let _ = wait.reply.send(Ok(data.clone()));
        }
    }

    /// `screenshot`: a PNG of a pane, as the user sees it. It is taken at the next frame.
    fn api_screenshot(&mut self, request: ApiRequest) {
        let result = (|| {
            let pane = self.target_pane(request.client, &request.params)?;
            // A minimized window draws no frames, so the screenshot would wait for ever.
            let minimized = self
                .running
                .as_ref()
                .is_some_and(|r| r.window.is_minimized().unwrap_or(false));
            if minimized {
                return Err(RpcError::invalid_params(
                    "the fterm window is minimized: there is nothing to take a picture of",
                ));
            }
            let path =
                crate::api::shot_path(&request.params, pane.0, now_ms(), &std::env::temp_dir())?;
            Ok((pane, path))
        })();
        match result {
            Ok((pane, path)) => {
                self.shots.push(PendingShot {
                    pane,
                    path,
                    reply: request.reply,
                });
                if let Some(running) = &self.running {
                    running.window.request_redraw();
                }
            }
            Err(err) => {
                let _ = request.reply.send(Err(err));
            }
        }
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
        let (ai_over, ai_waiting): (Vec<AiWait>, Vec<AiWait>) = std::mem::take(&mut self.ai_waits)
            .into_iter()
            .partition(|w| w.deadline <= now);
        self.ai_waits = ai_waiting;
        let replies = over.into_iter().map(|w| w.reply);
        for reply in replies.chain(ai_over.into_iter().map(|w| w.reply)) {
            let _ = reply.send(Err(RpcError::new(RpcError::TIMEOUT, "the time is over")));
        }
        let deadlines = self.waits.iter().map(|w| w.deadline);
        deadlines
            .chain(self.ai_waits.iter().map(|w| w.deadline))
            .min()
    }

    /// A pane closed: its inbox goes, and its waits get an error.
    pub(super) fn api_pane_closed(&mut self, pane: PaneId) {
        // "Always" for programs of this pane ends with it.
        self.api_always.forget_pane(pane.0);
        // An overlay with `wait`: its program ended, so the answer is its exit code.
        let exit = self
            .running
            .as_ref()
            .and_then(|r| r.panes.get(&pane))
            .and_then(|p| p.session.exit_code());
        self.resolve_waits(
            pane,
            Happening::Exit,
            json!({ "event": "exit", "pane": pane.0, "exit": exit }),
        );
        self.dialog_pane_closed(pane);
        self.picture_pane_closed(pane);
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
            "hello" => self.api_hello(client, params, None),
            "list" => Ok(self.api_list()),
            "spawn" => self.api_spawn(client, params),
            "scene_open" => self.api_scene_open(client, params),
            "show_image" => self.api_show_image(client, params),
            "scratch" => self.api_scratch(params),
            "companion" => self.api_companion(params),
            "dashboard" => self.api_dashboard(params),
            "dashboard_tile" => self.api_dashboard_tile(params),
            "agent_session" => self.api_agent_session(client, params),
            "agent_event" => self.api_agent_event(event_loop, client, params),
            "agent_usage" => self.api_agent_usage(event_loop, client, params),
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
            "themes" => {
                let names = fterm_config::theme::list_themes(&self.theme_dirs());
                Ok(json!({ "current": self.theme.name, "themes": names }))
            }
            "set_theme" => {
                let theme = crate::api::theme_param(params)?;
                // A bad theme file: `load_theme` showed a toast; the client gets the reason too.
                if let crate::themes::Override::Named(name) = &theme {
                    fterm_config::theme::find_theme(name, &self.theme_dirs())
                        .map_err(RpcError::invalid_params)?;
                }
                let name = self.use_theme(theme).map_err(RpcError::invalid_params)?;
                Ok(json!({ "name": name }))
            }
            "set_tab_color" => {
                let pane = self.target_pane(client, params)?;
                let color = crate::api::tab_color_param(params)?;
                if !self.set_tab_color(pane, color) {
                    return Err(not_found("tab"));
                }
                Ok(json!({}))
            }
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
            "ai_read" => {
                let last = params
                    .get("last")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize);
                let mut chat = self.ai.to_json(last);
                if let Some(p) = self.config.config.ai.current() {
                    chat["provider"] = json!(p.name);
                    chat["model"] = json!(p.model);
                }
                Ok(chat)
            }
            "ai_input" => {
                let text = str_param(params, "text")?.unwrap_or_default();
                self.ai.input.set(&text);
                self.redraw_ai();
                Ok(json!({}))
            }
            "ai_stop" => Ok(json!({ "stopped": self.ai_stop_answer() })),
            "ai_clear" => {
                self.ai_clear_chat();
                Ok(json!({}))
            }
            other => Err(RpcError::no_method(other)),
        }
    }

    /// `hello`: who the client is. Its pane is the one that its processes show (`chain`); the `pane` that it says
    /// is only a hint.
    fn api_hello(
        &mut self,
        client: ClientId,
        params: &Value,
        chain: Option<&[fterm_term::process::Link]>,
    ) -> Result<Value, RpcError> {
        let name = crate::api::client_name(str_param(params, "name")?.as_deref());
        let running = self.running.as_ref().expect("checked in api_call");
        let hint = pane_param(params, "pane")?
            .map(PaneId)
            .filter(|p| running.panes.contains_key(p));
        // The panes with a live shell of this machine.
        let shells: Vec<(u64, crate::identity::PaneIdentity)> = running
            .panes
            .iter()
            .filter(|(_, p)| p.identity.pid.is_some() && p.session.exit_code().is_none())
            .map(|(id, p)| (id.0, p.identity))
            .collect();
        let checked = chain
            .and_then(|c| crate::identity::verify(c, &shells))
            .map(|c| (PaneId(c.pane), c.program));
        let record = self.api_clients.entry(client).or_default();
        record.hello(name, hint, checked);
        let pane = record.pane;
        if pane.is_none() && hint.is_some() {
            tracing::info!(
                client,
                ?hint,
                "a client says a pane that its processes do not show"
            );
        }
        Ok(json!({
            "api_version": fterm_api::API_VERSION,
            "fterm": env!("CARGO_PKG_VERSION"),
            "pid": std::process::id(),
            "methods": METHODS,
            "events": crate::api::EVENTS,
            "pane": pane.map(|p| p.0),
            "proof": pane.map(|_| "process"),
        }))
    }

    /// The pane of the request: `pane` in the params, else the client's own pane, else the active pane.
    fn target_pane(&self, client: ClientId, params: &Value) -> Result<PaneId, RpcError> {
        let running = self.running.as_ref().expect("checked in api_call");
        let pane = match pane_param(params, "pane")? {
            Some(id) => PaneId(id),
            // The checked pane of the client, else the pane it says (a call that needs a yes still asks), else the
            // active pane.
            None => self
                .api_clients
                .get(&client)
                .and_then(|c| c.pane.or(c.hint))
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
            "workspace": running.mux.pane_place(pane).map(|(ws, _)| ws),
            "program": display_name(p.session.program()),
            "kind": if running.mux.owner_of(pane).is_some() {
                "companion"
            } else if running.mux.scratch_pane() == Some(pane) {
                "scratch"
            } else if running.mux.window_overlay().is_some_and(|(p, _)| p == pane) {
                "window_overlay"
            } else if running.mux.is_floating(pane) {
                "overlay"
            } else if p.review.is_some() {
                "review"
            } else if p.form.is_some() {
                "form"
            } else if p.scene.is_some() {
                "scene"
            } else if p.picture_pane.is_some() {
                "picture"
            } else {
                "terminal"
            },
            "title": p.app_title,
            "cwd": p.shell.cwd,
            "active": running.mux.active_pane() == Some(pane),
            "running": p.shell.is_running(),
            "at_prompt": p.shell.at_prompt(),
            "agent": p.agent.as_ref().map(|a| {
                let info = &p.agent_info;
                let usage = info.usage.as_ref();
                let todos = info.todo_progress().map(|(done, total, current)| {
                    json!({ "done": done, "total": total, "current": current })
                });
                json!({
                    "state": a.kind.name(),
                    "message": a.message,
                    "question": info.question,
                    "todos": todos,
                    "cost": usage.and_then(|u| u.cost_usd),
                    "context": usage.and_then(|u| u.context_pct),
                })
            }),
            "agent_session": p.agent_session,
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
        let active = running.mux.active_tab().map(|t| t.id);
        let tabs: Vec<Value> = running
            .mux
            .all_tabs()
            .map(|tab| {
                let (number, workspace) = running
                    .mux
                    .pane_place(tab.active_pane)
                    .map_or((0, ""), |(ws, i)| (i + 1, ws));
                // An overlay is listed with its tab, after the panes, with the pane under it.
                let overlay = tab.overlay.map(|o| {
                    let mut pane = self.pane_json(o.pane, number);
                    pane["over"] = json!(o.over.0);
                    pane["shown"] = json!(o.shown);
                    pane
                });
                // Companions after their pane, with it and whether they are shown.
                let companions = tab.companions.iter().flat_map(|c| {
                    c.panes.iter().map(|&pane| {
                        let mut json = self.pane_json(pane, number);
                        json["owner"] = json!(c.owner.0);
                        json["shown"] = json!(running.mux.companion_shown(pane));
                        json
                    })
                });
                let panes: Vec<Value> = tab
                    .layout
                    .panes()
                    .into_iter()
                    .map(|pane| self.pane_json(pane, number))
                    .chain(overlay)
                    .chain(companions)
                    .collect();
                json!({
                    "tab": number,
                    "workspace": workspace,
                    "id": tab.id.0,
                    "title": running.tab_title(tab),
                    "active": Some(tab.id) == active,
                    "zoomed": tab.zoomed.is_some(),
                    "color": tab.color.map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}")),
                    "panes": panes,
                })
            })
            .collect();
        let mut out = json!({
            "pid": std::process::id(),
            // The config file, for an agent that sets fterm up for the user.
            "config": self.config_path.display().to_string(),
            "active_pane": running.mux.active_pane().map(|p| p.0),
            "workspace": running.mux.workspace(),
            "tabs": tabs,
        });
        // The overlay of the window (not in a tab), and whether it is shown.
        if let Some((pane, shown)) = running.mux.window_overlay() {
            let mut overlay = self.pane_json(pane, 0);
            overlay["shown"] = json!(shown);
            out["window_overlay"] = overlay;
        }
        // The scratch shell of the window (not in a tab), and whether it is shown.
        if let Some(pane) = running.mux.scratch_pane() {
            let mut scratch = self.pane_json(pane, 0);
            scratch["shown"] = json!(running.mux.input_pane() == Some(pane));
            out["scratch"] = scratch;
        }
        out
    }

    fn api_spawn(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        match str_param(params, "place")?.as_deref() {
            Some("overlay") => return self.api_overlay(client, params),
            Some("companion") => return self.api_spawn_companion(client, params),
            _ => {}
        }
        let place = place_param(params)?;
        let profile = str_param(params, "profile")?;
        let cwd = str_param(params, "cwd")?;
        // A split goes next to `pane`, else next to the pane of the caller (an agent: its own pane, also when the
        // user is in another tab), else next to the active one. The new pane never takes the keys of the user.
        let next_to = match pane_param(params, "pane")?.map(PaneId) {
            Some(pane) => Some(pane),
            None => self
                .api_clients
                .get(&client)
                .and_then(|c| c.pane.or(c.hint)),
        };
        let running = self.running.as_ref().expect("checked in api_call");
        let next_to = match next_to {
            Some(pane) if running.panes.contains_key(&pane) => pane,
            Some(pane) if pane_param(params, "pane")?.is_some() => {
                return Err(not_found(&format!("pane {}", pane.0)));
            }
            _ => running.mux.active_pane().ok_or_else(|| not_found("pane"))?,
        };
        // Set only now: an early error must not leave it for the next pane of the user.
        self.spawn_cwd = cwd;
        let result = match place {
            SpawnWhere::Tab => self.new_tab_behind(next_to, profile.as_deref()),
            SpawnWhere::SplitRight => {
                self.split_beside(next_to, Direction::Right, profile.as_deref())
            }
            SpawnWhere::SplitDown => {
                self.split_beside(next_to, Direction::Down, profile.as_deref())
            }
        };
        self.spawn_cwd = None;
        let pane = result.map_err(|err| RpcError::new(RpcError::INTERNAL, format!("{err:#}")))?;
        if let Some(p) = self.running.as_mut().and_then(|r| r.panes.get_mut(&pane)) {
            p.opened_by = Some(client);
        }
        Ok(json!({ "pane": pane.0 }))
    }

    /// `spawn` with `place: "overlay"` and `wait: true`: the answer comes when the program ends, with its exit code.
    fn api_overlay_wait(&mut self, request: ApiRequest) {
        let opened = self.api_overlay(request.client, &request.params);
        let pane = match opened {
            Ok(answer) => match answer.get("pane").and_then(Value::as_u64) {
                Some(pane) => PaneId(pane),
                None => {
                    let _ = request.reply.send(Ok(answer));
                    return;
                }
            },
            Err(err) => {
                let _ = request.reply.send(Err(err));
                return;
            }
        };
        let ms = request
            .params
            .get("timeout_ms")
            .and_then(Value::as_f64)
            .unwrap_or(86_400_000.0)
            .clamp(0.0, 86_400_000.0);
        self.waits.push(Wait {
            client: request.client,
            pane,
            kind: WaitKind::Exit,
            deadline: Instant::now() + Duration::from_millis(ms as u64),
            reply: request.reply,
        });
    }

    /// `spawn` with `place: "overlay"`: `command` (or the program of `profile`) in a box over a pane.
    fn api_overlay(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let command = str_param(params, "command")?;
        let profile = str_param(params, "profile")?;
        if command.is_none() && profile.is_none() {
            return Err(RpcError::invalid_params(
                "an overlay needs `command` (or a `profile`)",
            ));
        }
        let size = params
            .get("size")
            .and_then(Value::as_f64)
            .map(|s| s.round().clamp(20.0, 100.0) as u8);
        if let Some(over) = pane_param(params, "pane")?.map(PaneId) {
            let running = self.running.as_mut().expect("checked in api_call");
            if !running.panes.contains_key(&over) {
                return Err(not_found(&format!("pane {}", over.0)));
            }
            running.mux.reveal(over);
        }
        if let Some(cwd) = str_param(params, "cwd")? {
            self.spawn_cwd = Some(cwd);
        }
        let opened = self.open_overlay(command, profile.as_deref(), size);
        self.spawn_cwd = None;
        let pane = opened.map_err(|err| RpcError::new(RpcError::INTERNAL, format!("{err:#}")))?;
        if let Some(p) = self.running.as_mut().and_then(|r| r.panes.get_mut(&pane)) {
            p.opened_by = Some(client);
        }
        Ok(json!({ "pane": pane.0 }))
    }

    /// `agent_event`: what the agent of a pane did (the hooks of Claude Code, through `ftermctl agent-event --hook`).
    /// The Agents panel shows it, and the state of the agent follows it (working, waiting with the real question,
    /// done), the same way as the state sequence.
    fn api_agent_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        client: ClientId,
        params: &Value,
    ) -> Result<Value, RpcError> {
        let pane = self.target_pane(client, params)?;
        let event = crate::agent_info::event_from_json(params).ok_or_else(|| {
            RpcError::invalid_params("`kind` must be prompt, tool, question, or done")
        })?;
        let running = self.running.as_mut().expect("checked in api_call");
        let Some(p) = running.panes.get_mut(&pane) else {
            return Err(not_found(&format!("pane {}", pane.0)));
        };
        // Hooks tell the state: no guessing from the title.
        p.agent_hooks = true;
        if let Some(id) =
            str_param(params, "session_id")?.filter(|id| crate::session_state::valid_session_id(id))
        {
            p.agent_session = Some(id);
        }
        p.agent_info.apply(&event, Instant::now());
        let counts = matches!(
            event,
            crate::agent_info::AgentEvent::Prompt(_) | crate::agent_info::AgentEvent::Done(_)
        );
        let (state, message) = match &event {
            crate::agent_info::AgentEvent::Prompt(_) => ("working", String::new()),
            crate::agent_info::AgentEvent::Tool { .. } => {
                ("working", p.agent_info.last_step().unwrap_or("").to_owned())
            }
            crate::agent_info::AgentEvent::Question(text) => ("waiting", text.clone()),
            crate::agent_info::AgentEvent::Done(_) | crate::agent_info::AgentEvent::Usage(_) => {
                ("done", String::new())
            }
        };
        self.agent_state(event_loop, pane, state, &message);
        self.agent_changed(event_loop, pane, Some(&event));
        if counts {
            self.remember_conversation(pane);
        }
        Ok(json!({ "pane": pane.0 }))
    }

    /// `agent_usage`: the cost, the context, and the model of the agent of a pane (`ftermctl statusline`).
    fn api_agent_usage(
        &mut self,
        event_loop: &ActiveEventLoop,
        client: ClientId,
        params: &Value,
    ) -> Result<Value, RpcError> {
        let pane = self.target_pane(client, params)?;
        let num = |key: &str| params.get(key).and_then(Value::as_f64);
        let usage = crate::agent_info::Usage {
            cost_usd: num("cost"),
            lines_added: params.get("lines_added").and_then(Value::as_u64),
            lines_removed: params.get("lines_removed").and_then(Value::as_u64),
            context_pct: num("context"),
            model: str_param(params, "model")?.map(|m| crate::agent_info::clean(&m, 100)),
            five_hour_pct: num("five_hour"),
            seven_day_pct: num("seven_day"),
        };
        let event = crate::agent_info::AgentEvent::Usage(usage);
        let running = self.running.as_mut().expect("checked in api_call");
        if let Some(p) = running.panes.get_mut(&pane) {
            p.agent_info.apply(&event, Instant::now());
            running.window.request_redraw();
        }
        self.agent_changed(event_loop, pane, Some(&event));
        Ok(json!({ "pane": pane.0 }))
    }

    /// `agent_session`: the conversation id of the agent in a pane (the SessionStart hook of Claude Code), so the
    /// pane comes back to it after a restore.
    fn api_agent_session(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let pane = self.target_pane(client, params)?;
        let id = str_param(params, "session_id")?
            .ok_or_else(|| RpcError::invalid_params("`session_id` is missing"))?;
        if !crate::session_state::valid_session_id(&id) {
            return Err(RpcError::invalid_params(
                "`session_id`: letters, digits, `-`, and `_` only (at most 128)",
            ));
        }
        let running = self.running.as_mut().expect("checked in api_call");
        if let Some(p) = running.panes.get_mut(&pane) {
            tracing::info!(pane = pane.0, session = %id, "the agent conversation of a pane");
            p.agent_session = Some(id);
        }
        Ok(json!({ "pane": pane.0 }))
    }

    /// `spawn` with `place: "companion"`: `command` (or a shell) as a companion of `pane` (default: the active one).
    fn api_spawn_companion(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let command = str_param(params, "command")?;
        // The owner: `pane`, else the pane of the caller (an agent: its own pane), else the active one.
        let owner = pane_param(params, "pane")?.map(PaneId);
        if let Some(owner) = owner {
            let running = self.running.as_ref().expect("checked in api_call");
            if !running.panes.contains_key(&owner) {
                return Err(not_found(&format!("pane {}", owner.0)));
            }
        }
        // A companion of a companion belongs to its owner.
        let running = self.running.as_ref().expect("checked in api_call");
        let owner = owner
            .or_else(|| {
                self.api_clients
                    .get(&client)
                    .and_then(|c| c.pane)
                    .filter(|p| running.panes.contains_key(p))
            })
            .map(|o| running.mux.owner_of(o).unwrap_or(o));
        if let Some(cwd) = str_param(params, "cwd")? {
            self.spawn_cwd = Some(cwd);
        }
        let opened = self.open_companion(command, owner, true);
        self.spawn_cwd = None;
        let pane = opened.map_err(|err| RpcError::new(RpcError::INTERNAL, format!("{err:#}")))?;
        if let Some(p) = self.running.as_mut().and_then(|r| r.panes.get_mut(&pane)) {
            p.opened_by = Some(client);
        }
        Ok(json!({ "pane": pane.0 }))
    }

    /// `companion`: `action` "toggle" (hide or show) or "next" for the companions of `pane` (default: the active one).
    fn api_companion(&mut self, params: &Value) -> Result<Value, RpcError> {
        let action = str_param(params, "action")?;
        let running = self.running.as_mut().expect("checked in api_call");
        if let Some(pane) = pane_param(params, "pane")?.map(PaneId) {
            if !running.panes.contains_key(&pane) {
                return Err(not_found(&format!("pane {}", pane.0)));
            }
            running.mux.reveal(pane);
        }
        let done = match action.as_deref() {
            Some("toggle") => running.mux.toggle_companion().is_some(),
            Some("next") => running.mux.next_companion().is_some(),
            _ => {
                return Err(RpcError::invalid_params(
                    "`action` must be \"toggle\" or \"next\"",
                ));
            }
        };
        if !done {
            return Err(not_found("a companion of this pane"));
        }
        self.tab_changed();
        self.update_ime();
        let mux = &self.running.as_ref().expect("checked in api_call").mux;
        let shown = mux.active_pane().filter(|p| mux.companion_shown(*p));
        Ok(json!({ "pane": shown.map(|p| p.0), "shown": shown.is_some() }))
    }

    /// `scratch`: shows or hides the scratch shell (`show`; no `show` = the other one).
    fn api_scratch(&mut self, params: &Value) -> Result<Value, RpcError> {
        let want = params.get("show").and_then(Value::as_bool);
        let shown = |app: &Self| {
            let mux = &app.running.as_ref().expect("checked in api_call").mux;
            mux.scratch_pane().is_some() && mux.input_pane() == mux.scratch_pane()
        };
        if want != Some(shown(self)) {
            self.toggle_scratch();
        }
        let mux = &self.running.as_ref().expect("checked in api_call").mux;
        Ok(json!({ "pane": mux.scratch_pane().map(|p| p.0), "shown": shown(self) }))
    }

    /// `scene_open`: a Braille scene next to a pane (Phase 11). The client may draw in it at once.
    fn api_scene_open(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let direction = crate::api::scene_place(params)?;
        // Next to `pane`, else next to the pane of the caller; the user keeps the keys (see `api_spawn`).
        let next_to = self.target_pane(client, params)?;
        let pane = self
            .split_beside_with(next_to, direction, |app, size| Ok(app.spawn_scene(size)))
            .map_err(|err| RpcError::new(RpcError::INTERNAL, format!("{err:#}")))?;
        if let Some(p) = self.running.as_mut().and_then(|r| r.panes.get_mut(&pane)) {
            p.opened_by = Some(client);
        }
        Ok(self.scene_size(pane))
    }

    /// `show_image`: a picture pane next to a pane. The picture is `data` (base64) or a `file` on this machine.
    fn api_show_image(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let (pane, width, height) = self.open_image(client, params)?;
        Ok(json!({ "pane": pane.0, "width": width, "height": height }))
    }

    /// Opens the picture pane of `show_image` (with the `question` of the client on its tool bar).
    pub(super) fn open_image(
        &mut self,
        client: ClientId,
        params: &Value,
    ) -> Result<(PaneId, u32, u32), RpcError> {
        let direction = crate::api::scene_place(params)?;
        // `pane` (it asked first when it is not the client's own), else the client's own pane: never the pane
        // that the user is in by chance.
        let next_to = match pane_param(params, "pane")?.map(PaneId) {
            Some(pane) => Some(pane),
            None => self.api_clients.get(&client).and_then(|c| c.pane),
        };
        if let Some(next_to) = next_to
            && !self
                .running
                .as_ref()
                .is_some_and(|r| r.panes.contains_key(&next_to))
        {
            return Err(not_found(&format!("pane {}", next_to.0)));
        }
        let bytes = crate::api::picture_param(params)?;
        let picture = fterm_term::picture::decode(&bytes, fterm_term::picture::MAX_SIDE)
            .map_err(RpcError::invalid_params)?;
        let (width, height) = (picture.width, picture.height);
        let pane = self
            .open_client_picture(picture, direction, next_to)
            .map_err(|err| RpcError::new(RpcError::INTERNAL, format!("{err:#}")))?;
        let question = crate::api::str_param(params, "question")?;
        if let Some(p) = self.running.as_mut().and_then(|r| r.panes.get_mut(&pane)) {
            p.opened_by = Some(client);
            if let Some(pp) = &mut p.picture_pane {
                pp.question = question;
            }
        }
        Ok((pane, width, height))
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
            let mut scene = scene.lock().unwrap();
            scene.draw(&ops).map_err(RpcError::invalid_params)?;
            p.session.feed(&fterm_scene::render(scene.canvas()));
        }
        running.window.request_redraw();
        Ok(self.scene_size(pane))
    }

    /// `scene_resized` for each scene pane that got a new size (zoom, a split, the window), so a live
    /// chart can be drawn again with more or fewer dots. Called when the event loop waits.
    pub(super) fn scene_events(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let resized: Vec<PaneId> = running
            .panes
            .iter()
            .filter(|(_, p)| {
                p.scene
                    .as_ref()
                    .is_some_and(|s| s.lock().unwrap().take_resized())
            })
            .map(|(id, _)| *id)
            .collect();
        for pane in resized {
            let mut data = self.scene_size(pane);
            data["event"] = json!("scene_resized");
            self.api_event("scene_resized", data.clone());
            self.resolve_waits(pane, crate::waits::Happening::SceneResized, data);
        }
    }

    /// The pane and the size of its scene, in cells and in dots.
    fn scene_size(&self, pane: PaneId) -> Value {
        let size = self
            .running
            .as_ref()
            .and_then(|r| r.panes.get(&pane))
            .and_then(|p| p.scene.as_ref())
            .map(|s| {
                let s = s.lock().unwrap();
                let c = s.canvas();
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
        let styled = bool_param(params, "styled")?.unwrap_or(false);
        let palette = self.palette();
        let running = self.running.as_ref().expect("checked in api_call");
        let p = &running.panes[&pane];
        let session = &p.session;
        // The lines (from the top of the history) to give.
        let range = session.with_term(|term| {
            let total = total_lines(term);
            match what.as_str() {
                "screen" => Ok(Some((term.grid().history_size(), total))),
                "history" => Ok(Some((total.saturating_sub(lines), total))),
                "last_output" => Ok(p
                    .shell
                    .last_output(total)
                    .map(|(start, end)| (start.max(end.saturating_sub(lines)), end))),
                other => Err(RpcError::invalid_params(format!(
                    "`what` must be \"screen\", \"history\", or \"last_output\", got `{other}`"
                ))),
            }
        })?;
        let Some((from, to)) = range else {
            return Err(RpcError::new(
                RpcError::NOT_FOUND,
                "no command output yet (it needs shell integration)",
            ));
        };
        let text = match what.as_str() {
            "screen" => session.screen_text(),
            _ => session.with_term(|term| lines_text(term, from, to)),
        };
        let mut answer = json!({ "pane": pane.0, "text": text.trim_end() });
        if what == "last_output" {
            let last = p.last_command.as_ref();
            answer["running"] = json!(p.shell.is_running());
            answer["command"] = json!(last.and_then(|c| c.command.clone()));
            answer["exit"] = json!(last.and_then(|c| c.exit));
        }
        if styled {
            let styled = session.with_term(|term| {
                let mut lines = fterm_term::styled::styled_lines(term, from, to, &palette);
                // Empty lines at the end are not in `text` either.
                while lines.last().is_some_and(Vec::is_empty) {
                    lines.pop();
                }
                let (fg, bg) = fterm_term::styled::default_colors(term, &palette);
                crate::api::styled_json(&lines, fg, bg)
            });
            for key in ["fg", "bg", "lines"] {
                answer[key] = styled[key].clone();
            }
        }
        Ok(answer)
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

    /// Colors the tab of `pane` (`None` = no color). `false` = no such pane.
    pub(super) fn set_tab_color(&mut self, pane: PaneId, color: Option<[u8; 3]>) -> bool {
        let Some(running) = self.running.as_mut() else {
            return false;
        };
        let Some(tab) = running.mux.pane_tab(pane) else {
            return false;
        };
        running.mux.set_color(tab, color);
        running.window.request_redraw();
        true
    }

    fn api_set_title(&mut self, client: ClientId, params: &Value) -> Result<Value, RpcError> {
        let pane = self.target_pane(client, params)?;
        let title = str_param(params, "title")?.unwrap_or_default();
        let running = self.running.as_mut().expect("checked in api_call");
        let tab = running.mux.pane_tab(pane).ok_or_else(|| not_found("tab"))?;
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
        let tab = running.pane_tab_title(to).unwrap_or_default();
        let first_line: String = text
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(80)
            .collect();
        self.notify(
            Some(to),
            &tr!("toast.message_for", tab = tab),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn program(pid: u32) -> crate::identity::Program {
        crate::identity::Program {
            pid,
            start: 100,
            name: "claude.exe".into(),
        }
    }

    #[test]
    fn keys_of_the_access_question() {
        use crate::access::AnswerKey as A;
        let k = |logical: Key, code: KeyCode, repeat: bool, mods: &str| {
            answer_key(
                &logical,
                PhysicalKey::Code(code),
                repeat,
                mods.contains("ctrl"),
                mods.contains("alt"),
            )
        };
        let ch = |c: &str| Key::Character(c.into());
        assert_eq!(k(ch("y"), KeyCode::KeyY, false, ""), A::Allow);
        assert_eq!(
            k(ch("A"), KeyCode::KeyA, false, ""),
            A::Always,
            "Shift or CapsLock: the same letter"
        );
        assert_eq!(k(ch("n"), KeyCode::KeyN, false, ""), A::No);
        assert_eq!(k(ch("1"), KeyCode::Digit1, false, ""), A::Allow);
        assert_eq!(
            k(ch("\u{e9}"), KeyCode::Digit2, false, ""),
            A::Always,
            "digits by place (AZERTY)"
        );
        assert_eq!(k(ch("3"), KeyCode::Numpad3, false, ""), A::No);
        assert_eq!(
            k(ch("\u{43d}"), KeyCode::KeyY, false, ""),
            A::Other,
            "the Russian layout: no Y there"
        );
        assert_eq!(
            k(Key::Named(NamedKey::Enter), KeyCode::Enter, false, ""),
            A::Confirm
        );
        assert_eq!(
            k(Key::Named(NamedKey::Escape), KeyCode::Escape, false, ""),
            A::No
        );
        assert_eq!(
            k(Key::Named(NamedKey::Shift), KeyCode::ShiftLeft, false, ""),
            A::Ignore
        );
        assert_eq!(k(ch("y"), KeyCode::KeyY, true, ""), A::Ignore, "a held key");
        assert_eq!(
            k(ch("a"), KeyCode::KeyA, false, "ctrl"),
            A::Other,
            "Ctrl+A (tmux, screen)"
        );
        assert_eq!(k(ch("x"), KeyCode::KeyX, false, ""), A::Other);
        assert!(is_answer_chord(
            &Key::Named(NamedKey::Enter),
            true,
            true,
            false
        ));
        assert!(!is_answer_chord(
            &Key::Named(NamedKey::Enter),
            true,
            false,
            false
        ));
        assert!(!is_answer_chord(
            &Key::Named(NamedKey::Enter),
            false,
            false,
            false
        ));
        assert!(!is_answer_chord(
            &Key::Named(NamedKey::Enter),
            true,
            true,
            true
        ));
        assert!(!is_answer_chord(&ch("y"), true, true, false));
    }

    #[test]
    fn the_answer_chord_is_free_in_the_default_keys() {
        // While a question is open the chord answers it, before the keymap: no default key may need it.
        let keys = fterm_config::keys::Keymap::with_defaults();
        let chord = fterm_config::keys::KeyChord::parse("ctrl+shift+enter").unwrap();
        assert_eq!(keys.get(&chord), None);
    }

    #[test]
    fn a_hook_is_a_client_of_its_own() {
        let (a, b) = (hook_client("on_agent"), hook_client("on_agent_event"));
        assert_ne!(a, b);
        assert!(
            a > u64::MAX / 2 && b > u64::MAX / 2,
            "far from the ids of the server"
        );
        let c = ApiClient::for_hook("on_agent");
        assert_eq!(c.name, "on_agent");
        assert_eq!(
            c.bucket(),
            crate::access::Bucket::Hooks,
            "a No for a hook is not a No for a program"
        );
        assert_eq!(c.always_key(), None, "no Always for a hook");
    }

    #[test]
    fn the_first_hello_says_who_a_client_is() {
        let mut c = ApiClient::default();
        c.hello(
            "claude".into(),
            Some(PaneId(9)),
            Some((PaneId(3), program(7))),
        );
        assert_eq!(
            (c.name.as_str(), c.pane, c.hint),
            ("claude", Some(PaneId(3)), Some(PaneId(9)))
        );
        assert_eq!(c.program, Some(program(7)));
        c.said = Some(true);
        // A second hello: another name or another pane changes nothing, and the answer stays.
        c.hello(
            "other".into(),
            Some(PaneId(4)),
            Some((PaneId(4), program(8))),
        );
        assert_eq!(
            (c.name.as_str(), c.pane, c.hint),
            ("claude", Some(PaneId(3)), Some(PaneId(9)))
        );
        assert_eq!((c.program, c.said), (Some(program(7)), Some(true)));
    }

    #[test]
    fn a_pane_that_was_not_known_may_come_later() {
        let mut c = ApiClient::default();
        c.hello("claude".into(), Some(PaneId(9)), None);
        assert_eq!(c.pane, None);
        c.hello(
            "claude".into(),
            Some(PaneId(9)),
            Some((PaneId(9), program(7))),
        );
        assert_eq!((c.pane, c.program), (Some(PaneId(9)), Some(program(7))));
    }
}
