//! Finds the OSC sequences that alacritty does not handle (shell integration, notifications, agent state,
//! pictures). The scanner only reads the bytes; the bytes still go to the alacritty parser, except the body of a
//! picture (`hidden`): alacritty does not need it, and it would keep a copy of megabytes.

use std::ops::Range;
use std::sync::Arc;

use crate::picture::{FileArgs, MAX_PAYLOAD, Picture};

/// The longest OSC payload we keep. Longer sequences are dropped. A picture (`1337;File=`) may have `MAX_PAYLOAD`.
pub const MAX_OSC: usize = 8 * 1024;

/// The start of a picture sequence (iTerm2).
const FILE: &[u8] = b"1337;File=";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OscEvent {
    /// The current folder (OSC 7, or ConEmu OSC 9;9).
    Cwd(String),
    /// A notification (OSC 9, OSC 99, OSC 777;notify).
    Notify { title: Option<String>, body: String },
    /// Shell integration marks (OSC 133).
    Prompt(PromptMark),
    /// An agent state from a hook: OSC 777;fterm-agent;<state>;<message>.
    Agent { state: String, message: String },
    /// A color for the tab of the pane: OSC 777;fterm-tab;color;<#rrggbb or none>.
    TabColor(String),
    /// The command line that the user typed (OSC 633;E, the VS Code format). It comes before 133;C.
    CommandLine(String),
    /// Not from the shell: the pty loop adds it right after 133;B. The place of the cursor at that moment,
    /// where the typed text starts. `line` counts from the top of the history (history size + screen line).
    InputStart { line: usize, column: usize },
    /// Not from the shell: the pty loop adds it right after 133;C and 133;D. The place of the cursor,
    /// so fterm knows the lines of the command output.
    OutputMark { line: usize, column: usize },
    /// A picture file from the output (OSC 1337;File=, `inline=1`), not decoded yet. The pty loop decodes it and
    /// gives `Picture` in its place.
    #[serde(skip)]
    PictureFile { args: FileArgs, data: Arc<Vec<u8>> },
    /// A picture in the output: its top-left cell has the mark `picture_uri(id)` (a hyperlink), so it moves with
    /// its cell and goes away with it.
    #[serde(skip)]
    Picture(PictureMark),
}

/// A decoded picture and the cells it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PictureMark {
    pub id: u32,
    pub picture: Picture,
    pub columns: usize,
    pub rows: usize,
}

/// The hyperlink that marks the top-left cell of picture `id`.
pub fn picture_uri(id: u32) -> String {
    format!("{PICTURE_SCHEME}{id}")
}

/// The id of a picture mark (`None` for a real link).
pub fn picture_id(uri: &str) -> Option<u32> {
    uri.strip_prefix(PICTURE_SCHEME)?.parse().ok()
}

const PICTURE_SCHEME: &str = "fterm-picture:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PromptMark {
    /// A: the prompt starts.
    PromptStart,
    /// B: the user types the command.
    CommandStart,
    /// C: the command runs.
    CommandExecuted,
    /// D: the command ended, with its exit code when the shell sends it.
    CommandFinished(Option<i32>),
}

/// Reads bytes in chunks. A sequence can start in one chunk and end in the next one.
#[derive(Default)]
pub struct Scanner {
    state: State,
    payload: Vec<u8>,
    /// Inside the body of a picture (after `1337;File=`): its bytes are hidden from the parser.
    file: bool,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    /// After ESC.
    Escape,
    /// Inside `ESC ]`.
    Osc,
    /// Inside OSC, after ESC (maybe the end `ESC \`).
    OscEscape,
    /// The payload is too long: skip to the end of this sequence.
    Skip,
    SkipEscape,
}

impl Scanner {
    /// Reads `bytes` and adds the events it finds to `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<OscEvent>) {
        let mut at = Vec::new();
        self.feed_at(bytes, &mut at);
        out.extend(at.into_iter().map(|(_, event)| event));
    }

    /// Like `feed`, but each event comes with the offset just after its sequence in `bytes`.
    /// So the parser can run up to that place first (for example, to know where the cursor is at 133;B).
    pub fn feed_at(&mut self, bytes: &[u8], out: &mut Vec<(usize, OscEvent)>) {
        self.feed_parts(bytes, out, &mut Vec::new());
    }

    /// Like `feed_at`, and `hidden` gets the byte ranges that the parser must not get (the body of a picture; never
    /// an ESC or a BEL, so the parser still sees the end of the sequence).
    pub fn feed_parts(
        &mut self,
        bytes: &[u8],
        out: &mut Vec<(usize, OscEvent)>,
        hidden: &mut Vec<Range<usize>>,
    ) {
        const ESC: u8 = 0x1b;
        const BEL: u8 = 0x07;
        let mut hide_from = self.file.then_some(0);
        for (i, &b) in bytes.iter().enumerate() {
            let end = i + 1;
            // A hidden range ends before an ESC or a BEL.
            if let Some(from) = hide_from
                && (b == ESC || b == BEL)
            {
                if from < i {
                    hidden.push(from..i);
                }
                hide_from = None;
            }
            let limit = if self.file { MAX_PAYLOAD } else { MAX_OSC };
            self.state = match (self.state, b) {
                (State::Ground, ESC) => State::Escape,
                (State::Ground, _) => State::Ground,
                (State::Escape, b']') => {
                    self.payload.clear();
                    self.file = false;
                    State::Osc
                }
                (State::Escape, ESC) => State::Escape,
                (State::Escape, _) => State::Ground,
                (State::Osc, BEL) => self.finish(end, out),
                (State::Osc, ESC) => State::OscEscape,
                (State::Osc, _) if self.payload.len() >= limit => {
                    self.payload = Vec::new();
                    State::Skip
                }
                (State::Osc, _) => {
                    self.payload.push(b);
                    if !self.file && self.payload == FILE {
                        self.file = true;
                    }
                    State::Osc
                }
                (State::OscEscape, b'\\') => self.finish(end, out),
                // ESC in the middle ends the OSC without a result; it can start a new one.
                (State::OscEscape, b']') => {
                    self.payload.clear();
                    self.file = false;
                    State::Osc
                }
                (State::OscEscape, _) => {
                    self.file = false;
                    State::Ground
                }
                (State::Skip, BEL) => {
                    self.file = false;
                    State::Ground
                }
                (State::Skip, ESC) => State::SkipEscape,
                (State::Skip, _) => State::Skip,
                (State::SkipEscape, _) => {
                    self.file = false;
                    State::Ground
                }
            };
            // The body of a picture starts after `1337;File=`, and goes on after an ESC that did not end it.
            if self.file && hide_from.is_none() && b != ESC && b != BEL {
                hide_from = Some(end);
            }
        }
        if let Some(from) = hide_from
            && self.file
            && from < bytes.len()
        {
            hidden.push(from..bytes.len());
        }
    }

    fn finish(&mut self, end: usize, out: &mut Vec<(usize, OscEvent)>) -> State {
        let event = if self.file {
            self.file = false;
            parse_picture(&self.payload["1337;".len()..])
        } else {
            parse(&String::from_utf8_lossy(&self.payload))
        };
        if let Some(event) = event {
            out.push((end, event));
        }
        if self.payload.capacity() > MAX_OSC {
            self.payload = Vec::new();
        } else {
            self.payload.clear();
        }
        State::Ground
    }
}

/// `File=<args>:<base64>`: a picture to show (`inline=1`, good base64), or nothing.
fn parse_picture(text: &[u8]) -> Option<OscEvent> {
    let (args, data) = crate::picture::parse_file(text)?;
    if !args.inline {
        return None;
    }
    let data = crate::picture::base64_decode(data)?;
    Some(OscEvent::PictureFile {
        args,
        data: Arc::new(data),
    })
}

/// Turns one OSC payload (without `ESC ]` and the end) into an event.
fn parse(payload: &str) -> Option<OscEvent> {
    let (code, rest) = payload.split_once(';').unwrap_or((payload, ""));
    match code {
        "7" => Some(OscEvent::Cwd(file_uri_path(rest))),
        "9" => {
            if let Some(path) = rest.strip_prefix("9;") {
                return Some(OscEvent::Cwd(path.trim_matches('"').to_owned()));
            }
            // 9;4 is a progress bar (Windows Terminal), not a notification.
            if rest.starts_with("4;") || rest.is_empty() {
                return None;
            }
            Some(OscEvent::Notify {
                title: None,
                body: rest.to_owned(),
            })
        }
        "99" => {
            let (_metadata, body) = rest.split_once(';').unwrap_or(("", rest));
            (!body.is_empty()).then(|| OscEvent::Notify {
                title: None,
                body: body.to_owned(),
            })
        }
        "777" => {
            let mut parts = rest.splitn(3, ';');
            match parts.next()? {
                "notify" => Some(OscEvent::Notify {
                    title: parts.next().map(str::to_owned).filter(|t| !t.is_empty()),
                    body: parts.next().unwrap_or_default().to_owned(),
                }),
                "fterm-agent" => Some(OscEvent::Agent {
                    state: parts.next()?.to_owned(),
                    message: parts.next().unwrap_or_default().to_owned(),
                }),
                "fterm-tab" => match parts.next()? {
                    "color" => Some(OscEvent::TabColor(parts.next()?.trim().to_owned())),
                    _ => None,
                },
                _ => None,
            }
        }
        "633" if rest.starts_with("E;") => {
            // E;<command>[;<nonce>]: `;` in the command is written as \x3b, so the first `;` ends it.
            let command = rest[2..].split(';').next().unwrap_or_default();
            Some(OscEvent::CommandLine(unescape_vscode(command)))
        }
        // VS Code sends the same A/B/C/D marks with 633.
        "133" | "633" => {
            let mut parts = rest.split(';');
            let mark = match parts.next()? {
                "A" => PromptMark::PromptStart,
                "B" => PromptMark::CommandStart,
                "C" => PromptMark::CommandExecuted,
                "D" => PromptMark::CommandFinished(parts.next().and_then(|code| code.parse().ok())),
                _ => return None,
            };
            Some(OscEvent::Prompt(mark))
        }
        _ => None,
    }
}

/// The VS Code escape: `\\` is `\`, `\xNN` is the byte NN. Anything else stays as it is.
fn unescape_vscode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            if bytes.get(i + 1) == Some(&b'\\') {
                out.push(b'\\');
                i += 2;
                continue;
            }
            if bytes.get(i + 1) == Some(&b'x')
                && let Some(hex) = text.get(i + 2..i + 4)
                && let Ok(byte) = u8::from_str_radix(hex, 16)
            {
                out.push(byte);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `file://host/home/me/my%20code` -> `/home/me/my code`; `file://pc/C:/x` -> `C:/x`.
fn file_uri_path(uri: &str) -> String {
    let Some(rest) = uri.strip_prefix("file://") else {
        return percent_decode(uri);
    };
    // Skip the host name.
    let path = rest.find('/').map_or(rest, |i| &rest[i..]);
    let path = percent_decode(path);
    let bytes = path.as_bytes();
    // "/C:/..." is a Windows path.
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        path[1..].to_owned()
    } else {
        path
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(hex) = text.get(i + 1..i + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<OscEvent> {
        let mut scanner = Scanner::default();
        let mut out = Vec::new();
        for chunk in chunks {
            scanner.feed(chunk, &mut out);
        }
        out
    }

    #[test]
    fn command_line_from_633_e() {
        // The VS Code format: `\\` is `\`, `\x3b` is `;`, `\x0a` is a new line.
        let seq: &[u8] = b"\x1b]633;E;git commit -m \"a\\x3bb\" C:\\\\x\\x0ay\x07";
        assert_eq!(
            scan(&[seq]),
            [OscEvent::CommandLine(
                "git commit -m \"a;b\" C:\\x\ny".into()
            )]
        );
        // A nonce after the command (VS Code sends one) is not part of the command.
        assert_eq!(
            scan(&[b"\x1b]633;E;ls -la;abc123\x07"]),
            [OscEvent::CommandLine("ls -la".into())]
        );
        // A broken escape stays as it is.
        assert_eq!(
            scan(&[b"\x1b]633;E;echo \\xZZ\x07"]),
            [OscEvent::CommandLine("echo \\xZZ".into())]
        );
    }

    #[test]
    fn vs_code_marks_are_prompt_marks() {
        assert_eq!(
            scan(&[b"\x1b]633;A\x07\x1b]633;B\x07\x1b]633;C\x07\x1b]633;D;3\x07"]),
            [
                OscEvent::Prompt(PromptMark::PromptStart),
                OscEvent::Prompt(PromptMark::CommandStart),
                OscEvent::Prompt(PromptMark::CommandExecuted),
                OscEvent::Prompt(PromptMark::CommandFinished(Some(3))),
            ]
        );
    }

    #[test]
    fn events_know_where_they_end() {
        let mut scanner = Scanner::default();
        let mut out = Vec::new();
        let bytes: &[u8] = b"ab\x1b]133;A\x07PS> \x1b]133;B\x1b\\x";
        scanner.feed_at(bytes, &mut out);
        assert_eq!(
            out,
            [
                (10, OscEvent::Prompt(PromptMark::PromptStart)),
                (23, OscEvent::Prompt(PromptMark::CommandStart)),
            ]
        );
        assert_eq!(&bytes[..10], b"ab\x1b]133;A\x07");
        assert_eq!(bytes[23], b'x');
    }

    fn notify(title: Option<&str>, body: &str) -> OscEvent {
        OscEvent::Notify {
            title: title.map(str::to_owned),
            body: body.to_owned(),
        }
    }

    #[test]
    fn osc_9_notification_with_bel_and_st() {
        assert_eq!(
            scan(&[b"\x1b]9;Build done\x07"]),
            [notify(None, "Build done")]
        );
        assert_eq!(
            scan(&[b"\x1b]9;Build done\x1b\\"]),
            [notify(None, "Build done")]
        );
    }

    #[test]
    fn osc_777_notify_has_a_title() {
        assert_eq!(
            scan(&[b"\x1b]777;notify;Claude;Waiting for you\x07"]),
            [notify(Some("Claude"), "Waiting for you")]
        );
        // A body with ";" in it stays whole.
        assert_eq!(
            scan(&[b"\x1b]777;notify;T;a;b;c\x07"]),
            [notify(Some("T"), "a;b;c")]
        );
    }

    #[test]
    fn osc_99_kitty_notification() {
        assert_eq!(
            scan(&[b"\x1b]99;;Hello kitty\x1b\\"]),
            [notify(None, "Hello kitty")]
        );
        assert_eq!(
            scan(&[b"\x1b]99;i=1:d=0;Hello\x1b\\"]),
            [notify(None, "Hello")]
        );
    }

    #[test]
    fn tab_color() {
        assert_eq!(
            scan(&[b"\x1b]777;fterm-tab;color;#f38ba8\x07"]),
            [OscEvent::TabColor("#f38ba8".into())]
        );
        assert_eq!(
            scan(&[b"\x1b]777;fterm-tab;color;none\x1b\\"]),
            [OscEvent::TabColor("none".into())]
        );
        assert_eq!(scan(&[b"\x1b]777;fterm-tab;size;9\x07"]), []);
    }

    #[test]
    fn agent_state() {
        assert_eq!(
            scan(&[b"\x1b]777;fterm-agent;waiting;Claude needs your OK\x07"]),
            [OscEvent::Agent {
                state: "waiting".into(),
                message: "Claude needs your OK".into()
            }]
        );
        assert_eq!(
            scan(&[b"\x1b]777;fterm-agent;working\x07"]),
            [OscEvent::Agent {
                state: "working".into(),
                message: String::new()
            }]
        );
    }

    #[test]
    fn cwd_from_osc_7() {
        assert_eq!(
            scan(&[b"\x1b]7;file://myhost/home/me/my%20code\x07"]),
            [OscEvent::Cwd("/home/me/my code".into())]
        );
        // Windows paths come as file://host/C:/...
        assert_eq!(
            scan(&[b"\x1b]7;file://pc/C:/work/fterm\x07"]),
            [OscEvent::Cwd("C:/work/fterm".into())]
        );
        // ConEmu style: OSC 9;9;"path".
        assert_eq!(
            scan(&[b"\x1b]9;9;\"C:\\work\"\x07"]),
            [OscEvent::Cwd("C:\\work".into())]
        );
    }

    #[test]
    fn shell_integration_marks() {
        assert_eq!(
            scan(&[b"\x1b]133;A\x07\x1b]133;B\x07ls\r\n\x1b]133;C\x07out\x1b]133;D;2\x07\x1b]133;D\x07"]),
            [
                OscEvent::Prompt(PromptMark::PromptStart),
                OscEvent::Prompt(PromptMark::CommandStart),
                OscEvent::Prompt(PromptMark::CommandExecuted),
                OscEvent::Prompt(PromptMark::CommandFinished(Some(2))),
                OscEvent::Prompt(PromptMark::CommandFinished(None)),
            ]
        );
        // Extra options after the mark are fine.
        assert_eq!(
            scan(&[b"\x1b]133;D;0;aid=12\x07"]),
            [OscEvent::Prompt(PromptMark::CommandFinished(Some(0)))]
        );
    }

    #[test]
    fn sequence_cut_into_many_chunks() {
        let events = scan(&[b"text \x1b", b"]9;He", b"llo\x1b", b"\\ more"]);
        assert_eq!(events, [notify(None, "Hello")]);
    }

    #[test]
    fn other_sequences_and_text_give_nothing() {
        // Titles, colors, hyperlinks and CSI are for alacritty, not for us.
        assert!(
            scan(&[b"\x1b]0;title\x07\x1b]8;;http://x\x07link\x1b]8;;\x07\x1b[31mred\x1b[0m"])
                .is_empty()
        );
        assert!(
            scan(&[b"\x1b]9;4;1;50\x07"]).is_empty(),
            "progress is not a notification"
        );
        assert!(scan(&[b"just text \x07 bell"]).is_empty());
    }

    #[test]
    fn too_long_sequence_is_dropped_and_the_next_one_works() {
        let mut long = b"\x1b]9;".to_vec();
        long.extend(std::iter::repeat_n(b'x', MAX_OSC + 10));
        long.extend(b"\x07\x1b]9;ok\x07");
        assert_eq!(scan(&[&long]), [notify(None, "ok")]);
    }

    /// Scans `chunks` and gives the events and the bytes that the parser gets (all but the hidden ones).
    fn scan_parts(chunks: &[&[u8]]) -> (Vec<OscEvent>, Vec<u8>) {
        let mut scanner = Scanner::default();
        let (mut events, mut seen) = (Vec::new(), Vec::new());
        for chunk in chunks {
            let (mut out, mut hidden) = (Vec::new(), Vec::new());
            scanner.feed_parts(chunk, &mut out, &mut hidden);
            events.extend(out.into_iter().map(|(_, e)| e));
            let mut at = 0;
            for range in hidden {
                assert!(range.start >= at && range.end <= chunk.len(), "{range:?}");
                seen.extend_from_slice(&chunk[at..range.start]);
                at = range.end;
            }
            seen.extend_from_slice(&chunk[at..]);
        }
        (events, seen)
    }

    fn picture(data: &[u8]) -> OscEvent {
        OscEvent::PictureFile {
            args: crate::picture::parse_file(b"File=inline=1:").unwrap().0,
            data: Arc::new(data.to_vec()),
        }
    }

    #[test]
    fn a_picture_is_found_and_its_body_is_hidden_from_the_parser() {
        let (events, seen) = scan_parts(&[b"a\x1b]1337;File=inline=1:QUJD\x07b"]);
        assert_eq!(events, [picture(b"ABC")]);
        assert_eq!(
            seen, b"a\x1b]1337;File=\x07b",
            "the parser sees only the start and the end"
        );
        // The ST end (`ESC \`) works too, and the ESC is not hidden.
        let (events, seen) = scan_parts(&[b"\x1b]1337;File=inline=1:QUJD\x1b\\x"]);
        assert_eq!(events, [picture(b"ABC")]);
        assert_eq!(seen, b"\x1b]1337;File=\x1b\\x");
    }

    #[test]
    fn a_picture_cut_into_many_chunks() {
        let (events, seen) = scan_parts(&[
            b"x\x1b]1337;Fi",
            b"le=inl",
            b"ine=1:QU",
            b"JD",
            b"\x1b",
            b"\\y",
        ]);
        assert_eq!(events, [picture(b"ABC")]);
        assert_eq!(seen, b"x\x1b]1337;File=\x1b\\y");
    }

    #[test]
    fn a_picture_longer_than_other_osc_sequences() {
        // 100 KiB of base64: much more than MAX_OSC.
        let body = "QUJD".repeat(25 * 1024);
        let seq = format!("\x1b]1337;File=inline=1:{body}\x07");
        let (events, seen) = scan_parts(&[seq.as_bytes()]);
        assert_eq!(events.len(), 1);
        let OscEvent::PictureFile { data, .. } = &events[0] else {
            panic!("{events:?}");
        };
        assert_eq!(data.len(), 75 * 1024);
        assert_eq!(seen, b"\x1b]1337;File=\x07");
    }

    #[test]
    fn not_a_picture_to_show() {
        // A download (no inline=1), bad base64, and other 1337 sequences give nothing; the next sequence works.
        let (events, _) = scan_parts(&[
            b"\x1b]1337;File=name=eA==:QUJD\x07\x1b]1337;File=inline=1:Q*\x07\x1b]1337;SetMark\x07\x1b]9;ok\x07",
        ]);
        assert_eq!(events, [notify(None, "ok")]);
    }

    #[test]
    fn a_picture_over_the_limit_is_dropped_and_still_hidden() {
        let mut long = b"\x1b]1337;File=inline=1:".to_vec();
        long.extend(std::iter::repeat_n(b'Q', MAX_PAYLOAD + 10));
        long.extend(b"\x07\x1b]9;ok\x07");
        let (events, seen) = scan_parts(&[&long]);
        assert_eq!(events, [notify(None, "ok")]);
        assert_eq!(seen, b"\x1b]1337;File=\x07\x1b]9;ok\x07");
    }

    #[test]
    fn picture_marks() {
        assert_eq!(picture_id(&picture_uri(42)), Some(42));
        assert_eq!(picture_id("https://example.com"), None);
        assert_eq!(picture_id("fterm-picture:x"), None);
    }

    #[test]
    fn utf8_text_in_notifications() {
        assert_eq!(
            scan(&["\x1b]9;Готово ✅\x07".as_bytes()]),
            [notify(None, "Готово ✅")]
        );
    }

    #[test]
    fn many_events_in_one_chunk_keep_their_order() {
        let events = scan(&[b"\x1b]9;one\x07\x1b]7;file://h/tmp\x07\x1b]9;two\x07"]);
        assert_eq!(
            events,
            [
                notify(None, "one"),
                OscEvent::Cwd("/tmp".into()),
                notify(None, "two")
            ]
        );
    }
}
