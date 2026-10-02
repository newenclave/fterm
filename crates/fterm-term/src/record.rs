//! A recording of what a program wrote to its pane, for finding bugs (`FTERM_RECORD`).
//! The format is asciinema v2: a JSON header line, then `[time, "o", text]` and `[time, "r", "80x24"]`.
//! asciinema can play it, and a test can feed it to a terminal again.

use std::io::{self, Write};
use std::time::Instant;

/// The recorder of one session: the pty loop writes the output, the session the sizes.
pub type Shared = std::sync::Arc<std::sync::Mutex<Option<Recorder<io::BufWriter<std::fs::File>>>>>;

/// A recording in `path` (its folder is made).
pub fn open(
    path: &std::path::Path,
    cols: usize,
    rows: usize,
) -> io::Result<Recorder<io::BufWriter<std::fs::File>>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = io::BufWriter::new(std::fs::File::create(path)?);
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Recorder::new(file, cols, rows, unix, Instant::now())
}

/// Writes with the shared recorder; after an error it stops (one warning, not one per read).
pub fn with(
    shared: &Shared,
    f: impl FnOnce(&mut Recorder<io::BufWriter<std::fs::File>>) -> io::Result<()>,
) {
    let mut slot = shared.lock().unwrap();
    if let Some(recorder) = slot.as_mut()
        && let Err(err) = f(recorder)
    {
        tracing::warn!("the recording stops: {err}");
        *slot = None;
    }
}

pub struct Recorder<W: Write> {
    out: W,
    start: Instant,
    /// The start of a UTF-8 char that the last read cut in half.
    carry: Vec<u8>,
}

impl<W: Write> Recorder<W> {
    /// Starts a recording: writes the header (`unix_time` in seconds).
    pub fn new(
        out: W,
        cols: usize,
        rows: usize,
        unix_time: u64,
        start: Instant,
    ) -> io::Result<Self> {
        let mut recorder = Self {
            out,
            start,
            carry: Vec::new(),
        };
        let header = serde_json::json!({
            "version": 2,
            "width": cols,
            "height": rows,
            "timestamp": unix_time,
        });
        recorder.line(&header)?;
        Ok(recorder)
    }

    /// What the program wrote.
    pub fn output(&mut self, bytes: &[u8], at: Instant) -> io::Result<()> {
        let mut data = std::mem::take(&mut self.carry);
        data.extend_from_slice(bytes);
        let mut text = String::new();
        let mut rest = data.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(good) => {
                    text.push_str(good);
                    break;
                }
                Err(err) => {
                    let (good, bad) = rest.split_at(err.valid_up_to());
                    text.push_str(std::str::from_utf8(good).unwrap_or_default());
                    match err.error_len() {
                        // Not UTF-8: one replacement char, then go on.
                        Some(len) => {
                            text.push('\u{fffd}');
                            rest = &bad[len..];
                        }
                        // A char that the next read ends.
                        None => {
                            self.carry = bad.to_vec();
                            break;
                        }
                    }
                }
            }
        }
        if text.is_empty() {
            return Ok(());
        }
        let event = serde_json::json!([self.time(at), "o", text]);
        self.line(&event)
    }

    /// The pane got a new size.
    pub fn resize(&mut self, cols: usize, rows: usize, at: Instant) -> io::Result<()> {
        let event = serde_json::json!([self.time(at), "r", format!("{cols}x{rows}")]);
        self.line(&event)
    }

    /// Seconds from the start, to the microsecond.
    fn time(&self, at: Instant) -> f64 {
        at.saturating_duration_since(self.start).as_micros() as f64 / 1_000_000.0
    }

    /// One line, written at once (a crash must not lose the end of a recording).
    fn line(&mut self, value: &serde_json::Value) -> io::Result<()> {
        writeln!(self.out, "{value}")?;
        self.out.flush()
    }

    pub fn into_inner(self) -> W {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn lines(r: Recorder<Vec<u8>>) -> Vec<serde_json::Value> {
        String::from_utf8(r.into_inner())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn a_recording_starts_with_the_header() {
        let t0 = Instant::now();
        let r = Recorder::new(Vec::new(), 120, 30, 1_790_000_000, t0).unwrap();
        let lines = lines(r);
        assert_eq!(
            lines,
            [
                serde_json::json!({ "version": 2, "width": 120, "height": 30, "timestamp": 1_790_000_000u64 })
            ]
        );
    }

    #[test]
    fn output_and_resize_have_their_times() {
        let t0 = Instant::now();
        let mut r = Recorder::new(Vec::new(), 80, 24, 0, t0).unwrap();
        r.output(b"a\x1b[31mb\r\n", t0 + Duration::from_millis(500))
            .unwrap();
        r.resize(1, 24, t0 + Duration::from_millis(1250)).unwrap();
        let lines = lines(r);
        assert_eq!(lines[1], serde_json::json!([0.5, "o", "a\u{1b}[31mb\r\n"]));
        assert_eq!(lines[2], serde_json::json!([1.25, "r", "1x24"]));
    }

    #[test]
    fn a_char_cut_by_a_read_comes_together() {
        let t0 = Instant::now();
        let mut r = Recorder::new(Vec::new(), 80, 24, 0, t0).unwrap();
        // "Ж" is D0 96: the first read has only D0.
        r.output(b"x\xd0", t0).unwrap();
        r.output(b"\x96y", t0).unwrap();
        // Bytes that are not UTF-8 at all do not break the file.
        r.output(b"\xffz", t0).unwrap();
        let lines = lines(r);
        assert_eq!(lines[1][2], "x");
        assert_eq!(lines[2][2], "Жy");
        assert_eq!(lines[3][2], "\u{fffd}z");
        assert_eq!(lines.len(), 4);
    }
}
