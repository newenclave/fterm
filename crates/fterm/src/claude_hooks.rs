//! Puts the fterm hooks into the Claude Code settings (`settings.json`), next to the user's own hooks.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The settings file of Claude Code: in `CLAUDE_CONFIG_DIR`, else in `~/.claude`.
pub fn settings_path(claude_config_dir: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let base = match (claude_config_dir, home) {
        (Some(dir), _) => dir.to_path_buf(),
        (None, Some(home)) => home.join(".claude"),
        (None, None) => return None,
    };
    Some(base.join("settings.json"))
}

/// All the fterm hooks: the agent states, and with `ftermctl` the plan review of Claude Code plan mode
/// (stub).
pub fn fterm_hooks(states: &Value, ftermctl: Option<&Path>) -> Value {
    let mut all = states.clone();
    let Some(exe) = ftermctl else {
        return all;
    };
    // Forward slashes work in the bash of Claude Code (Git Bash on Windows) and in cmd.
    let path = exe.to_string_lossy().replace('\\', "/");
    all["hooks"]["PreToolUse"] = serde_json::json!([{
        "matcher": "ExitPlanMode",
        "hooks": [{
            "type": "command",
            "command": format!("\"{path}\" review --hook"),
            "timeout": 3600
        }]
    }]);
    // The rich hook in place of the `echo` states (one process for each event): the Agents panel gets the prompt,
    // the tools, the real question, and the last message. The matchers stay as they were.
    let event_hook = serde_json::json!({
        "type": "command",
        "command": format!("\"{path}\" agent-event --hook"),
        "timeout": 10
    });
    for event in ["UserPromptSubmit", "PostToolUse", "Notification", "Stop"] {
        let mut entry = states["hooks"][event][0].clone();
        if entry.is_null() {
            entry = serde_json::json!({});
        }
        entry["hooks"] = serde_json::json!([event_hook.clone()]);
        all["hooks"][event] = serde_json::json!([entry]);
    }
    // The conversation id of the pane (start, resume, /clear, compact), so a restore brings it back.
    all["hooks"]["SessionStart"] = serde_json::json!([{
        "hooks": [{
            "type": "command",
            "command": format!("\"{path}\" agent-session --hook"),
            "timeout": 10
        }]
    }]);
    all
}

/// What "Install Claude Code hooks" does with the status line: the new settings, and the user's old status line
/// to keep in the chain file (ours runs it).
#[derive(Debug, PartialEq)]
pub struct StatusPlan {
    pub settings: Value,
    pub saved: Option<Value>,
}

/// Puts the status line of fterm (`ftermctl statusline`) into `settings`: it gives fterm the cost and the context,
/// then prints the status line of the user (saved in `chain`). `None` when it is ours already.
pub fn wrap_statusline(settings: &Value, ftermctl: &Path, chain: &Path) -> Option<StatusPlan> {
    let ctl = ftermctl.to_string_lossy().replace('\\', "/");
    let old = settings.get("statusLine").filter(|v| !v.is_null());
    if old
        .and_then(|o| o.get("command"))
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains("ftermctl") && c.contains(" statusline"))
    {
        return None;
    }
    let mut line = serde_json::json!({ "type": "command" });
    match old {
        Some(old) => {
            let chain = chain.to_string_lossy().replace('\\', "/");
            line["command"] =
                serde_json::json!(format!("\"{ctl}\" statusline --chain \"{chain}\""));
            for key in ["padding", "refreshInterval"] {
                if let Some(v) = old.get(key) {
                    line[key] = v.clone();
                }
            }
        }
        None => line["command"] = serde_json::json!(format!("\"{ctl}\" statusline")),
    }
    let mut settings = settings.clone();
    settings["statusLine"] = line;
    Some(StatusPlan {
        settings,
        saved: old.cloned(),
    })
}

/// An entry of a hook event is from fterm: one of its commands sends the fterm agent sequence.
fn is_ours(entry: &Value) -> bool {
    entry["hooks"].as_array().into_iter().flatten().any(|hook| {
        hook["command"].as_str().is_some_and(|c| {
            c.contains("fterm-agent")
                || c.ends_with("review --hook")
                || c.ends_with("agent-session --hook")
                || c.ends_with("agent-event --hook")
        })
    })
}

/// `settings` with the fterm hooks of `ours`. Gives the new settings and the hook events that changed. In each
/// event the old fterm entries (see `is_ours`) give way to the new ones, and the entries of the user stay; so an
/// older install gets the new hooks, and a second time changes nothing.
pub fn merge_hooks(settings: &Value, ours: &Value) -> Result<(Value, Vec<String>), String> {
    let mut merged = settings.clone();
    let root = merged
        .as_object_mut()
        .ok_or("the settings file is not a JSON object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or("`hooks` in the settings is not an object")?;
    let mut added = Vec::new();
    for (event, entries) in ours["hooks"].as_object().into_iter().flatten() {
        let list = hooks
            .entry(event.clone())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| format!("`hooks.{event}` in the settings is not a list"))?;
        let mut new: Vec<Value> = list.iter().filter(|e| !is_ours(e)).cloned().collect();
        new.extend(entries.as_array().into_iter().flatten().cloned());
        if *list != new {
            *list = new;
            added.push(event.clone());
        }
    }
    Ok((merged, added))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ours() -> Value {
        serde_json::from_str(crate::agent::CLAUDE_HOOKS).unwrap()
    }

    fn events(v: &Value) -> Vec<String> {
        let mut out: Vec<String> = v["hooks"].as_object().unwrap().keys().cloned().collect();
        out.sort();
        out
    }

    #[test]
    fn the_settings_file() {
        let home = Path::new("home");
        assert_eq!(
            settings_path(None, Some(home)),
            Some(home.join(".claude").join("settings.json"))
        );
        let dir = Path::new("cfg");
        assert_eq!(
            settings_path(Some(dir), Some(home)),
            Some(dir.join("settings.json"))
        );
        assert_eq!(settings_path(None, None), None);
    }

    #[test]
    fn empty_settings_get_all_hooks() {
        let (merged, added) = merge_hooks(&json!({}), &ours()).unwrap();
        assert_eq!(events(&merged), events(&ours()));
        let mut sorted = added.clone();
        sorted.sort();
        assert_eq!(sorted, events(&ours()));
        assert_eq!(merged["hooks"]["Stop"], ours()["hooks"]["Stop"]);
    }

    #[test]
    fn the_users_settings_and_hooks_stay() {
        let mine = json!({ "hooks": [{ "type": "command", "command": "my-script stop" }] });
        let settings = json!({
            "model": "opus",
            "permissions": { "allow": ["Bash(ls)"] },
            "hooks": { "Stop": [mine], "PreToolUse": [{ "hooks": [] }] }
        });
        let (merged, added) = merge_hooks(&settings, &ours()).unwrap();
        assert_eq!(merged["model"], "opus");
        assert_eq!(merged["permissions"], settings["permissions"]);
        assert_eq!(
            merged["hooks"]["PreToolUse"],
            settings["hooks"]["PreToolUse"]
        );
        let stop = merged["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop[0], mine, "the user's hook is first and kept");
        assert_eq!(
            stop.len(),
            1 + ours()["hooks"]["Stop"].as_array().unwrap().len()
        );
        assert!(added.contains(&"Stop".to_owned()));
    }

    #[test]
    fn an_old_install_gets_the_new_hooks_and_keeps_the_user_ones() {
        // Installed by an older fterm: the `echo` state hook on PostToolUse, next to a hook of the user.
        let mine = json!({ "hooks": [{ "type": "command", "command": "my-linter" }] });
        let old_ours = ours()["hooks"]["PostToolUse"][0].clone();
        let settings = json!({ "hooks": { "PostToolUse": [mine.clone(), old_ours] } });
        let new_entry = json!({ "matcher": "", "hooks": [{ "type": "command",
            "command": "\"C:/f/ftermctl.exe\" agent-event --hook" }] });
        let newer = json!({ "hooks": { "PostToolUse": [new_entry.clone()] } });
        let (merged, changed) = merge_hooks(&settings, &newer).unwrap();
        assert_eq!(
            merged["hooks"]["PostToolUse"],
            json!([mine, new_entry]),
            "ours replaced, the user's stays"
        );
        assert_eq!(changed, ["PostToolUse"]);
        let (again, changed) = merge_hooks(&merged, &newer).unwrap();
        assert_eq!(again, merged);
        assert!(changed.is_empty(), "a second time changes nothing");
    }

    #[test]
    fn the_status_line_of_the_user_stays_behind_ours() {
        let ctl = PathBuf::from("C:/f/ftermctl.exe");
        let chain = PathBuf::from("C:/f/claude-statusline.json");
        // No status line yet: ours.
        let plan = wrap_statusline(&json!({}), &ctl, &chain).unwrap();
        assert_eq!(
            plan.settings["statusLine"],
            json!({ "type": "command", "command": "\"C:/f/ftermctl.exe\" statusline" })
        );
        assert_eq!(plan.saved, None);
        // The user's own: saved, and ours runs it (its padding and refresh stay).
        let mine = json!({ "type": "command", "command": "~/.claude/line.sh", "padding": 0, "refreshInterval": 5 });
        let plan = wrap_statusline(
            &json!({ "statusLine": mine.clone(), "model": "opus" }),
            &ctl,
            &chain,
        )
        .unwrap();
        assert_eq!(plan.saved, Some(mine));
        assert_eq!(
            plan.settings["statusLine"],
            json!({ "type": "command", "padding": 0, "refreshInterval": 5,
                    "command": "\"C:/f/ftermctl.exe\" statusline --chain \"C:/f/claude-statusline.json\"" })
        );
        assert_eq!(plan.settings["model"], "opus", "other settings stay");
        // Ours already: no change (and the saved one is not lost).
        assert!(wrap_statusline(&plan.settings, &ctl, &chain).is_none());
    }

    #[test]
    fn a_second_time_changes_nothing() {
        let (once, _) = merge_hooks(&json!({}), &ours()).unwrap();
        let (twice, added) = merge_hooks(&once, &ours()).unwrap();
        assert_eq!(twice, once);
        assert!(added.is_empty());
    }

    #[test]
    fn strange_settings_are_not_changed() {
        assert!(merge_hooks(&json!([1]), &ours()).is_err());
        assert!(merge_hooks(&json!({ "hooks": [] }), &ours()).is_err());
        let err = merge_hooks(&json!({ "hooks": { "Stop": {} } }), &ours()).unwrap_err();
        assert!(err.contains("hooks.Stop"), "{err}");
    }

    #[test]
    fn the_plan_review_hook() {
        let exe = PathBuf::from("C:/tools/fterm/ftermctl.exe");
        let all = fterm_hooks(&ours(), Some(&exe));
        let entry = &all["hooks"]["PreToolUse"][0];
        assert_eq!(entry["matcher"], "ExitPlanMode");
        let hook = &entry["hooks"][0];
        assert_eq!(hook["type"], "command");
        assert_eq!(hook["timeout"], 3600, "the user takes time to review");
        let command = hook["command"].as_str().unwrap();
        assert!(command.ends_with(" review --hook"), "{command}");
        assert!(
            command.contains("C:/tools/fterm/ftermctl.exe"),
            "slashes work in bash and cmd: {command}"
        );
        assert!(
            command.starts_with('"'),
            "a path with spaces works: {command}"
        );
        // With no ftermctl the `echo` states stay.
        assert_eq!(
            fterm_hooks(&ours(), None)["hooks"]["Stop"],
            ours()["hooks"]["Stop"]
        );
        // No ftermctl: only the states.
        assert_eq!(fterm_hooks(&ours(), None), ours());
        // The conversation of the pane: SessionStart tells fterm its id.
        let start = &all["hooks"]["SessionStart"][0]["hooks"][0];
        let command = start["command"].as_str().unwrap();
        assert!(
            command.starts_with('"') && command.ends_with(" agent-session --hook"),
            "{command}"
        );
        assert!(
            fterm_hooks(&ours(), None)["hooks"]
                .get("SessionStart")
                .is_none()
        );
        // The rich hook takes the place of the `echo` state hooks: one process for each event, not two.
        for event in ["UserPromptSubmit", "PostToolUse", "Notification", "Stop"] {
            let entries = all["hooks"][event].as_array().unwrap();
            assert_eq!(entries.len(), 1, "{event}");
            let command = entries[0]["hooks"][0]["command"].as_str().unwrap();
            assert!(
                command.ends_with(" agent-event --hook"),
                "{event}: {command}"
            );
        }
        assert_eq!(
            all["hooks"]["Notification"][0]["matcher"],
            ours()["hooks"]["Notification"][0]["matcher"],
            "the same questions as before"
        );
        // These stay `echo`: no data for the panel.
        assert_eq!(all["hooks"]["StopFailure"], ours()["hooks"]["StopFailure"]);
        // Installed once, the review hook is known as ours.
        let (once, added) = merge_hooks(&json!({}), &all).unwrap();
        assert!(added.contains(&"PreToolUse".to_owned()));
        assert!(added.contains(&"SessionStart".to_owned()));
        let (twice, added) = merge_hooks(&once, &all).unwrap();
        assert_eq!(twice, once);
        assert!(added.is_empty());
    }
}
