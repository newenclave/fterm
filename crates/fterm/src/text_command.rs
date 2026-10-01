//! Text to command (Phase 9): the task in the prompt becomes a command for the shell.

/// The system text for "make one command".
pub fn system_prompt(os: &str, shell: &str, cwd: Option<&str>) -> String {
    let mut text = format!("Turn the user's task into one command for the {shell} shell on {os}.");
    if let Some(cwd) = cwd {
        text.push_str(&format!(" The current folder is {cwd}."));
    }
    text.push_str(
        " Answer with only the command: no words, no explanation, no code block. \
         Use the normal tools of that shell and OS. If the task needs many steps, \
         join them in one line the way that shell does it.",
    );
    match shell_kind(shell) {
        ShellKind::PowerShell => text.push_str(
            " Do not use Linux tools (ls, grep, find, du, head, awk, sed): use PowerShell cmdlets \
             such as Get-ChildItem, Select-String, Sort-Object, Select-Object, Measure-Object.",
        ),
        ShellKind::Cmd => text.push_str(
            " Do not use Linux tools (ls, grep, find, du): use cmd commands such as dir, findstr, where, type.",
        ),
        ShellKind::Posix => {}
    }
    text
}

/// Example tasks and commands for this shell (small models follow examples better than rules).
/// Pairs of (task, command).
pub fn examples(shell: &str) -> Vec<(&'static str, &'static str)> {
    match shell_kind(shell) {
        ShellKind::PowerShell => vec![
            (
                "show the 5 newest files",
                "Get-ChildItem -File | Sort-Object LastWriteTime -Descending | Select-Object -First 5",
            ),
            (
                "find TODO in all .rs files",
                "Get-ChildItem -Recurse -Filter *.rs | Select-String TODO",
            ),
            (
                "how much space does this folder use",
                "(Get-ChildItem -Recurse -File | Measure-Object Length -Sum).Sum / 1MB",
            ),
        ],
        ShellKind::Cmd => vec![
            ("show the 5 newest files", "dir /o-d /a-d"),
            ("find TODO in all .rs files", "findstr /s /n TODO *.rs"),
        ],
        ShellKind::Posix => vec![
            ("show the 5 newest files", "ls -t | head -5"),
            (
                "find TODO in all .rs files",
                "grep -rn TODO --include='*.rs' .",
            ),
            ("how much space does this folder use", "du -sh ."),
        ],
    }
}

enum ShellKind {
    PowerShell,
    Cmd,
    Posix,
}

fn shell_kind(shell: &str) -> ShellKind {
    let name = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    match name {
        "powershell" | "pwsh" => ShellKind::PowerShell,
        "cmd" => ShellKind::Cmd,
        _ => ShellKind::Posix,
    }
}

/// The command from an answer: without code fences, backticks, a `$ ` or `PS> ` prompt, and words around it.
pub fn clean_command(answer: &str) -> String {
    // A code block: its lines are the command.
    let blocks = crate::ai_chat::blocks(answer);
    let code = blocks.iter().find_map(|b| match b {
        crate::ai_chat::Block::Code { code, .. } => Some(code.clone()),
        _ => None,
    });
    let text = code.unwrap_or_else(|| answer.trim().to_owned());
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            let line = line.trim();
            let line = line
                .strip_prefix("$ ")
                .or_else(|| line.strip_prefix("PS> "))
                .or_else(|| line.strip_prefix("> "))
                .unwrap_or(line);
            line.trim_matches('`').trim().to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect();
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_prompt_asks_for_only_a_command() {
        let text = system_prompt("Windows", "powershell", Some("C:/work"));
        assert!(
            text.contains("powershell") && text.contains("Windows") && text.contains("C:/work")
        );
        assert!(text.contains("only the command"));
        assert!(!system_prompt("Linux", "bash", None).contains("folder"));
    }

    #[test]
    fn clean_answers() {
        assert_eq!(clean_command("ls -la"), "ls -la");
        assert_eq!(clean_command("```bash\nls -la\n```"), "ls -la");
        assert_eq!(
            clean_command("```\ndu -sh * | sort -h\n```\n"),
            "du -sh * | sort -h"
        );
        assert_eq!(clean_command("`git status`"), "git status");
        assert_eq!(clean_command("$ git log -5"), "git log -5");
        assert_eq!(clean_command("PS> Get-Date"), "Get-Date");
        // Words before the block: only the code counts.
        assert_eq!(
            clean_command(
                "Here is the command:\n```powershell\nGet-ChildItem | sort Length\n```\nIt sorts."
            ),
            "Get-ChildItem | sort Length"
        );
        // Many lines in a block stay many lines.
        assert_eq!(clean_command("```bash\ncd /tmp\nls\n```"), "cd /tmp\nls");
        assert_eq!(clean_command("  \n"), "");
    }

    #[test]
    fn powershell_gets_powershell_examples_and_no_linux_tools() {
        let text = system_prompt("Windows", "powershell", None);
        assert!(text.contains("Do not use Linux tools"), "{text}");
        let ex = examples("powershell");
        assert!(ex.len() >= 2);
        assert!(
            ex.iter()
                .all(|(_, cmd)| !cmd.starts_with("ls ") && !cmd.contains("grep"))
        );
        assert!(ex.iter().any(|(_, cmd)| cmd.contains("Get-ChildItem")));
        assert!(
            examples("pwsh.exe")
                .iter()
                .any(|(_, c)| c.contains("Get-ChildItem")),
            "pwsh too"
        );
    }

    #[test]
    fn other_shells_get_their_own_examples() {
        assert!(examples("cmd").iter().any(|(_, c)| c.starts_with("dir ")));
        assert!(
            examples("bash")
                .iter()
                .any(|(_, c)| c.contains("find ") || c.contains("du "))
        );
        assert!(examples("zsh").iter().any(|(_, c)| c.contains("du ")));
        assert!(!system_prompt("Linux", "bash", None).contains("Do not use Linux tools"));
    }
}
