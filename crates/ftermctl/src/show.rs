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

/// `ai_read` as text: `> ` before the lines of a question, the answers as they are.
pub fn chat(answer: &Value) -> String {
    let turns = answer["turns"].as_array().cloned().unwrap_or_default();
    let mut out = String::new();
    if turns.is_empty() {
        out.push_str("the chat is empty\n");
    }
    for turn in turns {
        let text = turn["text"].as_str().unwrap_or("");
        if turn["role"] == "user" {
            for line in text.lines() {
                out.push_str(&format!("> {line}\n"));
            }
        } else {
            out.push_str(text);
            if !text.is_empty() && !text.ends_with('\n') {
                out.push('\n');
            }
            if turn["streaming"] == true {
                out.push_str("(the answer is coming)\n");
            }
            if let Some(error) = turn["error"].as_str() {
                out.push_str(&format!("(failed: {error})\n"));
            }
            // A blank line after each question and its answer.
            out.push('\n');
        }
    }
    if let Some(input) = answer["input"].as_str().filter(|i| !i.is_empty()) {
        out.push_str(&format!("input: {input}\n"));
    }
    out
}

/// The themes, one per line; `*` marks the theme in use.
pub fn themes(answer: &Value) -> String {
    let current = answer["current"].as_str().unwrap_or_default();
    let mut out = String::new();
    for name in answer["themes"].as_array().into_iter().flatten() {
        let name = name.as_str().unwrap_or_default();
        let mark = if name == current { '*' } else { ' ' };
        out.push_str(&format!("{mark} {name}\n"));
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

    #[test]
    fn the_chat_as_text() {
        let answer = json!({
            "turns": [
                { "role": "user", "text": "what is ls?\n(from claude)", "streaming": false },
                { "role": "ai", "text": "It lists files.", "streaming": false },
                { "role": "user", "text": "and dir?", "streaming": false },
                { "role": "ai", "text": "", "streaming": false, "error": "stopped" }
            ],
            "input": "draft", "context": [], "running": false
        });
        let expected = [
            "> what is ls?",
            "> (from claude)",
            "It lists files.",
            "",
            "> and dir?",
            "(failed: stopped)",
            "",
            "input: draft",
            "",
        ]
        .join("\n");
        assert_eq!(chat(&answer), expected);
        assert_eq!(chat(&json!({ "turns": [] })), "the chat is empty\n");
    }
}
