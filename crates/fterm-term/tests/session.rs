//! These tests start real processes in a real pty (ConPTY on Windows).

use std::sync::mpsc;
use std::time::Duration;

use fterm_term::alacritty_terminal::grid::Dimensions;
use fterm_term::osc::{OscEvent, PromptMark};
use fterm_term::session::{Session, SessionOptions, TermEvent};
use fterm_term::size::GridSize;

/// How long a test waits for a shell. A cold PowerShell 5.1 or a first MSYS start is slow,
/// and on a CI runner even slower.
fn timeout() -> Duration {
    if std::env::var_os("CI").is_some() {
        Duration::from_secs(90)
    } else {
        Duration::from_secs(40)
    }
}

fn spawn(options: SessionOptions) -> (Session, mpsc::Receiver<TermEvent>) {
    let (tx, rx) = mpsc::channel();
    let session = Session::spawn(options, GridSize::new(80, 24), (8, 16), move |event| {
        let _ = tx.send(event);
    })
    .expect("cannot start the session");
    (session, rx)
}

fn wait_for_exit(session: &Session, rx: &mpsc::Receiver<TermEvent>) {
    let deadline = std::time::Instant::now() + timeout();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(TermEvent::Exit) => return,
            Ok(_) => {}
            Err(err) => panic!(
                "no Exit event: {err}
screen:
{}",
                session.screen_text()
            ),
        }
    }
}

fn echo_command() -> SessionOptions {
    if cfg!(windows) {
        // No wait here: the command ends at once, and its output must still be there.
        SessionOptions::command("cmd.exe", ["/c", "echo fterm-ok"])
    } else {
        SessionOptions::command("sh", ["-c", "echo fterm-ok"])
    }
}

#[test]
fn output_of_a_command_is_in_the_grid() {
    let (session, rx) = spawn(echo_command());
    wait_for_exit(&session, &rx);
    let text = session.screen_text();
    assert!(text.contains("fterm-ok"), "screen text was:\n{text}");
}

#[test]
fn shell_ends_after_exit_command() {
    let (session, rx) = spawn(SessionOptions::default());
    session.write(b"exit\r".to_vec());
    wait_for_exit(&session, &rx);
}

#[test]
fn resize_changes_the_grid_size() {
    let (session, _rx) = spawn(SessionOptions::default());
    session.resize(GridSize::new(100, 30), (8, 16));
    assert_eq!(session.grid_size(), GridSize::new(100, 30));
    let (columns, rows) = session.with_term(|term| (term.columns(), term.screen_lines()));
    assert_eq!((columns, rows), (100, 30));
    session.write(b"exit\r".to_vec());
}

#[test]
fn session_knows_its_shell_pid_and_program() {
    let options = if cfg!(windows) {
        SessionOptions::command("cmd.exe", ["/k"])
    } else {
        SessionOptions::command("sh", Vec::<String>::new())
    };
    let (session, _rx) = spawn(options);
    let pid = session.pid().expect("no pid");
    assert!(pid > 0);
    let expected = if cfg!(windows) { "cmd" } else { "sh" };
    assert_eq!(session.program(), expected);

    // Run a program in the shell: it shows up as a child of the shell.
    let (command, name) = if cfg!(windows) {
        ("ping -n 5 127.0.0.1\r", "ping")
    } else {
        ("sleep 5\r", "sleep")
    };
    session.write(command.as_bytes().to_vec());
    std::thread::sleep(Duration::from_millis(1500));
    let children = fterm_term::process::running_children(pid);
    assert!(
        children.iter().any(|c| c.to_lowercase().starts_with(name)),
        "{children:?}"
    );
    session.write(b"\x03exit\r".to_vec());
}

#[test]
fn session_starts_in_its_folder_with_its_env() {
    let dir = std::env::temp_dir();
    let mut options = if cfg!(windows) {
        SessionOptions::command("cmd.exe", ["/c", "cd & echo %FTERM_TEST%"])
    } else {
        SessionOptions::command("sh", ["-c", "pwd; echo $FTERM_TEST"])
    };
    options.cwd = Some(dir.clone());
    options.env = vec![("FTERM_TEST".to_owned(), "hello-env".to_owned())];
    let (session, rx) = spawn(options);
    wait_for_exit(&session, &rx);
    let text = session.screen_text();
    assert!(text.contains("hello-env"), "{text}");
    let dir_name = dir.file_name().unwrap().to_string_lossy().to_lowercase();
    assert!(text.to_lowercase().contains(&dir_name), "{text}");
}

#[test]
fn scrollback_size_comes_from_the_options() {
    let mut options = if cfg!(windows) {
        SessionOptions::command("cmd.exe", ["/c", "for /l %i in (1,1,200) do @echo line %i"])
    } else {
        SessionOptions::command("sh", ["-c", "seq 200"])
    };
    options.scrollback = 50;
    let (session, rx) = spawn(options);
    wait_for_exit(&session, &rx);
    // 200 lines on a 24-line screen: the history keeps only 50 of them.
    let history = session.with_term(|term| term.history_size());
    assert_eq!(history, 50);
}

#[test]
fn fast_commands_do_not_lose_their_output() {
    // Many runs, because the old bug did not happen every time.
    for _ in 0..5 {
        let (session, rx) = spawn(echo_command());
        wait_for_exit(&session, &rx);
        let text = session.screen_text();
        assert!(
            text.contains("fterm-ok"),
            "screen text was:
{text}"
        );
    }
}

/// All events until Exit.
fn events_until_exit(session: &Session, rx: &mpsc::Receiver<TermEvent>) -> Vec<TermEvent> {
    let deadline = std::time::Instant::now() + timeout();
    let mut events = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(TermEvent::Exit) => return events,
            Ok(TermEvent::Redraw) => {}
            Ok(event) => events.push(event),
            Err(err) => panic!(
                "no Exit event: {err}
screen:
{}",
                session.screen_text()
            ),
        }
    }
}

#[test]
fn osc_events_come_through_the_pty() {
    // This also checks that ConPTY passes these sequences to us.
    let script = if cfg!(windows) {
        r#"$e=[char]27; $b=[char]7; Write-Host -NoNewline "$e]9;hello 9$b$e]777;notify;Title;body 777$b$e]777;fterm-agent;waiting;need you$b$e]7;file://pc/C:/work$b$e]133;D;3$b"; Start-Sleep -Milliseconds 300"#
    } else {
        r#"printf ']9;hello 9]777;notify;Title;body 777]777;fterm-agent;waiting;need you]7;file://pc/C:/work]133;D;3'; sleep 0.3"#
    };
    let options = if cfg!(windows) {
        SessionOptions::command("powershell.exe", ["-NoProfile", "-Command", script])
    } else {
        SessionOptions::command("sh", ["-c", script])
    };
    let (session, rx) = spawn(options);
    let events = events_until_exit(&session, &rx);
    let osc: Vec<OscEvent> = events
        .into_iter()
        .filter_map(|e| match e {
            TermEvent::Osc(osc) => Some(osc),
            _ => None,
        })
        .collect();
    assert!(
        osc.contains(&OscEvent::Notify {
            title: None,
            body: "hello 9".into()
        }),
        "{osc:?}"
    );
    assert!(
        osc.contains(&OscEvent::Notify {
            title: Some("Title".into()),
            body: "body 777".into()
        }),
        "{osc:?}"
    );
    assert!(
        osc.contains(&OscEvent::Agent {
            state: "waiting".into(),
            message: "need you".into()
        }),
        "{osc:?}"
    );
    assert!(osc.contains(&OscEvent::Cwd("C:/work".into())), "{osc:?}");
    assert!(
        osc.contains(&OscEvent::Prompt(PromptMark::CommandFinished(Some(3)))),
        "{osc:?}"
    );
}

#[cfg(windows)]
#[test]
fn powershell_integration_sends_cwd_and_exit_codes() {
    use fterm_term::shell::{install_scripts, powershell_args};
    let dir = std::env::temp_dir().join(format!("fterm-ps-test-{}", std::process::id()));
    let script = install_scripts(&dir).unwrap();
    let args = powershell_args(&["-NoLogo".to_owned(), "-NoProfile".to_owned()], &script);
    let options = SessionOptions::command("powershell.exe", args);
    let (session, rx) = spawn(options);
    session.write(b"cmd /c exit 3\r".to_vec());
    session.write(b"echo 'a;b\\c'\r".to_vec());
    session.write(b"exit\r".to_vec());
    let osc: Vec<OscEvent> = events_until_exit(&session, &rx)
        .into_iter()
        .filter_map(|e| match e {
            TermEvent::Osc(osc) => Some(osc),
            _ => None,
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(osc.iter().any(|e| matches!(e, OscEvent::Cwd(_))), "{osc:?}");
    assert!(
        osc.contains(&OscEvent::Prompt(PromptMark::CommandExecuted)),
        "{osc:?}"
    );
    assert!(
        osc.contains(&OscEvent::Prompt(PromptMark::CommandFinished(Some(3)))),
        "{osc:?}"
    );
    // The command text comes before the command runs (633;E, then 133;C).
    let line = osc
        .iter()
        .position(|e| *e == OscEvent::CommandLine("cmd /c exit 3".into()));
    let executed = osc
        .iter()
        .position(|e| *e == OscEvent::Prompt(PromptMark::CommandExecuted));
    assert!(line.is_some() && line < executed, "{osc:?}");
    assert!(
        osc.contains(&OscEvent::CommandLine("echo 'a;b\\c'".into())),
        "`;` and the backslash survive the escape: {osc:?}"
    );
}

#[test]
fn input_start_is_the_cell_after_the_prompt() {
    // Many lines first, so the history is not empty and the place has to count it.
    let script = if cfg!(windows) {
        r#"$e=[char]27; $b=[char]7; 1..30 | % { Write-Host "line $_" }; Write-Host -NoNewline "abc$e]133;B${b}def"; Start-Sleep -Milliseconds 300"#
    } else {
        r#"for i in $(seq 30); do echo line $i; done; printf 'abc]133;Bdef'; sleep 0.3"#
    };
    let options = if cfg!(windows) {
        SessionOptions::command("powershell.exe", ["-NoProfile", "-Command", script])
    } else {
        SessionOptions::command("sh", ["-c", script])
    };
    let (session, rx) = spawn(options);
    let events = events_until_exit(&session, &rx);
    let start = events
        .iter()
        .find_map(|e| match e {
            TermEvent::Osc(OscEvent::InputStart { line, column }) => Some((*line, *column)),
            _ => None,
        })
        .expect("an InputStart event");
    assert_eq!(start.1, 3, "after `abc`");
    let text = session.with_term(|term| {
        use fterm_term::alacritty_terminal::index::{Column, Line};
        let grid = term.grid();
        let row = &grid[Line(start.0 as i32 - grid.history_size() as i32)];
        (0..3)
            .map(|c| row[Column(start.1 + c)].c)
            .collect::<String>()
    });
    assert_eq!(text, "def");
}

#[test]
fn the_intro_text_stays_above_the_output() {
    // A restored pane shows its old text first; the program must not wipe it.
    let options = SessionOptions {
        intro: b"\x1b[90mold line 1\r\nold line 2\x1b[0m\r\n".to_vec(),
        ..echo_command()
    };
    let (session, rx) = spawn(options);
    wait_for_exit(&session, &rx);
    let text = session.with_term(|term| {
        let total = fterm_term::input::total_lines(term);
        fterm_term::input::lines_text(term, 0, total)
    });
    let (old, new) = (text.find("old line 2"), text.find("fterm-ok"));
    assert!(
        matches!((old, new), (Some(o), Some(n)) if o < n),
        "the text was:\n{text}"
    );
    assert!(text.contains("old line 1\nold line 2"), "{text}");
}

#[test]
fn bash_integration_sends_cwd_and_exit_codes() {
    use fterm_term::shell::{bash_args, install_scripts};
    let bash = if cfg!(windows) {
        r"C:\Program Files\Git\bin\bash.exe"
    } else {
        "bash"
    };
    if cfg!(windows) && !std::path::Path::new(bash).exists() {
        eprintln!("no Git Bash: skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("fterm-bash-test-{}", std::process::id()));
    install_scripts(&dir).unwrap();
    let args = bash_args(&["--login".to_owned(), "-i".to_owned()], &dir).unwrap();
    let mut options = SessionOptions::command(bash, args);
    // No user files: the test must not depend on this computer.
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    options
        .env
        .push(("HOME".into(), home.display().to_string()));
    let (session, rx) = spawn(options);
    session.write(b"false\r".to_vec());
    session.write(b"echo 'a;b'\r".to_vec());
    session.write(b"exit\r".to_vec());
    let osc: Vec<OscEvent> = events_until_exit(&session, &rx)
        .into_iter()
        .filter_map(|e| match e {
            TermEvent::Osc(osc) => Some(osc),
            _ => None,
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    let cwd = osc.iter().find_map(|e| match e {
        OscEvent::Cwd(dir) => Some(dir.clone()),
        _ => None,
    });
    assert!(cwd.is_some(), "{osc:?}");
    if cfg!(windows) {
        // Git Bash says `/c/Users/...`; fterm needs the Windows folder.
        let cwd = cwd.unwrap();
        let bytes = cwd.as_bytes();
        assert!(
            bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':',
            "{cwd}"
        );
    }
    assert!(
        osc.contains(&OscEvent::Prompt(PromptMark::CommandFinished(Some(1)))),
        "`false` ends with 1: {osc:?}"
    );
    assert!(
        osc.contains(&OscEvent::CommandLine("echo 'a;b'".into())),
        "{osc:?}"
    );
}

#[test]
fn a_scene_session_has_no_program() {
    let session = Session::scene(GridSize::new(10, 3));
    assert_eq!(session.pid(), None);
    assert_eq!(session.program(), "scene");
    // Keys go nowhere (there is no program), and nothing breaks.
    session.write(b"hello\r".to_vec());
    session.feed(b"\x1b[2;3Hab\x1b[31mc");
    assert_eq!(session.screen_text(), "\n  abc\n\n");
    session.resize(GridSize::new(20, 5), (8, 16));
    assert_eq!(session.grid_size(), GridSize::new(20, 5));
    let rows = session.with_term(|term| term.screen_lines());
    assert_eq!(rows, 5);
    drop(session);
}

#[test]
fn a_recording_has_the_output_and_the_sizes() {
    let file = std::env::temp_dir().join(format!("fterm-record-test-{}.cast", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let options = SessionOptions {
        record: Some(file.clone()),
        ..SessionOptions::default()
    };
    let (session, rx) = spawn(options);
    session.resize(GridSize::new(100, 30), (8, 16));
    session.write(b"echo fterm-recorded\r".to_vec());
    session.write(b"exit\r".to_vec());
    wait_for_exit(&session, &rx);
    drop(session);
    let text = std::fs::read_to_string(&file).unwrap();
    let _ = std::fs::remove_file(&file);
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["version"], 2);
    assert_eq!(lines[0]["width"], 80);
    assert!(
        lines.iter().any(|l| l[1] == "r" && l[2] == "100x30"),
        "the resize: {text}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l[1] == "o" && l[2].as_str().is_some_and(|t| t.contains("fterm-recorded"))),
        "the output: {text}"
    );
}

#[cfg(windows)]
#[test]
fn resizing_does_not_lose_the_scrollback() {
    // ConPTY with no PSEUDOCONSOLE_RESIZE_QUIRK draws its screen again on every resize, over the
    // terminal's own reflow: lines of the scrollback go away (and the prompt comes twice).
    let (session, rx) = spawn(SessionOptions::command(
        "powershell.exe",
        ["-NoLogo", "-NoProfile"],
    ));
    session.write(b"1..100 | ForEach-Object { \"line-$_ \" + ('x' * 60) }\r".to_vec());
    let deadline = std::time::Instant::now() + timeout();
    while !session.with_term(|term| {
        let total = fterm_term::input::total_lines(term);
        fterm_term::input::lines_text(term, 0, total).contains("line-100 ")
    }) {
        assert!(
            std::time::Instant::now() < deadline,
            "no output: {}",
            session.screen_text()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    for (cols, rows) in [
        (70, 25),
        (120, 30),
        (50, 20),
        (130, 35),
        (40, 20),
        (120, 30),
        (90, 30),
        (140, 35),
        (60, 22),
        (120, 30),
    ] {
        session.resize(GridSize::new(cols, rows), (8, 16));
        std::thread::sleep(Duration::from_millis(400));
    }
    std::thread::sleep(Duration::from_secs(1));
    let text = session.with_term(|term| {
        let total = fterm_term::input::total_lines(term);
        fterm_term::input::lines_text(term, 0, total)
    });
    session.write(b"exit\r".to_vec());
    wait_for_exit(&session, &rx);
    let kept = (1..=100)
        .filter(|n| text.contains(&format!("line-{n} ")))
        .count();
    assert!(kept >= 65, "only {kept} of 100 lines are left:\n{text}");
}
