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
    all
}

/// An entry of a hook event is from fterm: one of its commands sends the fterm agent sequence.
fn is_ours(entry: &Value) -> bool {
    entry["hooks"].as_array().into_iter().flatten().any(|hook| {
        hook["command"]
            .as_str()
            .is_some_and(|c| c.contains("fterm-agent") || c.ends_with("review --hook"))
    })
}

/// `settings` with the fterm hooks of `ours` added. Gives the new settings and the hook events that got
/// an fterm hook. An event that has an fterm hook already is not changed, so it can run again.
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
        if list.iter().any(is_ours) {
            continue;
        }
        list.extend(entries.as_array().into_iter().flatten().cloned());
        added.push(event.clone());
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
        // The agent states are there too.
        assert_eq!(all["hooks"]["Stop"], ours()["hooks"]["Stop"]);
        // No ftermctl: only the states.
        assert_eq!(fterm_hooks(&ours(), None), ours());
        // Installed once, the review hook is known as ours.
        let (once, added) = merge_hooks(&json!({}), &all).unwrap();
        assert!(added.contains(&"PreToolUse".to_owned()));
        let (twice, added) = merge_hooks(&once, &all).unwrap();
        assert_eq!(twice, once);
        assert!(added.is_empty());
    }
}
