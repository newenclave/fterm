//! `ftermctl review`: the answer of a review for a script, and for a Claude Code plan mode hook.

use serde_json::{Value, json};

/// The exit code for a review result: 0 approved, 1 changes, 2 cancelled (or no answer).
pub fn exit_code(result: &Value) -> u8 {
    match result["decision"].as_str() {
        Some("approved") => 0,
        Some("changes") => 1,
        _ => 2,
    }
}

/// The plan of a Claude Code `PreToolUse` hook for `ExitPlanMode`.
pub fn hook_plan(input: &Value) -> Option<String> {
    input["tool_input"]["plan"]
        .as_str()
        .filter(|p| !p.trim().is_empty())
        .map(str::to_owned)
}

/// The title of a plan: its first heading, else "Plan".
pub fn plan_title(plan: &str) -> String {
    plan.lines()
        .find_map(|l| l.trim().strip_prefix('#'))
        .map(|h| h.trim_start_matches('#').trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "Plan".to_owned())
}

/// The answer of the hook to Claude Code: approved = allow (with the same input), changes = deny with
/// the feedback (Claude revises the plan), cancelled = no answer (the normal dialog) (stub).
pub fn hook_answer(input: &Value, result: &Value) -> Option<Value> {
    let (decision, reason) = match result["decision"].as_str()? {
        "approved" => ("allow", "The user approved the plan in the fterm review."),
        "changes" => ("deny", result["feedback"].as_str().unwrap_or_default()),
        _ => return None,
    };
    let mut out = json!({
        "hookEventName": "PreToolUse",
        "permissionDecision": decision,
        "permissionDecisionReason": reason,
    });
    if decision == "allow" {
        // ExitPlanMode needs its input with an "allow".
        out["updatedInput"] = input["tool_input"].clone();
    }
    Some(json!({ "hookSpecificOutput": out }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(decision: &str) -> Value {
        json!({ "decision": decision, "feedback": format!("{decision} text"), "items": [] })
    }

    fn input() -> Value {
        json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "ExitPlanMode",
            "tool_input": { "plan": "# Themes\n- one", "planFilePath": "/tmp/p.md" }
        })
    }

    #[test]
    fn exit_codes() {
        assert_eq!(exit_code(&result("approved")), 0);
        assert_eq!(exit_code(&result("changes")), 1);
        assert_eq!(exit_code(&result("cancelled")), 2);
        assert_eq!(exit_code(&json!({})), 2);
    }

    #[test]
    fn the_plan_of_the_hook() {
        assert_eq!(hook_plan(&input()).as_deref(), Some("# Themes\n- one"));
        assert_eq!(hook_plan(&json!({ "tool_input": {} })), None);
        assert_eq!(hook_plan(&json!({ "tool_input": { "plan": "  " } })), None);
        assert_eq!(
            plan_title("intro\n## Themes for fterm\n- a"),
            "Themes for fterm"
        );
        assert_eq!(plan_title("- a\n- b"), "Plan");
    }

    #[test]
    fn the_answer_of_the_hook() {
        let ok = hook_answer(&input(), &result("approved")).unwrap();
        let out = &ok["hookSpecificOutput"];
        assert_eq!(out["hookEventName"], "PreToolUse");
        assert_eq!(out["permissionDecision"], "allow");
        assert_eq!(
            out["updatedInput"],
            input()["tool_input"],
            "the plan as it was"
        );

        let no = hook_answer(&input(), &result("changes")).unwrap();
        let out = &no["hookSpecificOutput"];
        assert_eq!(out["permissionDecision"], "deny");
        assert_eq!(out["permissionDecisionReason"], "changes text");

        assert_eq!(hook_answer(&input(), &result("cancelled")), None);
    }
}
