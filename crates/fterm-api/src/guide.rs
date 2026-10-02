//! The guide for agents: the MCP instructions, `ftermctl guide`, and the Claude Code skill.

/// The guide (assets/agents/GUIDE.md).
pub const GUIDE: &str = include_str!("../../../assets/agents/GUIDE.md");

/// The short form, for the instructions of the MCP server.
pub const INSTRUCTIONS: &str = include_str!("../../../assets/agents/INSTRUCTIONS.md");

/// This line is in every skill file that fterm writes, so fterm updates only its own file.
pub const MARKER: &str = "<!-- fterm-skill: written by fterm; it updates this file -->";

/// When Claude Code loads the skill (it reads this line, not the whole file).
const DESCRIPTION: &str = "Use when you run inside the fterm terminal (FTERM_PANE_ID is set) and want to \
run tests or commands in another pane and read their output, wait for a command or another agent, \
send messages to the agent in another pane, notify the user, or draw a chart or a picture in a \
Braille scene pane.";

/// The Claude Code skill (`~/.claude/skills/fterm/SKILL.md`): the guide with the skill header.
pub fn skill_md() -> String {
    format!("---\nname: fterm\ndescription: {DESCRIPTION}\n---\n\n{MARKER}\n\n{GUIDE}")
}

/// What to do with the skill file that is there now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillPlan {
    /// Write it (there is no file, or an older one of fterm).
    Write,
    /// It is the same already.
    UpToDate,
    /// The user's own file: do not change it.
    Foreign,
}

/// What to do, from the text of the file that is there (`None` = no file).
pub fn skill_plan(existing: Option<&str>) -> SkillPlan {
    plan_for(existing, &skill_md())
}

/// `skill_plan` with our text as a parameter.
fn plan_for(existing: Option<&str>, ours: &str) -> SkillPlan {
    match existing {
        None => SkillPlan::Write,
        Some(text) if text.replace("\r\n", "\n") == ours => SkillPlan::UpToDate,
        Some(text) if text.contains(MARKER) => SkillPlan::Write,
        Some(_) => SkillPlan::Foreign,
    }
}

/// The skill file: `$CLAUDE_CONFIG_DIR/skills/fterm/SKILL.md`, or `~/.claude/skills/fterm/SKILL.md`.
pub fn skill_path(
    claude_config_dir: Option<&std::path::Path>,
    home: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let base = match (claude_config_dir, home) {
        (Some(dir), _) => dir.to_path_buf(),
        (None, Some(home)) => home.join(".claude"),
        (None, None) => return None,
    };
    Some(base.join("skills").join("fterm").join("SKILL.md"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_skill_has_a_header_that_says_when_to_use_it() {
        let skill = skill_md();
        let mut lines = skill.lines();
        assert_eq!(lines.next(), Some("---"));
        assert_eq!(lines.next(), Some("name: fterm"));
        let description = lines.next().unwrap();
        assert!(description.starts_with("description: "), "{description}");
        for word in ["fterm", "FTERM_PANE_ID", "pane", "chart"] {
            assert!(description.contains(word), "{word} in {description}");
        }
        assert_eq!(lines.next(), Some("---"));
        assert!(skill.contains(MARKER));
        assert!(skill.contains(GUIDE.trim()), "the whole guide");
    }

    #[test]
    fn only_the_own_file_of_fterm_is_changed() {
        assert_eq!(skill_plan(None), SkillPlan::Write);
        assert_eq!(skill_plan(Some(&skill_md())), SkillPlan::UpToDate);
        let old = format!("---\nname: fterm\n---\n{MARKER}\nan old guide\n");
        assert_eq!(skill_plan(Some(&old)), SkillPlan::Write);
        assert_eq!(
            skill_plan(Some("---\nname: fterm\n---\nmy own notes\n")),
            SkillPlan::Foreign
        );
        // Line ends can change on the way (an editor, git): still the same file.
        assert_eq!(
            skill_plan(Some(&skill_md().replace('\n', "\r\n"))),
            SkillPlan::UpToDate
        );
    }

    #[test]
    fn line_ends_do_not_matter_on_either_side() {
        // On a Windows checkout (core.autocrlf) the embedded guide has CRLF, the file on disk LF.
        let ours_lf = "---\nname: fterm\n---\nguide\n";
        let ours_crlf = ours_lf.replace('\n', "\r\n");
        assert_eq!(plan_for(Some(ours_lf), &ours_crlf), SkillPlan::UpToDate);
        assert_eq!(plan_for(Some(&ours_crlf), ours_lf), SkillPlan::UpToDate);
        assert_eq!(
            plan_for(Some("---\nname: fterm\n---\nother\n"), &ours_crlf),
            SkillPlan::Foreign
        );
    }

    #[test]
    fn the_skill_goes_to_the_claude_folder() {
        let home = Path::new("/home/me");
        assert_eq!(
            skill_path(None, Some(home)),
            Some(
                home.join(".claude")
                    .join("skills")
                    .join("fterm")
                    .join("SKILL.md")
            )
        );
        let custom = Path::new("/etc/claude");
        assert_eq!(
            skill_path(Some(custom), Some(home)),
            Some(custom.join("skills").join("fterm").join("SKILL.md"))
        );
        assert_eq!(skill_path(None, None), None);
    }
}
