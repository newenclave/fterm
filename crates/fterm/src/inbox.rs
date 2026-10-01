//! Messages between agents (Phase 7): each pane has an inbox. Nothing is typed into the other pane;
//! its agent reads the messages with `read_messages` (an MCP tool).

use std::collections::HashMap;

use fterm_mux::PaneId;

/// The longest message, in bytes.
pub const MAX_MESSAGE: usize = 64 * 1024;
/// The most messages in one inbox (older ones go away).
pub const MAX_IN_BOX: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub id: u64,
    /// The pane of the sender (`None` = a script outside of fterm).
    pub from: Option<PaneId>,
    /// Who sent it (the client name, for example "claude" or "ftermctl").
    pub from_name: String,
    pub text: String,
    /// Unix ms.
    pub time: u64,
    pub read: bool,
}

#[derive(Default)]
pub struct Inbox {
    boxes: HashMap<PaneId, Vec<Message>>,
    next_id: u64,
}

impl Inbox {
    /// Puts a message into the inbox of `to`. Returns its id.
    pub fn send(
        &mut self,
        to: PaneId,
        from: Option<PaneId>,
        from_name: &str,
        text: &str,
        time: u64,
    ) -> u64 {
        self.next_id += 1;
        let list = self.boxes.entry(to).or_default();
        list.push(Message {
            id: self.next_id,
            from,
            from_name: from_name.to_owned(),
            text: text.to_owned(),
            time,
            read: false,
        });
        let extra = list.len().saturating_sub(MAX_IN_BOX);
        list.drain(..extra);
        self.next_id
    }

    /// The messages of a pane, oldest first. `unread_only` leaves out the read ones; `mark_read` marks the
    /// ones it gives as read.
    pub fn read(&mut self, pane: PaneId, unread_only: bool, mark_read: bool) -> Vec<Message> {
        let Some(list) = self.boxes.get_mut(&pane) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for message in list.iter_mut().filter(|m| !unread_only || !m.read) {
            out.push(message.clone());
            if mark_read {
                message.read = true;
            }
        }
        out
    }

    pub fn unread(&self, pane: PaneId) -> usize {
        self.boxes
            .get(&pane)
            .map_or(0, |list| list.iter().filter(|m| !m.read).count())
    }

    /// The pane closed: its inbox goes away.
    pub fn remove(&mut self, pane: PaneId) {
        self.boxes.remove(&pane);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: PaneId = PaneId(1);
    const B: PaneId = PaneId(2);

    #[test]
    fn send_and_read() {
        let mut inbox = Inbox::default();
        let first = inbox.send(B, Some(A), "claude", "please review", 10);
        let second = inbox.send(B, None, "ftermctl", "build is green", 20);
        assert_ne!(first, second);
        assert_eq!(inbox.unread(B), 2);
        assert_eq!(inbox.unread(A), 0);
        let list = inbox.read(B, true, true);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].text, "please review", "oldest first");
        assert_eq!(
            (list[0].from, list[0].from_name.as_str()),
            (Some(A), "claude")
        );
        assert_eq!(inbox.unread(B), 0, "read now");
        assert!(inbox.read(B, true, true).is_empty(), "no unread ones left");
        assert_eq!(
            inbox.read(B, false, false).len(),
            2,
            "all of them are still there"
        );
    }

    #[test]
    fn read_without_marking() {
        let mut inbox = Inbox::default();
        inbox.send(A, None, "x", "hi", 1);
        assert_eq!(inbox.read(A, true, false).len(), 1);
        assert_eq!(inbox.unread(A), 1, "a peek does not mark");
    }

    #[test]
    fn an_inbox_keeps_the_newest() {
        let mut inbox = Inbox::default();
        for i in 0..(MAX_IN_BOX + 5) {
            inbox.send(A, None, "x", &format!("m{i}"), i as u64);
        }
        let list = inbox.read(A, false, false);
        assert_eq!(list.len(), MAX_IN_BOX);
        assert_eq!(list[0].text, "m5");
    }

    #[test]
    fn a_closed_pane_loses_its_inbox() {
        let mut inbox = Inbox::default();
        inbox.send(A, None, "x", "hi", 1);
        inbox.remove(A);
        assert_eq!(inbox.unread(A), 0);
    }
}
