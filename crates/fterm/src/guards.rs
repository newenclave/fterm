//! The guards of the API in the config: `api.ask` (questions before a program uses another pane) and
//! `ai.api_access` (programs may ask the AI panel). The config is code that you trust, but a program that can write
//! `fterm.lua` must not turn a guard off with no word: a looser value than the one you last confirmed waits for your
//! yes, and the old value stays until then.

use std::path::Path;

use fterm_config::tr;
use serde::{Deserialize, Serialize};

use crate::access::{Button, QuestionLines};

/// The values of the guards.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guards {
    /// `api.ask`.
    pub ask: bool,
    /// `ai.api_access`.
    pub ai_access: bool,
}

/// The defaults of the config: the guards are on.
impl Default for Guards {
    fn default() -> Self {
        Self {
            ask: true,
            ai_access: false,
        }
    }
}

/// What fterm keeps of the guards (`guards.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuardState {
    /// The values that you confirmed (with nothing confirmed: the defaults).
    pub confirmed: Guards,
    /// Looser values of the config that you said "keep the guard" to: no warning again until the config changes.
    pub kept: Option<Guards>,
}

/// What a config means for the guards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// The values that count now.
    pub effective: Guards,
    /// Ask about these looser values of the config.
    pub warn: Option<Guards>,
    /// The new state (to save when it changed).
    pub state: GuardState,
}

/// A config (at start or at a reload): a tighter value counts at once; a looser one keeps the confirmed value and
/// asks, unless you said "keep" to that same value before.
pub fn decide(state: &GuardState, config: Guards) -> Decision {
    // A tighter value is confirmed at once: a later loosening asks again.
    let confirmed = Guards {
        ask: state.confirmed.ask || config.ask,
        ai_access: state.confirmed.ai_access && config.ai_access,
    };
    let loosened = confirmed != config;
    // "Keep" counts only while the config stays at that value.
    let kept = if loosened { state.kept } else { None };
    Decision {
        effective: confirmed,
        warn: (loosened && kept != Some(config)).then_some(config),
        state: GuardState { confirmed, kept },
    }
}

/// Your answer to the warning about the looser `config`: turn the guard off, or keep it.
pub fn answer(state: &GuardState, config: Guards, turn_off: bool) -> GuardState {
    if turn_off {
        GuardState {
            confirmed: config,
            kept: None,
        }
    } else {
        GuardState {
            confirmed: state.confirmed,
            kept: Some(config),
        }
    }
}

/// Reads the state; no file or a bad one: nothing confirmed (the guards are on).
pub fn load(path: &Path) -> GuardState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, state: &GuardState) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let text = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// The lines and buttons of the warning about the looser `config`. "Turn the guard off" is `Button::Allow`, "Keep
/// the guard" is `Button::No`; only a click turns a guard off.
pub fn warning_box(
    confirmed: Guards,
    config: Guards,
    answering: bool,
) -> (QuestionLines, Vec<(Button, String)>) {
    use fterm_render::overlay::QuestionLook as L;
    let mut lines = vec![(tr!("box.guard_title"), L::Text)];
    if confirmed.ask && !config.ask {
        lines.push((tr!("box.guard_ask"), L::Fact));
    }
    if !confirmed.ai_access && config.ai_access {
        lines.push((tr!("box.guard_ai"), L::Fact));
    }
    lines.push((tr!("box.guard_who"), L::Text));
    let keys = if answering {
        tr!("box.guard_keys")
    } else {
        tr!("box.access_start")
    };
    lines.push((keys, L::Dim));
    let buttons = vec![
        (Button::Allow, tr!("box.guard_off")),
        (Button::No, tr!("box.guard_keep")),
    ];
    (lines, buttons)
}

/// The text in the corner of the tab bar: the mark while the questions are off, and the unread events.
pub fn corner_text(unread: Option<usize>, questions_off: bool) -> Option<String> {
    let unread = unread.map(|n| n.to_string());
    if !questions_off {
        return unread;
    }
    let off = tr!("title.no_questions");
    Some(match unread {
        Some(n) => format!("{off} · {n}"),
        None => off,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOSE_ASK: Guards = Guards {
        ask: false,
        ai_access: false,
    };
    const LOOSE_AI: Guards = Guards {
        ask: true,
        ai_access: true,
    };

    #[test]
    fn with_nothing_confirmed_the_guards_are_on() {
        let state = GuardState::default();
        assert_eq!(state.confirmed, Guards::default());
        let d = decide(&state, Guards::default());
        assert_eq!(d.effective, Guards::default());
        assert_eq!(d.warn, None);
    }

    #[test]
    fn a_looser_config_keeps_the_old_value_and_warns() {
        let d = decide(&GuardState::default(), LOOSE_ASK);
        assert!(d.effective.ask, "the questions stay on until you answer");
        assert_eq!(d.warn, Some(LOOSE_ASK));
        let d = decide(&GuardState::default(), LOOSE_AI);
        assert!(!d.effective.ai_access);
        assert_eq!(d.warn, Some(LOOSE_AI));
    }

    #[test]
    fn a_tighter_config_counts_at_once() {
        let state = GuardState {
            confirmed: Guards {
                ask: false,
                ai_access: true,
            },
            kept: None,
        };
        let d = decide(&state, Guards::default());
        assert_eq!(d.effective, Guards::default());
        assert_eq!(d.warn, None);
        assert_eq!(
            d.state.confirmed,
            Guards::default(),
            "a later loosening asks again"
        );
        // One guard tighter, the other looser: the tighter one counts, the looser one asks.
        let state = GuardState {
            confirmed: Guards {
                ask: true,
                ai_access: true,
            },
            kept: None,
        };
        let d = decide(&state, LOOSE_ASK);
        assert_eq!(
            d.effective,
            Guards {
                ask: true,
                ai_access: false
            }
        );
        assert_eq!(d.warn, Some(LOOSE_ASK));
    }

    #[test]
    fn turn_off_confirms_the_new_value() {
        let d = decide(&GuardState::default(), LOOSE_ASK);
        let state = answer(&d.state, LOOSE_ASK, true);
        let d = decide(&state, LOOSE_ASK);
        assert_eq!(d.effective, LOOSE_ASK);
        assert_eq!(d.warn, None);
    }

    #[test]
    fn keep_asks_again_only_when_the_value_changes() {
        let d = decide(&GuardState::default(), LOOSE_ASK);
        let state = answer(&d.state, LOOSE_ASK, false);
        let d = decide(&state, LOOSE_ASK);
        assert!(d.effective.ask, "kept");
        assert_eq!(
            d.warn, None,
            "the same value: no warning at each reload or start"
        );
        // The config goes back, then loose again: a new question.
        let d = decide(&d.state, Guards::default());
        let d = decide(&d.state, LOOSE_ASK);
        assert_eq!(d.warn, Some(LOOSE_ASK));
        // "Keep" for one loose value is not "keep" for another.
        let state = answer(&d.state, LOOSE_ASK, false);
        let both = Guards {
            ask: false,
            ai_access: true,
        };
        assert_eq!(decide(&state, both).warn, Some(both));
    }

    #[test]
    fn the_state_file_round_trips_and_a_bad_one_keeps_the_guards_on() {
        let dir = std::env::temp_dir().join(format!("fterm-guards-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("guards.json");
        assert_eq!(load(&path), GuardState::default(), "no file");
        let state = GuardState {
            confirmed: LOOSE_ASK,
            kept: Some(LOOSE_AI),
        };
        save(&path, &state).unwrap();
        assert_eq!(load(&path), state);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), GuardState::default(), "a bad file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_warning_says_which_guard_and_only_a_click_turns_it_off() {
        use fterm_render::overlay::QuestionLook as L;
        let (lines, buttons) = warning_box(Guards::default(), LOOSE_ASK, false);
        let text: Vec<&str> = lines.iter().map(|(t, _)| t.as_str()).collect();
        assert!(text.iter().any(|t| t.contains("api.ask")), "{text:?}");
        assert!(
            !text.iter().any(|t| t.contains("ai.api_access")),
            "{text:?}"
        );
        assert!(
            lines
                .iter()
                .any(|(t, l)| t.contains("api.ask") && *l == L::Fact)
        );
        assert!(
            lines.last().unwrap().0.contains("Ctrl+Shift+Enter"),
            "{text:?}"
        );
        let which: Vec<Button> = buttons.iter().map(|(b, _)| *b).collect();
        assert_eq!(which, [Button::Allow, Button::No]);
        let (lines, _) = warning_box(
            Guards::default(),
            Guards {
                ask: false,
                ai_access: true,
            },
            true,
        );
        let text: Vec<&str> = lines.iter().map(|(t, _)| t.as_str()).collect();
        assert!(text.iter().any(|t| t.contains("ai.api_access")), "{text:?}");
        assert!(
            lines.last().unwrap().0.contains("Esc"),
            "answer mode: {text:?}"
        );
    }

    #[test]
    fn the_corner_shows_that_the_questions_are_off() {
        assert_eq!(corner_text(None, false), None);
        assert_eq!(corner_text(Some(3), false).as_deref(), Some("3"));
        let off = corner_text(None, true).unwrap();
        assert!(off.contains("no API questions"), "{off}");
        let both = corner_text(Some(3), true).unwrap();
        assert!(
            both.contains("no API questions") && both.ends_with('3'),
            "{both}"
        );
    }
}
