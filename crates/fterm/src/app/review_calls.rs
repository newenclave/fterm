//! A Review tab: an agent (or a script) asks the user to check a plan item by item, and waits for the answer.

use std::time::Instant;

use fterm_api::RpcError;
use fterm_api::server::ClientId;
use serde_json::Value;

use super::*;
use crate::api::ApiRequest;
use crate::review::{Decision, Outcome, Review};

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

impl App {
    /// `review`: opens a Review tab; the answer comes when the user sends it, closes the tab, or the time ends.
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
            deadline: Instant::now() + asked.timeout,
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
