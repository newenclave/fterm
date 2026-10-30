//! Runs a command of the config in the background (the dashboard: command tiles and buttons): the shell runs it with
//! no window, the output is kept as text, and it is stopped after a timeout.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What a command did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunResult {
    /// The exit code (`None` = no code: it was stopped, or it did not start).
    pub exit: Option<i32>,
    /// The output and the errors, as text with no escape codes.
    pub output: String,
    /// Why it did not run or did not end (cannot start, the timeout).
    pub error: Option<String>,
}

impl RunResult {
    pub fn ok(&self) -> bool {
        self.exit == Some(0) && self.error.is_none()
    }

    /// The lines of the output with no spaces at their ends, the empty ones at the end removed.
    pub fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .output
            .lines()
            .map(|l| l.trim_end().to_owned())
            .collect();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        lines
    }
}

/// Runs `command` with `shell` (see `shell_run_args`) and waits for it, at most `timeout`. Blocks: call it on a
/// thread of its own.
pub fn run(shell: &str, args: &[String], timeout: Duration) -> RunResult {
    let mut cmd = Command::new(shell);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// No console window for the shell.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            return RunResult {
                error: Some(format!("{shell}: {err}")),
                ..Default::default()
            };
        }
    };
    // The pipes are read on threads, so a big output does not stop the program. A program that the command started
    // may keep a pipe open after the shell ends, so the result does not wait long for them.
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            let _ = tx.send(bytes);
        });
        rx
    };
    let out = read(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = read(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let start = Instant::now();
    let (exit, error) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code(), None),
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break (None, Some(format!("stopped after {} s", timeout.as_secs())));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => break (None, Some(e.to_string())),
        }
    };
    let wait = Duration::from_secs(1);
    let mut bytes = out.recv_timeout(wait).unwrap_or_default();
    bytes.extend(err.recv_timeout(wait).unwrap_or_default());
    RunResult {
        exit,
        output: plain_text(&String::from_utf8_lossy(&bytes)),
        error,
    }
}

/// `text` with no escape codes (colors, titles) and no `\r`.
pub fn plain_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: up to a letter (or `@`..`~`).
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {}
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shell of the tests and its arguments for `command`.
    fn shell(command: &str) -> (String, Vec<String>) {
        if cfg!(windows) {
            ("cmd.exe".into(), vec!["/c".into(), command.into()])
        } else {
            ("sh".into(), vec!["-c".into(), command.into()])
        }
    }

    #[test]
    fn a_command_gives_its_output_and_exit_code() {
        let (sh, args) = shell("echo hello && exit 3");
        let r = run(&sh, &args, Duration::from_secs(20));
        assert_eq!(r.exit, Some(3), "{r:?}");
        assert_eq!(r.lines(), ["hello"], "{r:?}");
        assert!(!r.ok());
        let (sh, args) = shell("echo ok");
        assert!(run(&sh, &args, Duration::from_secs(20)).ok());
    }

    #[test]
    fn a_command_that_takes_too_long_is_stopped() {
        let (sh, args) = if cfg!(windows) {
            shell("ping -n 30 127.0.0.1")
        } else {
            shell("sleep 30")
        };
        let start = Instant::now();
        let r = run(&sh, &args, Duration::from_millis(500));
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(r.exit, None);
        assert!(
            r.error.as_deref().is_some_and(|e| e.contains("stopped")),
            "{r:?}"
        );
    }

    #[test]
    fn a_program_that_is_not_there() {
        let r = run("no-such-program-fterm", &[], Duration::from_secs(5));
        assert!(r.error.is_some() && r.exit.is_none(), "{r:?}");
    }

    #[test]
    fn escape_codes_go_away() {
        assert_eq!(plain_text("\u{1b}[31mred\u{1b}[0m ok\r\n"), "red ok\n");
        assert_eq!(plain_text("\u{1b}]0;title\u{7}text"), "text");
        assert_eq!(
            plain_text("\u{1b}]8;;http://x\u{1b}\\link\u{1b}]8;;\u{1b}\\"),
            "link"
        );
        assert_eq!(plain_text("a\tb\u{8}"), "a\tb");
    }
}
