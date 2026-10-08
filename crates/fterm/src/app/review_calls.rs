//! A Review tab: an agent (or a script) asks the user to check a plan item by item, and waits for the answer.

use std::time::Instant;

use fterm_api::RpcError;
use fterm_api::server::ClientId;
use serde_json::Value;

use super::*;
use crate::api::ApiRequest;
use crate::review::{Decision, Outcome, Review, Start};
use fterm_config::load::PlanReview;

/// A review in a tab, and the grid size it was last drawn for.
pub(super) struct ReviewPane {
    pub review: Review,
    pub drawn: Option<GridSize>,
}

/// A `review` call that waits for the user.
pub(super) struct PendingReview {
    pub pane: PaneId,
    pub client: ClientId,
    pub deadline: Instant,
    pub reply: std::sync::mpsc::Sender<Result<Value, RpcError>>,
}

/// "Review the plan here?" before a plan of plan mode opens (`plan_review = "ask"`).
pub(super) struct PlanQuestion {
    pub request: ApiRequest,
    pub asked: crate::api::ReviewRequest,
    pub near: Option<PaneId>,
    /// No answer by then: the dialog of the agent.
    pub deadline: Instant,
    /// The time of the review counts from the call.
    pub review_deadline: Instant,
    pub lines: Vec<String>,
}

/// How long the question waits for R or Esc.
const QUESTION_TIME: std::time::Duration = std::time::Duration::from_secs(120);

impl App {
    /// `review`: opens a Review tab; the answer comes when the user sends it, closes the tab, or the time ends.
    /// A plan of the plan mode hook follows `plan_review`: it may ask first, or not open.
    pub(super) fn api_review(&mut self, event_loop: &ActiveEventLoop, request: ApiRequest) {
        let client = self.api_clients.get(&request.client);
        let name = client.map_or("an agent".to_owned(), |c| c.name.clone());
        let near = match crate::api::pane_param(&request.params, "pane") {
            Ok(pane) => pane.map(PaneId).or(client.and_then(|c| c.pane)),
            Err(err) => {
                let _ = request.reply.send(Err(err));
                return;
            }
        };
        let asked = match crate::api::review_param(&request.params, &name) {
            Ok(asked) => asked,
            Err(err) => {
                let _ = request.reply.send(Err(err));
                return;
            }
        };
        let now = Instant::now();
        let review_deadline = now + asked.timeout;
        match crate::review::start(asked.plan_mode, self.plan_review()) {
            Start::Open => self.start_review(event_loop, request, asked, near, review_deadline),
            Start::Skip => {
                let _ = request.reply.send(Ok(crate::review::skipped()));
            }
            Start::Ask => {
                let lines = crate::review::question_lines(&asked.from, &asked.title);
                let title = lines[0].clone();
                let body = format!("R = review it in fterm, Esc = the dialog of {}", asked.from);
                self.plan_questions.push(PlanQuestion {
                    request,
                    asked,
                    near,
                    deadline: now + QUESTION_TIME,
                    review_deadline,
                    lines,
                });
                self.notify(near, &title, &body, Level::Attention, Source::App);
                if let Some(running) = &self.running {
                    running.window.request_redraw();
                }
            }
        }
    }

    /// `plan_review` now: a mode chosen in the palette, else the config.
    pub(super) fn plan_review(&self) -> PlanReview {
        self.plan_review_live
            .unwrap_or(self.config.config.plan_review)
    }

    /// `plan_review_mode`: the next mode, with a toast.
    pub(super) fn step_plan_review(&mut self) {
        let mode = self.plan_review().next();
        self.plan_review_live = Some(mode);
        let what = match mode {
            PlanReview::Always => "Each plan of plan mode opens in a Review tab.",
            PlanReview::Ask => "fterm asks first: R = review, Esc = the dialog of Claude Code.",
            PlanReview::Never => "Claude Code shows its own dialog.",
        };
        let body = format!(
            "{what} plan_review = \"{}\" in fterm.lua keeps it.",
            mode.name()
        );
        let title = format!("Plan review: {}", mode.name());
        self.notify(None, &title, &body, Level::Info, Source::App);
    }

    /// The lines of the first open plan question.
    pub(super) fn plan_question_lines(&self) -> Option<Vec<String>> {
        self.plan_questions.first().map(|q| q.lines.clone())
    }

    /// Keys while a plan question is open: R reviews, Esc gives the dialog of the agent.
    pub(super) fn plan_question_key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        let review = match &event.logical_key {
            Key::Character(c) if c.eq_ignore_ascii_case("r") => true,
            Key::Named(NamedKey::Escape) => false,
            _ => return,
        };
        if self.plan_questions.is_empty() {
            return;
        }
        let q = self.plan_questions.remove(0);
        if review {
            self.start_review(event_loop, q.request, q.asked, q.near, q.review_deadline);
        } else {
            let _ = q.request.reply.send(Ok(crate::review::skipped()));
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Plan questions with no answer in time: the dialog of the agent. Gives the next deadline.
    pub(super) fn expire_plan_questions(&mut self, now: Instant) -> Option<Instant> {
        let (over, waiting): (Vec<PlanQuestion>, Vec<PlanQuestion>) =
            std::mem::take(&mut self.plan_questions)
                .into_iter()
                .partition(|q| q.deadline <= now);
        self.plan_questions = waiting;
        if !over.is_empty()
            && let Some(running) = &self.running
        {
            running.window.request_redraw();
        }
        for q in over {
            let _ = q.request.reply.send(Ok(crate::review::skipped()));
        }
        self.plan_questions.iter().map(|q| q.deadline).min()
    }

    /// Opens the Review tab of a request, and waits for the user.
    fn start_review(
        &mut self,
        event_loop: &ActiveEventLoop,
        request: ApiRequest,
        asked: crate::api::ReviewRequest,
        near: Option<PaneId>,
        deadline: Instant,
    ) {
        let review = Review::new(&asked.title, &asked.from, asked.items);
        let Some(pane) = self.open_review(event_loop, review, near) else {
            let _ = request
                .reply
                .send(Err(RpcError::new(RpcError::INTERNAL, "no window")));
            return;
        };
        self.reviews.push(PendingReview {
            pane,
            client: request.client,
            deadline,
            reply: request.reply,
        });
    }

    /// A new tab with the review. It becomes the active tab when the user looks at the pane that asked.
    fn open_review(
        &mut self,
        event_loop: &ActiveEventLoop,
        review: Review,
        near: Option<PaneId>,
    ) -> Option<PaneId> {
        let focused = self.focused;
        let running = self.running.as_mut()?;
        let size = running.grid_for(running.tab_area());
        let id = running.mux.new_pane_id();
        let title = format!("Review: {}", review.title);
        running.panes.insert(
            id,
            Pane {
                session: Session::scene(size),
                app_title: Some(title.clone()),
                shell: ShellState::default(),
                agent: None,
                agent_hooks: false,
                last_command: None,
                remote: true,
                opened_by: None,
                profile: None,
                rerun: None,
                intro_lines: 0,
                scene: None,
                review: Some(ReviewPane {
                    review,
                    drawn: None,
                }),
                harmonize: false,
                palette_changes: Some(false),
            },
        );
        running
            .mux
            .add_tab(fterm_mux::Layout::Pane(id), id, Some(title.clone()));
        let looking = focused && near.is_some() && running.mux.active_pane() == near;
        if looking {
            running.mux.select_last();
        }
        tracing::info!(pane = id.0, %title, "new review");
        self.draw_reviews();
        // A yellow dot and a notification: the user must act.
        self.agent_state(event_loop, id, "waiting", "a plan to review");
        self.tab_changed();
        Some(id)
    }

    /// Draws the reviews that are new, changed size, or got new colors.
    pub(super) fn draw_reviews(&mut self) {
        let ui = self.ui;
        let Some(running) = self.running.as_mut() else {
            return;
        };
        for p in running.panes.values_mut() {
            let Some(rp) = p.review.as_mut() else {
                continue;
            };
            let size = p.session.grid_size();
            if rp.drawn == Some(size) {
                continue;
            }
            let text = crate::review::render(&mut rp.review, size.columns, size.rows, &ui);
            p.session.feed(text.as_bytes());
            rp.drawn = Some(size);
        }
    }

    /// Draws every review again (for example with a new theme).
    pub(super) fn redraw_reviews(&mut self) {
        if let Some(running) = self.running.as_mut() {
            for rp in running.panes.values_mut().filter_map(|p| p.review.as_mut()) {
                rp.drawn = None;
            }
        }
        self.draw_reviews();
    }

    /// The active pane is a review.
    pub(super) fn active_review(&self) -> Option<PaneId> {
        let running = self.running.as_ref()?;
        let pane = running.mux.active_pane()?;
        running.panes.get(&pane)?.review.as_ref().map(|_| pane)
    }

    /// A key in the active Review tab.
    pub(super) fn review_key(&mut self, event: &KeyEvent) {
        use crate::review::Key as R;
        let Some(pane) = self.active_review() else {
            return;
        };
        let (ctrl, shift, alt) = (
            self.mods.control_key(),
            self.mods.shift_key(),
            self.mods.alt_key(),
        );
        let text = event
            .text
            .as_deref()
            .filter(|_| !ctrl && !alt)
            .map(|t| t.chars().filter(|c| !c.is_control()).collect::<String>())
            .filter(|t| !t.is_empty());
        let key = match &event.logical_key {
            Key::Named(NamedKey::ArrowUp) => R::Up,
            Key::Named(NamedKey::ArrowDown) => R::Down,
            Key::Named(NamedKey::PageUp) => R::PageUp,
            Key::Named(NamedKey::PageDown) => R::PageDown,
            Key::Named(NamedKey::Home) => R::Home,
            Key::Named(NamedKey::End) => R::End,
            Key::Named(NamedKey::ArrowLeft) => R::Left,
            Key::Named(NamedKey::ArrowRight) => R::Right,
            Key::Named(NamedKey::Enter) => R::Enter { ctrl, shift },
            Key::Named(NamedKey::Escape) => R::Escape,
            Key::Named(NamedKey::Backspace) => R::Backspace,
            Key::Named(NamedKey::Delete) => R::Delete,
            _ => match &text {
                Some(text) => R::Text(text),
                None => return,
            },
        };
        let Some(running) = self.running.as_mut() else {
            return;
        };
        let page = running
            .panes
            .get(&pane)
            .map_or(10, |p| p.session.grid_size().rows / 2);
        let Some(rp) = running.panes.get_mut(&pane).and_then(|p| p.review.as_mut()) else {
            return;
        };
        match rp.review.key(key, page) {
            Outcome::Nothing => return,
            Outcome::Changed => rp.drawn = None,
            Outcome::Send => {
                let decision = rp.review.decision();
                return self.finish_review(pane, decision);
            }
        }
        self.draw_reviews();
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// Sends the answer of a review to the client that waits, and closes its tab.
    fn finish_review(&mut self, pane: PaneId, decision: Decision) {
        let result = self
            .running
            .as_ref()
            .and_then(|r| r.panes.get(&pane))
            .and_then(|p| p.review.as_ref())
            .map(|rp| rp.review.result(decision));
        let (done, waiting): (Vec<PendingReview>, Vec<PendingReview>) =
            std::mem::take(&mut self.reviews)
                .into_iter()
                .partition(|r| r.pane == pane);
        self.reviews = waiting;
        if let Some(result) = result {
            for r in done {
                let _ = r.reply.send(Ok(result.clone()));
            }
        }
        self.close_review_tab(pane);
    }

    /// Closes the tab of a review, unless it is the last tab (then the window would close).
    fn close_review_tab(&mut self, pane: PaneId) {
        let many = self
            .running
            .as_ref()
            .is_some_and(|r| r.mux.tabs().len() > 1);
        if many {
            self.close_pane_now(pane);
        }
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }

    /// A review tab closed: the client gets "cancelled".
    pub(super) fn review_pane_closed(&mut self, pane: PaneId) {
        let result = self
            .running
            .as_ref()
            .and_then(|r| r.panes.get(&pane))
            .and_then(|p| p.review.as_ref())
            .map(|rp| rp.review.result(Decision::Cancelled));
        let (gone, waiting): (Vec<PendingReview>, Vec<PendingReview>) =
            std::mem::take(&mut self.reviews)
                .into_iter()
                .partition(|r| r.pane == pane);
        self.reviews = waiting;
        if let Some(result) = result {
            for r in gone {
                let _ = r.reply.send(Ok(result.clone()));
            }
        }
    }

    /// The client stopped waiting (for example the agent was stopped): its review tabs go.
    pub(super) fn review_client_gone(&mut self, client: ClientId) {
        let panes: Vec<PaneId> = self
            .reviews
            .iter()
            .filter(|r| r.client == client)
            .map(|r| r.pane)
            .collect();
        self.reviews.retain(|r| r.client != client);
        for pane in panes {
            self.close_review_tab(pane);
        }
    }

    /// Reviews whose time is over: "cancelled" with `timed_out`, and the tab goes. Gives the next deadline.
    pub(super) fn expire_reviews(&mut self, now: Instant) -> Option<Instant> {
        let over: Vec<PaneId> = self
            .reviews
            .iter()
            .filter(|r| r.deadline <= now)
            .map(|r| r.pane)
            .collect();
        for pane in over {
            let result = self
                .running
                .as_ref()
                .and_then(|r| r.panes.get(&pane))
                .and_then(|p| p.review.as_ref())
                .map(|rp| rp.review.result(Decision::Cancelled));
            let (done, waiting): (Vec<PendingReview>, Vec<PendingReview>) =
                std::mem::take(&mut self.reviews)
                    .into_iter()
                    .partition(|r| r.pane == pane);
            self.reviews = waiting;
            for r in done {
                let mut answer = result.clone().unwrap_or_default();
                answer["timed_out"] = true.into();
                let _ = r.reply.send(Ok(answer));
            }
            self.close_review_tab(pane);
        }
        self.reviews.iter().map(|r| r.deadline).min()
    }
}
