//! The command palette: a list of actions and a filter that the user types.

use fterm_config::keys::Action;

/// How many lines the palette shows at once.
pub const VISIBLE_ROWS: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    pub label: String,
    /// The key for this action, like "Ctrl+Shift+T" (empty when there is none).
    pub key: String,
    pub action: Action,
}

/// How well `query` matches `text`: `None` = no match, a bigger number = a better match.
/// The chars of the query must come in order. Word starts and chars next to each other score more.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let query: Vec<char> = query.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut next = 0;
    let mut last_match: Option<usize> = None;
    for (i, &c) in text.iter().enumerate() {
        if next == query.len() {
            break;
        }
        if c != query[next] {
            continue;
        }
        score += 1;
        let word_start = i == 0 || matches!(text[i - 1], ' ' | ':' | '-' | '_' | '/' | '.');
        match last_match {
            // A run of matching chars is the best sign.
            Some(last) if last + 1 == i => score += 7,
            Some(last) => score -= ((i - last - 1) as i32).min(3),
            // The first match: earlier is better.
            None => score -= (i as i32).min(10),
        }
        if word_start {
            score += 6;
        }
        last_match = Some(i);
        next += 1;
    }
    (next == query.len()).then_some(score)
}

pub struct PaletteState {
    items: Vec<PaletteItem>,
    query: String,
    /// Items that match the query, best first.
    shown: Vec<usize>,
    selected: usize,
    /// The first shown line (when the list is longer than the box).
    top: usize,
}

impl PaletteState {
    pub fn new(items: Vec<PaletteItem>) -> Self {
        let mut state = Self {
            items,
            query: String::new(),
            shown: Vec::new(),
            selected: 0,
            top: 0,
        };
        state.refilter();
        state
    }

    /// Finds the matching items again, best first. The selection goes to the best one.
    fn refilter(&mut self) {
        let mut scored: Vec<(i32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| score(&self.query, &item.label).map(|s| (s, i)))
            .collect();
        // A stable sort keeps the list order for the same score.
        scored.sort_by_key(|(s, _)| -s);
        self.shown = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
        self.top = 0;
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn type_text(&mut self, text: &str) {
        self.query.extend(text.chars().filter(|c| !c.is_control()));
        self.refilter();
    }

    pub fn backspace(&mut self) {
        if self.query.pop().is_some() {
            self.refilter();
        }
    }

    /// Moves the selection (`-1` = up, `+1` = down, `±VISIBLE_ROWS` = a page). It stops at the ends.
    pub fn move_selection(&mut self, step: i32) {
        if self.shown.is_empty() {
            return;
        }
        let last = self.shown.len() - 1;
        self.selected = (self.selected as i64 + i64::from(step)).clamp(0, last as i64) as usize;
        // Keep the selected line inside the box.
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + VISIBLE_ROWS {
            self.top = self.selected + 1 - VISIBLE_ROWS;
        }
    }

    /// The action of the selected line.
    pub fn selected_action(&self) -> Option<&Action> {
        let index = *self.shown.get(self.selected)?;
        Some(&self.items[index].action)
    }

    /// The lines to draw now: (item, is it selected).
    pub fn visible(&self) -> Vec<(&PaletteItem, bool)> {
        self.shown
            .iter()
            .enumerate()
            .skip(self.top)
            .take(VISIBLE_ROWS)
            .map(|(row, &index)| (&self.items[index], row == self.selected))
            .collect()
    }

    /// How many items match the query.
    #[cfg(test)]
    pub fn match_count(&self) -> usize {
        self.shown.len()
    }
}

#[cfg(test)]
mod tests {
    use fterm_config::keys::BuiltinAction;

    use super::*;

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(score("", "New tab"), Some(0));
    }

    #[test]
    fn chars_must_come_in_order() {
        assert!(score("nt", "New tab").is_some());
        assert!(score("tn", "New tab").is_none());
        assert!(score("xyz", "New tab").is_none());
        // Case does not matter.
        assert!(score("NEW", "new tab").is_some());
    }

    #[test]
    fn word_starts_and_runs_score_more() {
        // "nt": n and t start words in "New tab", but not in "Paint".
        assert!(score("nt", "New tab") > score("nt", "Paint"));
        // A run of chars beats the same chars far apart.
        assert!(score("split", "Split right") > score("split", "Sp lit"));
        // An earlier match wins when the rest is the same.
        assert!(score("cl", "Close pane") > score("cl", "New tab: Claude"));
    }

    fn item(label: &str) -> PaletteItem {
        PaletteItem {
            label: label.to_owned(),
            key: String::new(),
            action: Action::Builtin(BuiltinAction::NewTab),
        }
    }

    fn labels(state: &PaletteState) -> Vec<String> {
        state
            .visible()
            .iter()
            .map(|(item, _)| item.label.clone())
            .collect()
    }

    #[test]
    fn typing_filters_and_sorts() {
        let mut state = PaletteState::new(vec![
            item("Close pane"),
            item("New tab: Claude"),
            item("Split right: Claude"),
            item("Zoom pane"),
        ]);
        assert_eq!(state.match_count(), 4);
        state.type_text("cl");
        assert_eq!(state.query(), "cl");
        assert_eq!(state.match_count(), 3, "Zoom pane has no c-l");
        state.type_text("aude");
        assert_eq!(state.match_count(), 2);
        assert_eq!(labels(&state), ["New tab: Claude", "Split right: Claude"]);
        state.backspace();
        state.backspace();
        state.backspace();
        state.backspace();
        state.backspace();
        state.backspace();
        state.type_text("split cl");
        assert_eq!(labels(&state)[0], "Split right: Claude");
        while !state.query().is_empty() {
            state.backspace();
        }
        assert_eq!(state.match_count(), 4);
    }

    #[test]
    fn selection_stays_in_range() {
        let mut state = PaletteState::new((0..30).map(|i| item(&format!("item {i}"))).collect());
        assert_eq!(state.visible()[0], (&state.items[0], true));
        state.move_selection(-1);
        assert!(state.visible()[0].1, "stops at the top");
        state.move_selection(100);
        let visible = state.visible();
        assert_eq!(visible.len(), VISIBLE_ROWS);
        assert_eq!(visible.last().unwrap().0.label, "item 29");
        assert!(
            visible.last().unwrap().1,
            "the last item is selected and seen"
        );
    }

    #[test]
    fn filter_resets_the_selection_to_the_best_match() {
        let mut state = PaletteState::new(vec![item("aaa"), item("bbb"), item("abc")]);
        state.move_selection(2);
        state.type_text("b");
        let visible = state.visible();
        assert!(visible[0].1);
        assert_eq!(state.match_count(), 2);
    }

    #[test]
    fn selected_action_is_from_the_selected_line() {
        let mut items = vec![item("first"), item("second")];
        items[1].action = Action::Builtin(BuiltinAction::Zoom);
        let mut state = PaletteState::new(items);
        state.move_selection(1);
        assert_eq!(
            state.selected_action(),
            Some(&Action::Builtin(BuiltinAction::Zoom))
        );
        state.type_text("zzz");
        assert_eq!(state.selected_action(), None, "nothing matches");
    }
}
