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
    text
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
}
