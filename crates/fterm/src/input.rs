//! Keyboard: turns a winit key press into bytes for the pty (xterm style).

use fterm_mux::Edge;
use fterm_term::alacritty_terminal::selection::SelectionType;
use fterm_term::alacritty_terminal::vi_mode::ViMotion;
use fterm_term::copy_mode::CopyAction;
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

/// One key press, with the parts of `winit::event::KeyEvent` that we need.
pub struct KeyInput<'a> {
    pub logical: &'a Key,
    pub physical: PhysicalKey,
    pub text: Option<&'a str>,
    pub mods: ModifiersState,
}

/// Returns the bytes to send, or `None` when the key sends nothing.
/// `app_cursor` is the DECCKM mode: arrows then send `ESC O A` and not `ESC [ A`.
pub fn encode_key(key: &KeyInput, app_cursor: bool) -> Option<Vec<u8>> {
    let mods = key.mods;
    let alt = mods.alt_key();
    let ctrl = mods.control_key();
    // xterm modifier number: 1 + shift(1) + alt(2) + ctrl(4). 1 means "no modifiers".
    let m = 1 + u8::from(mods.shift_key()) + 2 * u8::from(alt) + 4 * u8::from(ctrl);

    let with_alt = |bytes: &[u8]| {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        if alt {
            out.push(0x1b);
        }
        out.extend_from_slice(bytes);
        out
    };
    // Arrows, Home, End, F1-F4: `ESC [ X` (or `ESC O X`) and `ESC [ 1 ; m X` with modifiers.
    let letter = |c: u8, ss3: bool| {
        if m > 1 {
            format!("\x1b[1;{m}{}", c as char).into_bytes()
        } else if ss3 {
            vec![0x1b, b'O', c]
        } else {
            vec![0x1b, b'[', c]
        }
    };
    // Insert, Delete, PageUp, PageDown, F5-F12: `ESC [ n ~` and `ESC [ n ; m ~`.
    let tilde = |n: u8| {
        if m > 1 {
            format!("\x1b[{n};{m}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };

    if let Key::Named(named) = key.logical {
        let bytes = match named {
            // Shift+Enter = ESC CR: Claude Code and other TUI apps read it as "new line".
            NamedKey::Enter if mods.shift_key() => b"\x1b\r".to_vec(),
            NamedKey::Enter => with_alt(b"\r"),
            NamedKey::Backspace if ctrl => with_alt(b"\x08"),
            NamedKey::Backspace => with_alt(b"\x7f"),
            NamedKey::Tab if mods.shift_key() => b"\x1b[Z".to_vec(),
            NamedKey::Tab => with_alt(b"\t"),
            NamedKey::Escape => with_alt(b"\x1b"),
            NamedKey::Space if ctrl => with_alt(b"\x00"),
            NamedKey::Space => with_alt(b" "),
            NamedKey::ArrowUp => letter(b'A', app_cursor),
            NamedKey::ArrowDown => letter(b'B', app_cursor),
            NamedKey::ArrowRight => letter(b'C', app_cursor),
            NamedKey::ArrowLeft => letter(b'D', app_cursor),
            NamedKey::Home => letter(b'H', app_cursor),
            NamedKey::End => letter(b'F', app_cursor),
            NamedKey::F1 => letter(b'P', true),
            NamedKey::F2 => letter(b'Q', true),
            NamedKey::F3 => letter(b'R', true),
            NamedKey::F4 => letter(b'S', true),
            NamedKey::Insert => tilde(2),
            NamedKey::Delete => tilde(3),
            NamedKey::PageUp => tilde(5),
            NamedKey::PageDown => tilde(6),
            NamedKey::F5 => tilde(15),
            NamedKey::F6 => tilde(17),
            NamedKey::F7 => tilde(18),
            NamedKey::F8 => tilde(19),
            NamedKey::F9 => tilde(20),
            NamedKey::F10 => tilde(21),
            NamedKey::F11 => tilde(23),
            NamedKey::F12 => tilde(24),
            _ => return None,
        };
        return Some(bytes);
    }

    if ctrl && let Some(code) = ctrl_code(key) {
        return Some(with_alt(&[code]));
    }

    let text = key.text.or(match key.logical {
        Key::Character(s) => Some(s.as_str()),
        _ => None,
    })?;
    (!text.is_empty()).then(|| with_alt(text.as_bytes()))
}

/// Ctrl+key control code: Ctrl+A..Z = 0x01..0x1A, Ctrl+[ \ ] ^ _ = 0x1B..0x1F, Ctrl+@ = 0.
/// On non-Latin layouts (for example Russian) we use the physical key, so Ctrl+С is still ^C.
fn ctrl_code(key: &KeyInput) -> Option<u8> {
    let from_char = |c: char| match c.to_ascii_lowercase() {
        c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' | '2' | ' ' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        _ => None,
    };
    if let Key::Character(s) = key.logical
        && let Some(code) = s.chars().next().and_then(from_char)
    {
        return Some(code);
    }
    physical_letter(key.physical).and_then(from_char)
}

/// The Latin letter (or bracket) on a key, the same on every layout.
fn physical_letter(physical: PhysicalKey) -> Option<char> {
    let PhysicalKey::Code(code) = physical else {
        return None;
    };
    let letter = match code {
        KeyCode::KeyA => 'a',
        KeyCode::KeyB => 'b',
        KeyCode::KeyC => 'c',
        KeyCode::KeyD => 'd',
        KeyCode::KeyE => 'e',
        KeyCode::KeyF => 'f',
        KeyCode::KeyG => 'g',
        KeyCode::KeyH => 'h',
        KeyCode::KeyI => 'i',
        KeyCode::KeyJ => 'j',
        KeyCode::KeyK => 'k',
        KeyCode::KeyL => 'l',
        KeyCode::KeyM => 'm',
        KeyCode::KeyN => 'n',
        KeyCode::KeyO => 'o',
        KeyCode::KeyP => 'p',
        KeyCode::KeyQ => 'q',
        KeyCode::KeyR => 'r',
        KeyCode::KeyS => 's',
        KeyCode::KeyT => 't',
        KeyCode::KeyU => 'u',
        KeyCode::KeyV => 'v',
        KeyCode::KeyW => 'w',
        KeyCode::KeyX => 'x',
        KeyCode::KeyY => 'y',
        KeyCode::KeyZ => 'z',
        KeyCode::BracketLeft => '[',
        KeyCode::BracketRight => ']',
        KeyCode::Backslash => '\\',
        _ => return None,
    };
    Some(letter)
}

/// A key in copy mode. Keys like in vi and less. `None` = the key does nothing in copy mode.
pub fn copy_mode_action(key: &KeyInput) -> Option<CopyAction> {
    use CopyAction::{Move, Select};
    let ctrl = key.mods.control_key();
    if let Key::Named(named) = key.logical {
        return match named {
            NamedKey::ArrowUp => Some(Move(ViMotion::Up)),
            NamedKey::ArrowDown => Some(Move(ViMotion::Down)),
            NamedKey::ArrowLeft => Some(Move(ViMotion::Left)),
            NamedKey::ArrowRight => Some(Move(ViMotion::Right)),
            NamedKey::Home => Some(Move(ViMotion::First)),
            NamedKey::End => Some(Move(ViMotion::Last)),
            NamedKey::PageUp => Some(CopyAction::PageUp),
            NamedKey::PageDown => Some(CopyAction::PageDown),
            NamedKey::Enter => Some(CopyAction::Copy),
            NamedKey::Escape => Some(CopyAction::Exit),
            _ => None,
        };
    }

    // The char on the key. On a non-Latin layout we use the Latin letter of the physical key.
    let typed = match key.logical {
        Key::Character(s) => s.chars().next(),
        _ => None,
    };
    let c = match typed {
        Some(c) if c.is_ascii() => c,
        _ => {
            let letter = physical_letter(key.physical)?;
            if key.mods.shift_key() {
                letter.to_ascii_uppercase()
            } else {
                letter
            }
        }
    };
    if ctrl {
        return match c.to_ascii_lowercase() {
            'u' => Some(CopyAction::HalfPageUp),
            'd' => Some(CopyAction::HalfPageDown),
            'b' => Some(CopyAction::PageUp),
            'f' => Some(CopyAction::PageDown),
            'v' => Some(Select(SelectionType::Block)),
            _ => None,
        };
    }
    match c {
        'k' => Some(Move(ViMotion::Up)),
        'j' => Some(Move(ViMotion::Down)),
        'h' => Some(Move(ViMotion::Left)),
        'l' => Some(Move(ViMotion::Right)),
        'w' => Some(Move(ViMotion::SemanticRight)),
        'b' => Some(Move(ViMotion::SemanticLeft)),
        'e' => Some(Move(ViMotion::SemanticRightEnd)),
        'W' => Some(Move(ViMotion::WordRight)),
        'B' => Some(Move(ViMotion::WordLeft)),
        'E' => Some(Move(ViMotion::WordRightEnd)),
        '0' => Some(Move(ViMotion::First)),
        '$' => Some(Move(ViMotion::Last)),
        '^' => Some(Move(ViMotion::FirstOccupied)),
        'H' => Some(Move(ViMotion::High)),
        'M' => Some(Move(ViMotion::Middle)),
        'L' => Some(Move(ViMotion::Low)),
        '{' => Some(Move(ViMotion::ParagraphUp)),
        '}' => Some(Move(ViMotion::ParagraphDown)),
        '%' => Some(Move(ViMotion::Bracket)),
        'g' => Some(CopyAction::Top),
        'G' => Some(CopyAction::Bottom),
        'v' => Some(Select(SelectionType::Simple)),
        'V' => Some(Select(SelectionType::Lines)),
        'y' => Some(CopyAction::Copy),
        'q' => Some(CopyAction::Exit),
        _ => None,
    }
}

/// Actions of fterm itself (tabs). They do not go to the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppAction {
    NewTab,
    NextTab,
    PrevTab,
    /// Tab number, from 0.
    SelectTab(usize),
    LastTab,
    MoveTabLeft,
    MoveTabRight,
    RenameTab,
    SplitRight,
    SplitDown,
    /// Focus the neighbor pane.
    FocusPane(Edge),
    /// Move the nearest divider of the active pane.
    ResizePane(Edge),
    ZoomPane,
    /// Close the active pane (the last pane closes the tab).
    ClosePane,
}

/// The tab keys. They use the key position, so they work on every layout.
pub fn app_action(key: &KeyInput) -> Option<AppAction> {
    use AppAction::*;
    let (ctrl, shift, alt) = (
        key.mods.control_key(),
        key.mods.shift_key(),
        key.mods.alt_key(),
    );
    let PhysicalKey::Code(code) = key.physical else {
        return None;
    };
    // Panes: Alt (+ Shift) and arrows, Alt+Shift+= and Alt+Shift+-.
    if alt && !ctrl {
        let edge = match code {
            KeyCode::ArrowLeft => Some(Edge::Left),
            KeyCode::ArrowRight => Some(Edge::Right),
            KeyCode::ArrowUp => Some(Edge::Up),
            KeyCode::ArrowDown => Some(Edge::Down),
            _ => None,
        };
        return match (code, edge, shift) {
            (_, Some(edge), false) => Some(FocusPane(edge)),
            (_, Some(edge), true) => Some(ResizePane(edge)),
            (KeyCode::Equal, None, true) => Some(SplitRight),
            (KeyCode::Minus, None, true) => Some(SplitDown),
            _ => None,
        };
    }
    if !ctrl || alt {
        return None;
    }
    let action = match (code, shift) {
        (KeyCode::Tab, false) | (KeyCode::PageDown, false) => NextTab,
        (KeyCode::Tab, true) | (KeyCode::PageUp, false) => PrevTab,
        (KeyCode::PageUp, true) => MoveTabLeft,
        (KeyCode::PageDown, true) => MoveTabRight,
        (KeyCode::KeyT, true) => NewTab,
        (KeyCode::KeyW, true) => ClosePane,
        (KeyCode::KeyZ, true) => ZoomPane,
        (KeyCode::KeyR, true) => RenameTab,
        (KeyCode::Digit1, true) => SelectTab(0),
        (KeyCode::Digit2, true) => SelectTab(1),
        (KeyCode::Digit3, true) => SelectTab(2),
        (KeyCode::Digit4, true) => SelectTab(3),
        (KeyCode::Digit5, true) => SelectTab(4),
        (KeyCode::Digit6, true) => SelectTab(5),
        (KeyCode::Digit7, true) => SelectTab(6),
        (KeyCode::Digit8, true) => SelectTab(7),
        (KeyCode::Digit9, true) => LastTab,
        _ => return None,
    };
    Some(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(logical: Key, mods: ModifiersState) -> Option<Vec<u8>> {
        press_full(logical, PhysicalKey::Code(KeyCode::KeyZ), None, mods, false)
    }

    fn press_full(
        logical: Key,
        physical: PhysicalKey,
        text: Option<&str>,
        mods: ModifiersState,
        app_cursor: bool,
    ) -> Option<Vec<u8>> {
        let key = KeyInput {
            logical: &logical,
            physical,
            text,
            mods,
        };
        encode_key(&key, app_cursor)
    }

    fn named(k: NamedKey) -> Key {
        Key::Named(k)
    }

    fn ch(s: &str) -> Key {
        Key::Character(s.into())
    }

    const NONE: ModifiersState = ModifiersState::empty();
    const CTRL: ModifiersState = ModifiersState::CONTROL;
    const ALT: ModifiersState = ModifiersState::ALT;
    const SHIFT: ModifiersState = ModifiersState::SHIFT;

    #[test]
    fn text_is_sent_as_utf8() {
        let got = press_full(
            ch("я"),
            PhysicalKey::Code(KeyCode::KeyZ),
            Some("я"),
            NONE,
            false,
        );
        assert_eq!(got, Some("я".as_bytes().to_vec()));
    }

    #[test]
    fn simple_keys() {
        assert_eq!(press(named(NamedKey::Enter), NONE), Some(b"\r".to_vec()));
        assert_eq!(
            press(named(NamedKey::Backspace), NONE),
            Some(b"\x7f".to_vec())
        );
        assert_eq!(
            press(named(NamedKey::Backspace), CTRL),
            Some(b"\x08".to_vec())
        );
        assert_eq!(press(named(NamedKey::Tab), NONE), Some(b"\t".to_vec()));
        assert_eq!(press(named(NamedKey::Tab), SHIFT), Some(b"\x1b[Z".to_vec()));
        assert_eq!(press(named(NamedKey::Escape), NONE), Some(b"\x1b".to_vec()));
    }

    #[test]
    fn ctrl_letter_is_a_control_code() {
        assert_eq!(press(ch("c"), CTRL), Some(vec![0x03]));
        assert_eq!(press(ch("a"), CTRL), Some(vec![0x01]));
        assert_eq!(press(ch("Z"), CTRL), Some(vec![0x1a]));
        assert_eq!(press(ch("["), CTRL), Some(vec![0x1b]));
        assert_eq!(press(ch("]"), CTRL), Some(vec![0x1d]));
        assert_eq!(press(named(NamedKey::Space), CTRL), Some(vec![0x00]));
    }

    #[test]
    fn ctrl_letter_works_on_other_layouts() {
        // Russian layout: the "C" key gives "с", but Ctrl+С must still send ^C.
        let got = press_full(ch("с"), PhysicalKey::Code(KeyCode::KeyC), None, CTRL, false);
        assert_eq!(got, Some(vec![0x03]));
    }

    #[test]
    fn alt_adds_escape() {
        let got = press_full(
            ch("x"),
            PhysicalKey::Code(KeyCode::KeyX),
            Some("x"),
            ALT,
            false,
        );
        assert_eq!(got, Some(b"\x1bx".to_vec()));
        assert_eq!(
            press(named(NamedKey::Backspace), ALT),
            Some(b"\x1b\x7f".to_vec())
        );
    }

    #[test]
    fn arrows_follow_the_cursor_mode() {
        let up = || named(NamedKey::ArrowUp);
        let any = PhysicalKey::Code(KeyCode::ArrowUp);
        assert_eq!(
            press_full(up(), any, None, NONE, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            press_full(up(), any, None, NONE, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            press(named(NamedKey::ArrowDown), NONE),
            Some(b"\x1b[B".to_vec())
        );
        assert_eq!(
            press(named(NamedKey::ArrowRight), NONE),
            Some(b"\x1b[C".to_vec())
        );
        assert_eq!(
            press(named(NamedKey::ArrowLeft), NONE),
            Some(b"\x1b[D".to_vec())
        );
        assert_eq!(press(named(NamedKey::Home), NONE), Some(b"\x1b[H".to_vec()));
        assert_eq!(press(named(NamedKey::End), NONE), Some(b"\x1b[F".to_vec()));
    }

    #[test]
    fn arrows_with_modifiers() {
        // The modifier number is 1 + shift(1) + alt(2) + ctrl(4).
        assert_eq!(
            press(named(NamedKey::ArrowLeft), CTRL),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(
            press(named(NamedKey::ArrowUp), SHIFT),
            Some(b"\x1b[1;2A".to_vec())
        );
        let any = PhysicalKey::Code(KeyCode::ArrowUp);
        // With modifiers, app cursor mode does not change the sequence.
        let got = press_full(named(NamedKey::ArrowUp), any, None, CTRL | SHIFT, true);
        assert_eq!(got, Some(b"\x1b[1;6A".to_vec()));
    }

    #[test]
    fn tilde_keys() {
        assert_eq!(
            press(named(NamedKey::Insert), NONE),
            Some(b"\x1b[2~".to_vec())
        );
        assert_eq!(
            press(named(NamedKey::Delete), NONE),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            press(named(NamedKey::PageUp), NONE),
            Some(b"\x1b[5~".to_vec())
        );
        assert_eq!(
            press(named(NamedKey::PageDown), NONE),
            Some(b"\x1b[6~".to_vec())
        );
        assert_eq!(
            press(named(NamedKey::Delete), CTRL),
            Some(b"\x1b[3;5~".to_vec())
        );
    }

    #[test]
    fn function_keys() {
        assert_eq!(press(named(NamedKey::F1), NONE), Some(b"\x1bOP".to_vec()));
        assert_eq!(press(named(NamedKey::F4), NONE), Some(b"\x1bOS".to_vec()));
        assert_eq!(press(named(NamedKey::F5), NONE), Some(b"\x1b[15~".to_vec()));
        assert_eq!(
            press(named(NamedKey::F12), NONE),
            Some(b"\x1b[24~".to_vec())
        );
        assert_eq!(
            press(named(NamedKey::F1), SHIFT),
            Some(b"\x1b[1;2P".to_vec())
        );
    }

    #[test]
    fn modifier_keys_alone_send_nothing() {
        assert_eq!(press(named(NamedKey::Shift), SHIFT), None);
        assert_eq!(press(named(NamedKey::Control), CTRL), None);
    }

    fn copy_key(logical: Key, mods: ModifiersState) -> Option<CopyAction> {
        let key = KeyInput {
            logical: &logical,
            physical: PhysicalKey::Code(KeyCode::KeyZ),
            text: None,
            mods,
        };
        copy_mode_action(&key)
    }

    #[test]
    fn copy_mode_moves_with_arrows_and_hjkl() {
        use CopyAction::Move;
        assert_eq!(
            copy_key(named(NamedKey::ArrowUp), NONE),
            Some(Move(ViMotion::Up))
        );
        assert_eq!(copy_key(ch("k"), NONE), Some(Move(ViMotion::Up)));
        assert_eq!(copy_key(ch("j"), NONE), Some(Move(ViMotion::Down)));
        assert_eq!(copy_key(ch("h"), NONE), Some(Move(ViMotion::Left)));
        assert_eq!(copy_key(ch("l"), NONE), Some(Move(ViMotion::Right)));
        assert_eq!(copy_key(ch("w"), NONE), Some(Move(ViMotion::SemanticRight)));
        assert_eq!(copy_key(ch("b"), NONE), Some(Move(ViMotion::SemanticLeft)));
        assert_eq!(copy_key(ch("0"), NONE), Some(Move(ViMotion::First)));
        assert_eq!(copy_key(ch("$"), NONE), Some(Move(ViMotion::Last)));
        assert_eq!(
            copy_key(named(NamedKey::Home), NONE),
            Some(Move(ViMotion::First))
        );
    }

    #[test]
    fn copy_mode_pages_and_ends() {
        assert_eq!(
            copy_key(named(NamedKey::PageUp), NONE),
            Some(CopyAction::PageUp)
        );
        assert_eq!(
            copy_key(named(NamedKey::PageDown), NONE),
            Some(CopyAction::PageDown)
        );
        assert_eq!(copy_key(ch("u"), CTRL), Some(CopyAction::HalfPageUp));
        assert_eq!(copy_key(ch("d"), CTRL), Some(CopyAction::HalfPageDown));
        assert_eq!(copy_key(ch("g"), NONE), Some(CopyAction::Top));
        assert_eq!(copy_key(ch("G"), SHIFT), Some(CopyAction::Bottom));
    }

    #[test]
    fn copy_mode_select_copy_and_exit() {
        use CopyAction::Select;
        assert_eq!(copy_key(ch("v"), NONE), Some(Select(SelectionType::Simple)));
        assert_eq!(copy_key(ch("V"), SHIFT), Some(Select(SelectionType::Lines)));
        assert_eq!(copy_key(ch("v"), CTRL), Some(Select(SelectionType::Block)));
        assert_eq!(copy_key(ch("y"), NONE), Some(CopyAction::Copy));
        assert_eq!(
            copy_key(named(NamedKey::Enter), NONE),
            Some(CopyAction::Copy)
        );
        assert_eq!(
            copy_key(named(NamedKey::Escape), NONE),
            Some(CopyAction::Exit)
        );
        assert_eq!(copy_key(ch("q"), NONE), Some(CopyAction::Exit));
    }

    #[test]
    fn copy_mode_ignores_other_keys() {
        assert_eq!(copy_key(ch("x"), NONE), None);
        assert_eq!(copy_key(named(NamedKey::F5), NONE), None);
    }

    #[test]
    fn copy_mode_works_on_the_russian_layout() {
        // On the Russian layout the "K" key gives "л": we use the physical key.
        let logical = ch("л");
        let key = KeyInput {
            logical: &logical,
            physical: PhysicalKey::Code(KeyCode::KeyK),
            text: Some("л"),
            mods: NONE,
        };
        assert_eq!(copy_mode_action(&key), Some(CopyAction::Move(ViMotion::Up)));
    }

    #[test]
    fn shift_enter_is_a_new_line_for_claude() {
        // Claude Code (and many TUI apps) read ESC + CR as "new line, do not send".
        assert_eq!(
            press(named(NamedKey::Enter), SHIFT),
            Some(b"\x1b\r".to_vec())
        );
        assert_eq!(press(named(NamedKey::Enter), NONE), Some(b"\r".to_vec()));
    }

    fn app_key(physical: KeyCode, logical: Key, mods: ModifiersState) -> Option<AppAction> {
        let key = KeyInput {
            logical: &logical,
            physical: PhysicalKey::Code(physical),
            text: None,
            mods,
        };
        app_action(&key)
    }

    const CTRL_SHIFT: ModifiersState = ModifiersState::CONTROL.union(ModifiersState::SHIFT);

    #[test]
    fn tab_keys() {
        use AppAction::*;
        assert_eq!(app_key(KeyCode::KeyT, ch("T"), CTRL_SHIFT), Some(NewTab));
        assert_eq!(app_key(KeyCode::KeyR, ch("R"), CTRL_SHIFT), Some(RenameTab));
        assert_eq!(
            app_key(KeyCode::Tab, named(NamedKey::Tab), CTRL),
            Some(NextTab)
        );
        assert_eq!(
            app_key(KeyCode::Tab, named(NamedKey::Tab), CTRL_SHIFT),
            Some(PrevTab)
        );
        assert_eq!(
            app_key(KeyCode::PageDown, named(NamedKey::PageDown), CTRL),
            Some(NextTab)
        );
        assert_eq!(
            app_key(KeyCode::PageUp, named(NamedKey::PageUp), CTRL),
            Some(PrevTab)
        );
        assert_eq!(
            app_key(KeyCode::PageUp, named(NamedKey::PageUp), CTRL_SHIFT),
            Some(MoveTabLeft)
        );
        assert_eq!(
            app_key(KeyCode::PageDown, named(NamedKey::PageDown), CTRL_SHIFT),
            Some(MoveTabRight)
        );
    }

    #[test]
    fn tab_number_keys() {
        // Shift+1 gives "!" as the char, so the key position is used.
        assert_eq!(
            app_key(KeyCode::Digit1, ch("!"), CTRL_SHIFT),
            Some(AppAction::SelectTab(0))
        );
        assert_eq!(
            app_key(KeyCode::Digit8, ch("*"), CTRL_SHIFT),
            Some(AppAction::SelectTab(7))
        );
        assert_eq!(
            app_key(KeyCode::Digit9, ch("("), CTRL_SHIFT),
            Some(AppAction::LastTab)
        );
    }

    #[test]
    fn tab_keys_work_on_the_russian_layout() {
        // The "T" key gives "Е" on the Russian layout.
        assert_eq!(
            app_key(KeyCode::KeyT, ch("Е"), CTRL_SHIFT),
            Some(AppAction::NewTab)
        );
    }

    #[test]
    fn other_keys_are_not_tab_keys() {
        assert_eq!(
            app_key(KeyCode::KeyT, ch("t"), CTRL),
            None,
            "Ctrl+T goes to the app"
        );
        assert_eq!(app_key(KeyCode::KeyT, ch("T"), SHIFT), None);
        assert_eq!(app_key(KeyCode::Tab, named(NamedKey::Tab), NONE), None);
        assert_eq!(
            app_key(KeyCode::PageUp, named(NamedKey::PageUp), SHIFT),
            None
        );
    }

    const ALT_SHIFT: ModifiersState = ModifiersState::ALT.union(ModifiersState::SHIFT);

    #[test]
    fn pane_keys() {
        use AppAction::*;
        assert_eq!(
            app_key(KeyCode::Equal, ch("+"), ALT_SHIFT),
            Some(SplitRight)
        );
        assert_eq!(app_key(KeyCode::Minus, ch("_"), ALT_SHIFT), Some(SplitDown));
        assert_eq!(
            app_key(KeyCode::ArrowLeft, named(NamedKey::ArrowLeft), ALT),
            Some(FocusPane(Edge::Left))
        );
        assert_eq!(
            app_key(KeyCode::ArrowDown, named(NamedKey::ArrowDown), ALT),
            Some(FocusPane(Edge::Down))
        );
        assert_eq!(
            app_key(KeyCode::ArrowRight, named(NamedKey::ArrowRight), ALT_SHIFT),
            Some(ResizePane(Edge::Right))
        );
        assert_eq!(
            app_key(KeyCode::ArrowUp, named(NamedKey::ArrowUp), ALT_SHIFT),
            Some(ResizePane(Edge::Up))
        );
        assert_eq!(app_key(KeyCode::KeyZ, ch("Z"), CTRL_SHIFT), Some(ZoomPane));
        // Ctrl+Shift+W closes the active pane now (the last pane closes the tab).
        assert_eq!(app_key(KeyCode::KeyW, ch("W"), CTRL_SHIFT), Some(ClosePane));
    }

    #[test]
    fn alt_alone_with_other_keys_goes_to_the_app() {
        // Alt+B, Alt+F and others are word moves in shells: they are not fterm keys.
        assert_eq!(app_key(KeyCode::KeyB, ch("b"), ALT), None);
        assert_eq!(app_key(KeyCode::Equal, ch("="), ALT), None);
    }
}
