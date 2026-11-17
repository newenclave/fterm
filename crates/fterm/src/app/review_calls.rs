//! A Review tab: an agent (or a script) asks the user to check a plan item by item, and waits for the answer.

use std::time::Instant;

use fterm_api::RpcError;

use super::dialog_calls::{
    Dialog, DialogInput, DialogKind, PendingDialog, dialog_deadline, dialog_input,
};
use super::*;
use crate::api::ApiRequest;
use crate::review::{Outcome, Review, Start};
use fterm_config::load::PlanReview;

/// A review in a tab, and the grid size it was last drawn for.
pub(super) struct ReviewPane {
    pub review: Review,
    pub drawn: Option<GridSize>,
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
        let Some(review_deadline) = dialog_deadline(request.asked, asked.timeout, now) else {
            // It waited for the access question until its time was over: its client stopped waiting.
            let mut answer = DialogKind::Review.cancelled();
            answer["timed_out"] = true.into();
            let _ = request.reply.send(Ok(answer));
            return;
        };
        match crate::review::start(asked.plan_mode, self.plan_review()) {
            Start::Open => {
                self.start_review(event_loop, request, asked, near, review_deadline, false)
            }
            Start::Skip => {
                let _ = request.reply.send(Ok(crate::review::skipped()));
            }
            Start::Ask => {
                let lines = crate::review::question_lines(&asked.from, &asked.agent, &asked.title);
                let title = lines[0].clone();
                let body = tr!("toast.plan_question_body", agent = asked.from);
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
        let name = mode.name();
        let body = match mode {
            PlanReview::Always => tr!("toast.plan_review_always", mode = name),
            PlanReview::Ask => tr!("toast.plan_review_ask", mode = name),
            PlanReview::Never => tr!("toast.plan_review_never", mode = name),
        };
        let title = tr!("toast.plan_review", mode = name);
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
        if review && q.review_deadline <= Instant::now() {
            // Its time ran out while the question was open: no Review.
            let mut answer = DialogKind::Review.cancelled();
            answer["timed_out"] = true.into();
            let _ = q.request.reply.send(Ok(answer));
        } else if review {
            // The user asked for it (R): it comes to the front (see `ReviewFocus::Auto`).
            self.start_review(
                event_loop,
                q.request,
                q.asked,
                q.near,
                q.review_deadline,
                true,
            );
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
        asked_by_user: bool,
    ) {
        let review = Review::new(&asked.title, &asked.from, asked.items);
        let title = tr!("review.tab", title = review.title);
        let dialog = Dialog::Review(review);
        let Some(pane) = self.place_dialog(event_loop, dialog, title, near, asked_by_user) else {
            let _ = request
                .reply
                .send(Err(RpcError::new(RpcError::INTERNAL, "no window")));
            return;
        };
        self.dialogs.push(PendingDialog {
            pane,
            kind: DialogKind::Review,
            client: request.client,
            deadline,
            reply: request.reply,
        });
    }

    /// The pane with the keys is a review (in a tab, a split, or an overlay).
    pub(super) fn active_review(&self) -> Option<PaneId> {
        let running = self.running.as_ref()?;
        let pane = running.mux.input_pane()?;
        running.panes.get(&pane)?.review.as_ref().map(|_| pane)
    }

    /// A key in the active Review tab.
    pub(super) fn review_key(&mut self, event: &KeyEvent) {
        use crate::review::Key as R;
        let Some(pane) = self.active_review() else {
            return;
        };
        let input = dialog_input(
            &event.logical_key,
            event.physical_key,
            event.text.as_deref(),
            self.mods.control_key(),
            self.mods.shift_key(),
            self.mods.alt_key(),
        );
        match input {
            Some(DialogInput::Key(key)) => self.review_apply(pane, key),
            Some(DialogInput::Text(text)) => self.review_apply(pane, R::Text(&text)),
            Some(DialogInput::Paste) => {
                if let Some(text) = self.clipboard.paste() {
                    self.review_apply(pane, R::Paste(&text));
                }
            }
            None => {}
        }
    }

    /// Text of the input method (CJK) for the review that has the keys: into its open input. With no open input it
    /// goes nowhere (a letter there is a command). `false` = no review has the keys.
    pub(super) fn review_ime(&mut self, text: &str) -> bool {
        if self.keys_elsewhere() {
            return false;
        }
        let Some(pane) = self.active_review() else {
            return false;
        };
        let typing = self
            .running
            .as_ref()
            .and_then(|r| r.panes.get(&pane))
            .and_then(|p| p.review.as_ref())
            .is_some_and(|rp| rp.review.typing());
        if typing {
            self.review_apply(pane, crate::review::Key::Text(text));
        }
        true
    }

    /// Text from the clipboard for the review that has the keys (the `paste` action).
    pub(super) fn review_paste(&mut self, pane: PaneId, text: &str) {
        self.review_apply(pane, crate::review::Key::Paste(text));
    }

    /// One key for the review `pane`: draw it again, send it, or copy what it gives.
    fn review_apply(&mut self, pane: PaneId, key: crate::review::Key) {
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
                let answer = rp.review.result(rp.review.decision());
                return self.finish_dialog(pane, answer);
            }
            Outcome::Copy(text) => return self.copy_text(text),
        }
        self.draw_dialogs();
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
}
