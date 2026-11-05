//! The keys of copy mode: "G" → `bottom`, "ctrl+u" → `half_page_up`, "i w" → `select_word`.
//! Unlike the window keys (`keys.rs`), a char here is the char that the key types, with its case:
//! "V" is Shift+v, and "$" is the dollar key of any layout.

use std::collections::HashMap;

use crate::keys::Key;

/// One key of copy mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CopyKey {
    pub ctrl: bool,
    pub alt: bool,
    /// Only for named keys (Shift+PageUp); for a char the case says it.
    pub shift: bool,
    /// `Key::Char` holds the typed char (case-sensitive). With Ctrl it is lowercase.
    pub key: Key,
}

impl CopyKey {
    pub fn char(c: char) -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
            key: Key::Char(c),
        }
    }

    pub fn named(key: Key) -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
            key,
        }
    }

    pub fn with_ctrl(mut self) -> Self {
        self.ctrl = true;
        if let Key::Char(c) = self.key {
            self.key = Key::Char(c.to_ascii_lowercase());
        }
        self
    }

    /// Reads one key: `G`, `$`, `+`, `ctrl+u`, `alt+w`, `shift+pageup`, `esc`, `space`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut chars = text.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_whitespace() || c.is_control() {
                return Err(format!("`{text}`: write `space` or a key name"));
            }
            return Ok(Self::char(c));
        }
        let parts: Vec<&str> = text.split('+').collect();
        let (name, mod_names) = parts.split_last().expect("split gives at least one part");
        let mut key = Self::char(' ');
        for m in mod_names {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => key.ctrl = true,
                "alt" | "option" => key.alt = true,
                "shift" => key.shift = true,
                "" => return Err(format!("`{text}`: a `+` without a key (write `plus`)")),
                other => return Err(format!("`{text}`: unknown modifier `{other}`")),
            }
        }
        let mut one = name.chars();
        key.key = match (one.next(), one.next()) {
            (Some(c), None) if !c.is_whitespace() => Key::Char(c),
            _ => named_key(name).ok_or_else(|| format!("`{text}`: unknown key `{name}`"))?,
        };
        if let Key::Char(c) = key.key {
            if key.shift {
                return Err(format!(
                    "`{text}`: write the char itself (`{}`), not shift",
                    c.to_ascii_uppercase()
                ));
            }
            if key.ctrl {
                key.key = Key::Char(c.to_ascii_lowercase());
            }
        }
        Ok(key)
    }

    /// For people: "G", "Ctrl+U", "Esc".
    pub fn display(&self) -> String {
        let mut out = String::new();
        for (on, name) in [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
        ] {
            if on {
                out.push_str(name);
            }
        }
        match self.key {
            Key::Char(c) if self.ctrl => out.push(c.to_ascii_uppercase()),
            Key::Char(c) => out.push(c),
            Key::Escape => out.push_str("Esc"),
            other => out.push_str(&format!("{other:?}")),
        }
        out
    }
}

fn named_key(name: &str) -> Option<Key> {
    let key = match name.to_ascii_lowercase().as_str() {
        "tab" => Key::Tab,
        "enter" | "return" => Key::Enter,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" | "ins" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "plus" => Key::Char('+'),
        "minus" => Key::Char('-'),
        other => {
            let n: u8 = other.strip_prefix('f')?.parse().ok()?;
            return (1..=24).contains(&n).then_some(Key::F(n));
        }
    };
    Some(key)
}

/// Reads the keys of a binding: one key, or two with a space ("i w").
pub fn parse_keys(text: &str) -> Result<Vec<CopyKey>, String> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    match parts.len() {
        0 => Err("empty key".to_owned()),
        1 | 2 => parts.iter().map(|p| CopyKey::parse(p)).collect(),
        _ => Err(format!("`{text}`: at most two keys (like `i w`)")),
    }
}

/// How a jump goes on the line: `f` `F` `t` `T`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JumpKind {
    /// `f`: to the char.
    Forward,
    /// `F`: back to the char.
    Backward,
    /// `t`: to the cell before the char.
    TillForward,
    /// `T`: back to the cell after the char.
    TillBackward,
}

impl JumpKind {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "f" => Self::Forward,
            "F" => Self::Backward,
            "t" => Self::TillForward,
            "T" => Self::TillBackward,
            _ => return None,
        })
    }
}

/// What a key of copy mode does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CopyCommand {
    Up,
    Down,
    Left,
    Right,
    WordRight,
    WordLeft,
    WordEnd,
    BigWordRight,
    BigWordLeft,
    BigWordEnd,
    LineStart,
    LineEnd,
    FirstChar,
    ScreenTop,
    ScreenMiddle,
    ScreenBottom,
    ParagraphUp,
    ParagraphDown,
    Bracket,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    Top,
    Bottom,
    Select,
    SelectLines,
    SelectBlock,
    SelectWord,
    SelectWordAround,
    SelectBigWord,
    SelectBigWordAround,
    SwapEnds,
    /// Copy; leave copy mode or stay, as `copy_mode.exit_after_copy` says.
    Copy,
    CopyAndStay,
    CopyAndExit,
    /// Copy the cursor line (or the lines of the selection).
    CopyLine,
    Exit,
    SearchForward,
    SearchBackward,
    NextMatch,
    PrevMatch,
    /// Wait for a char, then jump to it.
    Jump(JumpKind),
    /// Jump to this char (from Lua: `{ jump = "f", char = "x" }`).
    JumpTo(JumpKind, char),
    RepeatJump,
    RepeatJumpBack,
}

/// The commands with a plain name (all but `JumpTo`), in the order of the docs.
const NAMES: &[(&str, CopyCommand)] = {
    use CopyCommand::*;
    &[
        ("up", Up),
        ("down", Down),
        ("left", Left),
        ("right", Right),
        ("word_right", WordRight),
        ("word_left", WordLeft),
        ("word_end", WordEnd),
        ("big_word_right", BigWordRight),
        ("big_word_left", BigWordLeft),
        ("big_word_end", BigWordEnd),
        ("line_start", LineStart),
        ("line_end", LineEnd),
        ("first_char", FirstChar),
        ("screen_top", ScreenTop),
        ("screen_middle", ScreenMiddle),
        ("screen_bottom", ScreenBottom),
        ("paragraph_up", ParagraphUp),
        ("paragraph_down", ParagraphDown),
        ("bracket", Bracket),
        ("page_up", PageUp),
        ("page_down", PageDown),
        ("half_page_up", HalfPageUp),
        ("half_page_down", HalfPageDown),
        ("top", Top),
        ("bottom", Bottom),
        ("select", Select),
        ("select_lines", SelectLines),
        ("select_block", SelectBlock),
        ("select_word", SelectWord),
        ("select_word_around", SelectWordAround),
        ("select_big_word", SelectBigWord),
        ("select_big_word_around", SelectBigWordAround),
        ("swap_ends", SwapEnds),
        ("copy", Copy),
        ("copy_and_stay", CopyAndStay),
        ("copy_and_exit", CopyAndExit),
        ("copy_line", CopyLine),
        ("exit", Exit),
        ("search_forward", SearchForward),
        ("search_backward", SearchBackward),
        ("next_match", NextMatch),
        ("prev_match", PrevMatch),
        ("jump_forward", Jump(JumpKind::Forward)),
        ("jump_backward", Jump(JumpKind::Backward)),
        ("till_forward", Jump(JumpKind::TillForward)),
        ("till_backward", Jump(JumpKind::TillBackward)),
        ("repeat_jump", RepeatJump),
        ("repeat_jump_back", RepeatJumpBack),
    ]
};

impl CopyCommand {
    pub fn from_name(name: &str) -> Option<Self> {
        NAMES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
    }

    pub fn name(&self) -> Option<&'static str> {
        NAMES.iter().find(|(_, c)| c == self).map(|(n, _)| *n)
    }

    /// All the names, for the error text and the docs.
    pub fn names() -> impl Iterator<Item = &'static str> {
        NAMES.iter().map(|(n, _)| *n)
    }
}

/// What a key runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyBinding {
    Command(CopyCommand),
    /// A Lua function of the config (its number), called with `(fterm, cm)`.
    Lua(usize),
}

/// The answer for the keys typed so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    Found(CopyBinding),
    /// The first key of a longer binding: wait for the next key.
    Prefix,
    None,
}

/// The default keys of copy mode (the same as in `docs/KEYS.md`).
pub const DEFAULT_COPY_KEYS: &[(&str, &str)] = &[
    ("up", "up"),
    ("down", "down"),
    ("left", "left"),
    ("right", "right"),
    ("k", "up"),
    ("j", "down"),
    ("h", "left"),
    ("l", "right"),
    ("w", "word_right"),
    ("b", "word_left"),
    ("e", "word_end"),
    ("W", "big_word_right"),
    ("B", "big_word_left"),
    ("E", "big_word_end"),
    ("0", "line_start"),
    ("home", "line_start"),
    ("$", "line_end"),
    ("end", "line_end"),
    ("^", "first_char"),
    ("H", "screen_top"),
    ("M", "screen_middle"),
    ("L", "screen_bottom"),
    ("{", "paragraph_up"),
    ("}", "paragraph_down"),
    ("%", "bracket"),
    ("pageup", "page_up"),
    ("pagedown", "page_down"),
    ("ctrl+b", "page_up"),
    ("ctrl+f", "page_down"),
    ("ctrl+u", "half_page_up"),
    ("ctrl+d", "half_page_down"),
    ("g", "top"),
    ("G", "bottom"),
    ("v", "select"),
    ("V", "select_lines"),
    ("ctrl+v", "select_block"),
    ("i w", "select_word"),
    ("a w", "select_word_around"),
    ("i W", "select_big_word"),
    ("a W", "select_big_word_around"),
    ("o", "swap_ends"),
    ("y", "copy"),
    ("enter", "copy"),
    ("Y", "copy_line"),
    ("esc", "exit"),
    ("q", "exit"),
    ("/", "search_forward"),
    ("?", "search_backward"),
    ("n", "next_match"),
    ("N", "prev_match"),
    ("f", "jump_forward"),
    ("F", "jump_backward"),
    ("t", "till_forward"),
    ("T", "till_backward"),
    (";", "repeat_jump"),
    (",", "repeat_jump_back"),
];

/// The keys of copy mode: the defaults, then the user's bindings.
#[derive(Clone, Debug)]
pub struct CopyKeymap {
    map: HashMap<Vec<CopyKey>, CopyBinding>,
}

impl Default for CopyKeymap {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl CopyKeymap {
    pub fn with_defaults() -> Self {
        let mut keys = Self {
            map: HashMap::new(),
        };
        for (text, name) in DEFAULT_COPY_KEYS {
            let k = parse_keys(text).expect("default copy keys are valid");
            let command = CopyCommand::from_name(name).expect("default copy actions are valid");
            keys.bind(k, Some(CopyBinding::Command(command)));
        }
        keys
    }

    /// Adds or changes a binding; `None` removes it. A key can not be a binding and the start of a longer
    /// one: the newer binding wins ("g g" removes "g", and "g" removes "g g").
    pub fn bind(&mut self, keys: Vec<CopyKey>, binding: Option<CopyBinding>) {
        let Some(binding) = binding else {
            self.map.remove(&keys);
            return;
        };
        let first = keys[0];
        if keys.len() > 1 {
            self.map.remove(&vec![first]);
        } else {
            self.map.retain(|k, _| k.len() == 1 || k[0] != first);
        }
        self.map.insert(keys, binding);
    }

    pub fn lookup(&self, keys: &[CopyKey]) -> Lookup {
        if let Some(binding) = self.map.get(keys) {
            return Lookup::Found(*binding);
        }
        if keys.len() == 1 && self.map.keys().any(|k| k.len() > 1 && k[0] == keys[0]) {
            return Lookup::Prefix;
        }
        Lookup::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> CopyKey {
        CopyKey::parse(text).unwrap()
    }

    fn found(map: &CopyKeymap, text: &str) -> Option<CopyCommand> {
        match map.lookup(&parse_keys(text).unwrap()) {
            Lookup::Found(CopyBinding::Command(c)) => Some(c),
            _ => None,
        }
    }

    #[test]
    fn a_char_keeps_its_case_and_symbols_work() {
        assert_eq!(one("G"), CopyKey::char('G'));
        assert_ne!(one("G"), one("g"));
        assert_eq!(one("$"), CopyKey::char('$'));
        assert_eq!(one("+"), CopyKey::char('+'));
        assert_eq!(one("plus"), CopyKey::char('+'));
        assert_eq!(one("л"), CopyKey::char('л'));
    }

    #[test]
    fn modifiers_and_named_keys() {
        assert_eq!(one("ctrl+u"), CopyKey::char('u').with_ctrl());
        assert_eq!(
            one("ctrl+U"),
            one("ctrl+u"),
            "with Ctrl the case does not count"
        );
        assert_eq!(one("Ctrl+V").display(), "Ctrl+V");
        assert_eq!(one("esc"), CopyKey::named(Key::Escape));
        assert_eq!(one("PageUp"), CopyKey::named(Key::PageUp));
        let shift_up = one("shift+pageup");
        assert!(shift_up.shift && shift_up.key == Key::PageUp);
        assert_eq!(one("alt+w").display(), "Alt+w");
        assert_eq!(one("f3"), CopyKey::named(Key::F(3)));
    }

    #[test]
    fn bad_keys_say_why() {
        assert!(CopyKey::parse("").is_err());
        assert!(CopyKey::parse(" ").unwrap_err().contains("space"));
        assert!(CopyKey::parse("shift+v").unwrap_err().contains("`V`"));
        assert!(CopyKey::parse("hyper+a").unwrap_err().contains("hyper"));
        assert!(CopyKey::parse("nokey").unwrap_err().contains("nokey"));
        assert!(CopyKey::parse("ctrl+").is_err());
        assert!(parse_keys("a b c").unwrap_err().contains("two"));
        assert!(parse_keys("  ").is_err());
    }

    #[test]
    fn two_keys_with_a_space() {
        assert_eq!(
            parse_keys("i w").unwrap(),
            vec![CopyKey::char('i'), CopyKey::char('w')]
        );
        assert_eq!(parse_keys(" G ").unwrap(), vec![CopyKey::char('G')]);
    }

    #[test]
    fn names_go_both_ways() {
        for name in CopyCommand::names() {
            let c = CopyCommand::from_name(name).unwrap();
            assert_eq!(c.name(), Some(name));
        }
        assert_eq!(CopyCommand::from_name("nope"), None);
        assert_eq!(
            CopyCommand::JumpTo(JumpKind::Forward, 'x').name(),
            None,
            "only from a table"
        );
        assert_eq!(JumpKind::from_name("T"), Some(JumpKind::TillBackward));
        assert_eq!(JumpKind::from_name("x"), None);
    }

    #[test]
    fn the_defaults_keep_the_old_keys() {
        use CopyCommand::*;
        let map = CopyKeymap::with_defaults();
        for (key, want) in [
            ("k", Up),
            ("up", Up),
            ("j", Down),
            ("h", Left),
            ("l", Right),
            ("w", WordRight),
            ("b", WordLeft),
            ("e", WordEnd),
            ("W", BigWordRight),
            ("0", LineStart),
            ("home", LineStart),
            ("$", LineEnd),
            ("^", FirstChar),
            ("H", ScreenTop),
            ("M", ScreenMiddle),
            ("L", ScreenBottom),
            ("{", ParagraphUp),
            ("}", ParagraphDown),
            ("%", Bracket),
            ("pageup", PageUp),
            ("ctrl+b", PageUp),
            ("ctrl+f", PageDown),
            ("ctrl+u", HalfPageUp),
            ("ctrl+d", HalfPageDown),
            ("g", Top),
            ("G", Bottom),
            ("v", Select),
            ("V", SelectLines),
            ("ctrl+v", SelectBlock),
            ("y", Copy),
            ("enter", Copy),
            ("esc", Exit),
            ("q", Exit),
        ] {
            assert_eq!(found(&map, key), Some(want), "{key}");
        }
        assert_eq!(found(&map, "x"), None);
    }

    #[test]
    fn the_new_default_keys() {
        use CopyCommand::*;
        let map = CopyKeymap::with_defaults();
        for (key, want) in [
            ("/", SearchForward),
            ("?", SearchBackward),
            ("n", NextMatch),
            ("N", PrevMatch),
            ("f", Jump(JumpKind::Forward)),
            ("T", Jump(JumpKind::TillBackward)),
            (";", RepeatJump),
            (",", RepeatJumpBack),
            ("o", SwapEnds),
            ("i w", SelectWord),
            ("a W", SelectBigWordAround),
            ("Y", CopyLine),
        ] {
            assert_eq!(found(&map, key), Some(want), "{key}");
        }
        assert_eq!(map.lookup(&[CopyKey::char('i')]), Lookup::Prefix);
        assert_eq!(map.lookup(&parse_keys("i x").unwrap()), Lookup::None);
    }

    #[test]
    fn the_user_changes_and_removes_keys() {
        let mut map = CopyKeymap::with_defaults();
        map.bind(vec![CopyKey::char('x')], Some(CopyBinding::Lua(3)));
        assert_eq!(
            map.lookup(&[CopyKey::char('x')]),
            Lookup::Found(CopyBinding::Lua(3))
        );
        map.bind(vec![CopyKey::char('q')], None);
        assert_eq!(map.lookup(&[CopyKey::char('q')]), Lookup::None);
    }

    #[test]
    fn a_longer_binding_and_its_first_key_do_not_live_together() {
        let mut map = CopyKeymap::with_defaults();
        map.bind(
            parse_keys("g g").unwrap(),
            Some(CopyBinding::Command(CopyCommand::Top)),
        );
        assert_eq!(map.lookup(&[CopyKey::char('g')]), Lookup::Prefix);
        assert_eq!(found(&map, "g g"), Some(CopyCommand::Top));
        map.bind(
            vec![CopyKey::char('i')],
            Some(CopyBinding::Command(CopyCommand::Exit)),
        );
        assert_eq!(found(&map, "i"), Some(CopyCommand::Exit));
        assert_eq!(found(&map, "i w"), None, "the single key won");
    }
}
