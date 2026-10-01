//! Server-sent events: `event: name` and `data: ...` lines, and an empty line ends one event.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Reads the stream line by line.
#[derive(Default)]
pub struct Parser {
    event: Option<String>,
    data: Vec<String>,
}

impl Parser {
    /// One line (without its end). Gives an event when the line ends one.
    pub fn line(&mut self, line: &str) -> Option<SseEvent> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return self.finish();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
        None
    }

    /// The stream ended: an event without its empty line still counts.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if self.data.is_empty() && self.event.is_none() {
            return None;
        }
        Some(SseEvent {
            event: self.event.take(),
            data: std::mem::take(&mut self.data).join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(text: &str) -> Vec<SseEvent> {
        let mut p = Parser::default();
        let mut out: Vec<SseEvent> = text.lines().filter_map(|l| p.line(l)).collect();
        out.extend(p.finish());
        out
    }

    #[test]
    fn events_with_names_and_data() {
        let events =
            feed("event: content_block_delta\ndata: {\"a\":1}\n\nevent: ping\ndata: {}\n\n");
        assert_eq!(
            events,
            [
                SseEvent {
                    event: Some("content_block_delta".into()),
                    data: "{\"a\":1}".into()
                },
                SseEvent {
                    event: Some("ping".into()),
                    data: "{}".into()
                },
            ]
        );
    }

    #[test]
    fn data_only_comments_and_many_lines() {
        let events = feed(": a comment\ndata: one\ndata: two\n\ndata:three\n\n");
        assert_eq!(
            events[0],
            SseEvent {
                event: None,
                data: "one\ntwo".into()
            }
        );
        assert_eq!(events[1].data, "three", "no space after the colon is fine");
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn windows_line_ends_and_the_last_event() {
        let mut p = Parser::default();
        assert_eq!(p.line("data: x\r"), None);
        assert_eq!(p.line("\r").unwrap().data, "x");
        assert_eq!(p.line("data: last"), None);
        assert_eq!(p.finish().unwrap().data, "last", "no empty line at the end");
        assert_eq!(p.finish(), None);
    }
}
