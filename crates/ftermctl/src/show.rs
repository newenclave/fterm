//! Text for people (without `--json`).

use serde_json::Value;

/// `list` as a small table.
pub fn list(answer: &Value) -> String {
    let mut out = String::new();
    for tab in answer["tabs"].as_array().into_iter().flatten() {
        let mut line = format!(
            "tab {}  {}",
            tab["tab"],
            tab["title"].as_str().unwrap_or("")
        );
        if tab["active"] == true {
            line.push_str("  (active)");
        }
        out.push_str(&line);
        out.push('\n');
        for pane in tab["panes"].as_array().into_iter().flatten() {
            let star = if pane["active"] == true { "*" } else { " " };
            let mut parts = vec![
                format!("  pane {} {star}", pane["id"]),
                pane["program"].as_str().unwrap_or("").to_owned(),
            ];
            if let Some(cwd) = pane["cwd"].as_str() {
                parts.push(cwd.to_owned());
            }
            if pane["running"] == true {
                parts.push("running".to_owned());
            } else if pane["at_prompt"] == true {
                parts.push("prompt".to_owned());
            }
            if let Some(state) = pane["agent"]["state"].as_str() {
                parts.push(format!("agent: {state}"));
            }
            if let Some(n) = pane["messages"].as_u64().filter(|n| *n > 0) {
                parts.push(format!("messages: {n}"));
            }
            out.push_str(&parts.join("  "));
            out.push('\n');
        }
    }
    out
}

/// `read_messages` as lines.
pub fn messages(answer: &Value) -> String {
    let list = answer["messages"].as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        return "no messages\n".to_owned();
    }
    let mut out = String::new();
    for m in list {
        let from = m["from_name"].as_str().unwrap_or("?");
        match m["from"].as_u64() {
            Some(pane) => out.push_str(&format!("#{} from {from} (pane {pane}):\n", m["id"])),
            None => out.push_str(&format!("#{} from {from}:\n", m["id"])),
        }
        for line in m["text"].as_str().unwrap_or("").lines() {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_list() {
        let answer = json!({
            "tabs": [
                { "tab": 1, "title": "powershell", "active": true, "panes": [
                    { "id": 1, "program": "powershell", "cwd": "C:/work", "active": true,
                      "running": false, "at_prompt": true, "agent": null, "messages": 0 },
                    { "id": 3, "program": "claude", "cwd": "C:/x", "active": false,
                      "running": true, "at_prompt": false,
                      "agent": { "state": "waiting", "message": "Allow?" }, "messages": 2 }
                ]},
                { "tab": 2, "title": "logs", "active": false, "panes": [
                    { "id": 4, "program": "cmd", "cwd": null, "active": false,
                      "running": false, "at_prompt": false, "agent": null, "messages": 0 }
                ]}
            ]
        });
        let expected = [
            "tab 1  powershell  (active)",
            "  pane 1 *  powershell  C:/work  prompt",
            "  pane 3    claude  C:/x  running  agent: waiting  messages: 2",
            "tab 2  logs",
            "  pane 4    cmd",
            "",
        ]
        .join("\n");
        assert_eq!(list(&answer), expected);
    }

    #[test]
    fn messages_as_lines() {
        let answer = json!({ "pane": 2, "messages": [
            { "id": 1, "from": 3, "from_name": "claude", "text": "please review" },
            { "id": 2, "from": null, "from_name": "ftermctl", "text": "line 1\nline 2" }
        ]});
        let expected = [
            "#1 from claude (pane 3):",
            "  please review",
            "#2 from ftermctl:",
            "  line 1",
            "  line 2",
            "",
        ]
        .join("\n");
        assert_eq!(messages(&answer), expected);
        assert_eq!(messages(&json!({ "messages": [] })), "no messages\n");
    }
}
