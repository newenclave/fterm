//! Which pane an API client runs in: the nearest parent process that is the shell of a pane of this window. The
//! client's own word (`FTERM_PANE_ID`) proves nothing; the process tree does (see `fterm_term::process::chain_in`).

use fterm_term::process::Link;

/// The shell of a pane: its process id and start time. Both must match a process of a chain: a process id alone can
/// belong to another process later. `pid` is `None` for a pane with no shell of this machine (a scene, a dialog, a
/// pane of a remote mux server, or a mux pane whose server did not say it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneIdentity {
    pub pid: Option<u32>,
    pub start: u64,
}

/// The program of a pane that a client belongs to: the nearest process between the client and the pane's shell that
/// is not a shell (for example `claude.exe`), else the process just below the pane's shell (a script, or the client).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub pid: u32,
    pub start: u64,
    pub name: String,
}

/// The pane that a client runs in, as the process tree shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    pub pane: u64,
    pub program: Program,
}

/// The pane whose shell is the nearest process of `chain` (the client first, then its parents), or `None`.
pub fn verify(chain: &[Link], panes: &[(u64, PaneIdentity)]) -> Option<Checked> {
    chain.iter().enumerate().find_map(|(i, link)| {
        let (pane, _) = panes
            .iter()
            .find(|(_, id)| id.pid == Some(link.pid) && id.start != 0 && id.start == link.start)?;
        // The program: the nearest process above the client and below the shell that is not a shell too (a bash
        // that the user started in the pane runs many programs). With none: the process just below the shell (a
        // shell that runs a script calls the client again and again), or the client when the shell started it.
        let below = chain[1..i.max(1)]
            .iter()
            .rev()
            .find(|l| !fterm_term::process::is_shell(&l.name))
            .unwrap_or(&chain[i.saturating_sub(1)]);
        Some(Checked {
            pane: *pane,
            program: Program {
                pid: below.pid,
                start: below.start,
                name: below.name.clone(),
            },
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(pid: u32, start: u64, name: &str) -> Link {
        Link {
            pid,
            start,
            name: name.to_owned(),
        }
    }

    fn shell(pid: u32, start: u64) -> PaneIdentity {
        PaneIdentity {
            pid: Some(pid),
            start,
        }
    }

    fn chain() -> Vec<Link> {
        vec![
            link(10, 500, "ftermctl.exe"),
            link(7, 400, "claude.exe"),
            link(3, 300, "pwsh.exe"),
            link(1, 100, "explorer.exe"),
        ]
    }

    #[test]
    fn the_program_is_not_a_shell_that_the_user_started() {
        // pwsh (the pane) -> bash (started by the user) -> claude -> ftermctl: "Always" is for claude, not for
        // everything that runs in that bash.
        let chain = vec![
            link(10, 500, "ftermctl"),
            link(8, 450, "claude"),
            link(7, 400, "bash"),
            link(3, 300, "pwsh.exe"),
        ];
        let checked = verify(&chain, &[(5, shell(3, 300))]).expect("pane 5");
        assert_eq!(checked.program.name, "claude");
        assert_eq!(checked.program.pid, 8);
        // A script (a shell that runs it) calls ftermctl again and again: "Always" is for the script, not for one
        // ftermctl that ends at once.
        let chain = vec![
            link(10, 500, "ftermctl.exe"),
            link(7, 400, "pwsh.exe"),
            link(3, 300, "pwsh.exe"),
        ];
        let checked = verify(&chain, &[(5, shell(3, 300))]).expect("pane 5");
        assert_eq!(checked.program.pid, 7);
    }

    #[test]
    fn the_shell_of_a_pane_in_the_chain_is_its_pane() {
        let checked = verify(&chain(), &[(4, shell(9, 900)), (5, shell(3, 300))]).expect("pane 5");
        assert_eq!(checked.pane, 5);
        assert_eq!(
            checked.program,
            Program {
                pid: 7,
                start: 400,
                name: "claude.exe".into()
            },
            "the program is the child of the shell"
        );
    }

    #[test]
    fn the_nearest_pane_wins() {
        // A shell started from the shell of another pane (nested): the inner one is the pane.
        let checked = verify(&chain(), &[(5, shell(3, 300)), (6, shell(7, 400))]).expect("a pane");
        assert_eq!(checked.pane, 6);
        assert_eq!(checked.program.pid, 10, "its program is the client itself");
    }

    #[test]
    fn the_start_time_must_match_too() {
        assert_eq!(
            verify(&chain(), &[(5, shell(3, 301))]),
            None,
            "pid 3 is another process now"
        );
        assert_eq!(
            verify(&chain(), &[(5, shell(3, 0))]),
            None,
            "an unknown start time proves nothing"
        );
    }

    #[test]
    fn a_pane_with_no_shell_is_never_matched() {
        let none = PaneIdentity::default();
        assert_eq!(verify(&chain(), &[(5, none)]), None);
        assert_eq!(
            verify(&[], &[(5, shell(3, 300))]),
            None,
            "no chain: no pane"
        );
    }

    #[test]
    fn the_client_can_be_the_shell_itself() {
        // A pane whose program is ftermctl (a step of the dashboard).
        let checked = verify(&chain(), &[(8, shell(10, 500))]).expect("pane 8");
        assert_eq!((checked.pane, checked.program.pid), (8, 10));
    }
}
