//! Keyboard: turns a winit key press into bytes for the pty (xterm style).

use fterm_config::copy_keys::{CopyCommand, CopyKey, JumpKind};
use fterm_config::keys::{Key as ChordKey, KeyChord, Mods};
use fterm_term::alacritty_terminal::selection::SelectionType;
use fterm_term::alacritty_terminal::vi_mode::ViMotion;
use fterm_term::copy_mode::{CopyAction, Jump};
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
pub fn physical_letter(physical: PhysicalKey) -> Option<char> {
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

/// A key in copy mode, as the copy mode key table writes it: the typed char with its case ("G", "$"),
/// or a named key. On a non-Latin layout a letter comes from the physical key ("л" on the K key = "k").
/// `None` for keys that are only modifiers.
pub fn copy_key(key: &KeyInput) -> Option<CopyKey> {
    let ctrl = key.mods.control_key();
    let alt = key.mods.alt_key();
    if let Key::Named(named) = key.logical {
        let name = match named {
            NamedKey::ArrowUp => ChordKey::Up,
            NamedKey::ArrowDown => ChordKey::Down,
            NamedKey::ArrowLeft => ChordKey::Left,
            NamedKey::ArrowRight => ChordKey::Right,
            NamedKey::Home => ChordKey::Home,
            NamedKey::End => ChordKey::End,
            NamedKey::PageUp => ChordKey::PageUp,
            NamedKey::PageDown => ChordKey::PageDown,
            NamedKey::Enter => ChordKey::Enter,
            NamedKey::Escape => ChordKey::Escape,
            NamedKey::Tab => ChordKey::Tab,
            NamedKey::Backspace => ChordKey::Backspace,
            NamedKey::Delete => ChordKey::Delete,
            NamedKey::Insert => ChordKey::Insert,
            NamedKey::Space => ChordKey::Space,
            other => ChordKey::F(function_key(*other)?),
        };
        return Some(CopyKey {
            ctrl,
            alt,
            shift: key.mods.shift_key(),
            key: name,
        });
    }
    let typed = match key.logical {
        Key::Character(s) => s.chars().next(),
        _ => None,
    };
    let c = match typed {
        Some(' ') => {
            return Some(CopyKey {
                ctrl,
                alt,
                shift: false,
                key: ChordKey::Space,
            });
        }
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
    let mut out = CopyKey::char(c);
    out.alt = alt;
    Some(if ctrl { out.with_ctrl() } else { out })
}

fn function_key(named: NamedKey) -> Option<u8> {
    use NamedKey::*;
    let keys = [
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20,
        F21, F22, F23, F24,
    ];
    keys.iter().position(|k| *k == named).map(|i| i as u8 + 1)
}

/// What a copy mode command does now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyStep {
    Run(CopyAction),
    /// `f` `F` `t` `T`: wait for the char.
    WaitJump(Jump),
}

/// A command of the copy mode key table as a step of the engine.
pub fn copy_step(command: CopyCommand, exit_after_copy: bool) -> CopyStep {
    use CopyAction as A;
    use CopyCommand as C;
    let jump = |kind: JumpKind| match kind {
        JumpKind::Forward => Jump::Forward,
        JumpKind::Backward => Jump::Backward,
        JumpKind::TillForward => Jump::TillForward,
        JumpKind::TillBackward => Jump::TillBackward,
    };
    let stay = !exit_after_copy;
    CopyStep::Run(match command {
        C::Up => A::Move(ViMotion::Up),
        C::Down => A::Move(ViMotion::Down),
        C::Left => A::Move(ViMotion::Left),
        C::Right => A::Move(ViMotion::Right),
        C::WordRight => A::Move(ViMotion::SemanticRight),
        C::WordLeft => A::Move(ViMotion::SemanticLeft),
        C::WordEnd => A::Move(ViMotion::SemanticRightEnd),
        C::BigWordRight => A::Move(ViMotion::WordRight),
        C::BigWordLeft => A::Move(ViMotion::WordLeft),
        C::BigWordEnd => A::Move(ViMotion::WordRightEnd),
        C::LineStart => A::Move(ViMotion::First),
        C::LineEnd => A::Move(ViMotion::Last),
        C::FirstChar => A::Move(ViMotion::FirstOccupied),
        C::ScreenTop => A::Move(ViMotion::High),
        C::ScreenMiddle => A::Move(ViMotion::Middle),
        C::ScreenBottom => A::Move(ViMotion::Low),
        C::ParagraphUp => A::Move(ViMotion::ParagraphUp),
        C::ParagraphDown => A::Move(ViMotion::ParagraphDown),
        C::Bracket => A::Move(ViMotion::Bracket),
        C::PageUp => A::PageUp,
        C::PageDown => A::PageDown,
        C::HalfPageUp => A::HalfPageUp,
        C::HalfPageDown => A::HalfPageDown,
        C::Top => A::Top,
        C::Bottom => A::Bottom,
        C::Select => A::Select(SelectionType::Simple),
        C::SelectLines => A::Select(SelectionType::Lines),
        C::SelectBlock => A::Select(SelectionType::Block),
        C::SelectWord => A::SelectWord {
            big: false,
            around: false,
        },
        C::SelectWordAround => A::SelectWord {
            big: false,
            around: true,
        },
        C::SelectBigWord => A::SelectWord {
            big: true,
            around: false,
        },
        C::SelectBigWordAround => A::SelectWord {
            big: true,
            around: true,
        },
        C::SwapEnds => A::SwapEnds,
        C::Copy => A::Copy { stay },
        C::CopyAndStay => A::Copy { stay: true },
        C::CopyAndExit => A::Copy { stay: false },
        C::CopyLine => A::CopyLine { stay },
        C::Exit => A::Exit,
        C::SearchForward => A::SearchStart { forward: true },
        C::SearchBackward => A::SearchStart { forward: false },
        C::NextMatch => A::NextMatch { same_way: true },
        C::PrevMatch => A::NextMatch { same_way: false },
        C::Jump(kind) => return CopyStep::WaitJump(jump(kind)),
        C::JumpTo(kind, c) => A::JumpTo(jump(kind), c),
        C::RepeatJump => A::RepeatJump { same_way: true },
        C::RepeatJumpBack => A::RepeatJump { same_way: false },
    })
}

/// The key as a `KeyChord` for the keymap. It uses the key position, so it works on every layout.
/// `None` for keys that are only modifiers.
pub fn key_chord(key: &KeyInput) -> Option<KeyChord> {
    let PhysicalKey::Code(code) = key.physical else {
        return None;
    };
    let chord_key = match code {
        KeyCode::Tab => ChordKey::Tab,
        KeyCode::Enter | KeyCode::NumpadEnter => ChordKey::Enter,
        KeyCode::Escape => ChordKey::Escape,
        KeyCode::Space => ChordKey::Space,
        KeyCode::Backspace => ChordKey::Backspace,
        KeyCode::Delete => ChordKey::Delete,
        KeyCode::Insert => ChordKey::Insert,
        KeyCode::Home => ChordKey::Home,
        KeyCode::End => ChordKey::End,
        KeyCode::PageUp => ChordKey::PageUp,
        KeyCode::PageDown => ChordKey::PageDown,
        KeyCode::ArrowUp => ChordKey::Up,
        KeyCode::ArrowDown => ChordKey::Down,
        KeyCode::ArrowLeft => ChordKey::Left,
        KeyCode::ArrowRight => ChordKey::Right,
        KeyCode::Equal => ChordKey::Char('='),
        KeyCode::Minus => ChordKey::Char('-'),
        KeyCode::Semicolon => ChordKey::Char(';'),
        KeyCode::Quote => ChordKey::Char('\''),
        KeyCode::Comma => ChordKey::Char(','),
        KeyCode::Period => ChordKey::Char('.'),
        KeyCode::Slash => ChordKey::Char('/'),
        KeyCode::Backquote => ChordKey::Char('`'),
        KeyCode::Digit0 => ChordKey::Char('0'),
        KeyCode::Digit1 => ChordKey::Char('1'),
        KeyCode::Digit2 => ChordKey::Char('2'),
        KeyCode::Digit3 => ChordKey::Char('3'),
        KeyCode::Digit4 => ChordKey::Char('4'),
        KeyCode::Digit5 => ChordKey::Char('5'),
        KeyCode::Digit6 => ChordKey::Char('6'),
        KeyCode::Digit7 => ChordKey::Char('7'),
        KeyCode::Digit8 => ChordKey::Char('8'),
        KeyCode::Digit9 => ChordKey::Char('9'),
        KeyCode::F1 => ChordKey::F(1),
        KeyCode::F2 => ChordKey::F(2),
        KeyCode::F3 => ChordKey::F(3),
        KeyCode::F4 => ChordKey::F(4),
        KeyCode::F5 => ChordKey::F(5),
        KeyCode::F6 => ChordKey::F(6),
        KeyCode::F7 => ChordKey::F(7),
        KeyCode::F8 => ChordKey::F(8),
        KeyCode::F9 => ChordKey::F(9),
        KeyCode::F10 => ChordKey::F(10),
        KeyCode::F11 => ChordKey::F(11),
        KeyCode::F12 => ChordKey::F(12),
        // The numpad with NumLock off (or with Shift): its keys are Ins, Del, Home, End, ...
        // Many laptops have these only there. With NumLock on they are digits: not for the keymap.
        KeyCode::Numpad0
        | KeyCode::Numpad1
        | KeyCode::Numpad2
        | KeyCode::Numpad3
        | KeyCode::Numpad4
        | KeyCode::Numpad6
        | KeyCode::Numpad7
        | KeyCode::Numpad8
        | KeyCode::Numpad9
        | KeyCode::NumpadDecimal => match key.logical {
            Key::Named(NamedKey::Insert) => ChordKey::Insert,
            Key::Named(NamedKey::Delete) => ChordKey::Delete,
            Key::Named(NamedKey::Home) => ChordKey::Home,
            Key::Named(NamedKey::End) => ChordKey::End,
            Key::Named(NamedKey::PageUp) => ChordKey::PageUp,
            Key::Named(NamedKey::PageDown) => ChordKey::PageDown,
            Key::Named(NamedKey::ArrowUp) => ChordKey::Up,
            Key::Named(NamedKey::ArrowDown) => ChordKey::Down,
            Key::Named(NamedKey::ArrowLeft) => ChordKey::Left,
            Key::Named(NamedKey::ArrowRight) => ChordKey::Right,
            _ => return None,
        },
        // Letters and the bracket and backslash keys.
        _ => ChordKey::Char(physical_letter(key.physical)?),
    };
    Some(KeyChord {
        mods: Mods {
            ctrl: key.mods.control_key(),
            shift: key.mods.shift_key(),
            alt: key.mods.alt_key(),
            logo: key.mods.super_key(),
        },
        key: chord_key,
    })
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

    /// The step of a key with the default copy mode keys (`physical` for the Russian layout test).
    fn copy_with(logical: Key, physical: KeyCode, mods: ModifiersState) -> Option<CopyStep> {
        use fterm_config::copy_keys::{CopyBinding, CopyKeymap, Lookup};
        let key = KeyInput {
            logical: &logical,
            physical: PhysicalKey::Code(physical),
            text: None,
            mods,
        };
        match CopyKeymap::with_defaults().lookup(&[copy_key(&key)?]) {
            Lookup::Found(CopyBinding::Command(c)) => Some(copy_step(c, true)),
            _ => None,
        }
    }

    fn copy_step_of(logical: Key, mods: ModifiersState) -> Option<CopyAction> {
        match copy_with(logical, KeyCode::KeyZ, mods)? {
            CopyStep::Run(action) => Some(action),
            CopyStep::WaitJump(_) => None,
        }
    }

    #[test]
    fn copy_mode_moves_with_arrows_and_hjkl() {
        use CopyAction::Move;
        assert_eq!(
            copy_step_of(named(NamedKey::ArrowUp), NONE),
            Some(Move(ViMotion::Up))
        );
        assert_eq!(copy_step_of(ch("k"), NONE), Some(Move(ViMotion::Up)));
        assert_eq!(copy_step_of(ch("j"), NONE), Some(Move(ViMotion::Down)));
        assert_eq!(copy_step_of(ch("h"), NONE), Some(Move(ViMotion::Left)));
        assert_eq!(copy_step_of(ch("l"), NONE), Some(Move(ViMotion::Right)));
        assert_eq!(
            copy_step_of(ch("w"), NONE),
            Some(Move(ViMotion::SemanticRight))
        );
        assert_eq!(
            copy_step_of(ch("b"), NONE),
            Some(Move(ViMotion::SemanticLeft))
        );
        assert_eq!(copy_step_of(ch("0"), NONE), Some(Move(ViMotion::First)));
        assert_eq!(copy_step_of(ch("$"), SHIFT), Some(Move(ViMotion::Last)));
        assert_eq!(
            copy_step_of(named(NamedKey::Home), NONE),
            Some(Move(ViMotion::First))
        );
    }

    #[test]
    fn copy_mode_pages_and_ends() {
        assert_eq!(
            copy_step_of(named(NamedKey::PageUp), NONE),
            Some(CopyAction::PageUp)
        );
        assert_eq!(
            copy_step_of(named(NamedKey::PageDown), NONE),
            Some(CopyAction::PageDown)
        );
        assert_eq!(copy_step_of(ch("u"), CTRL), Some(CopyAction::HalfPageUp));
        assert_eq!(copy_step_of(ch("d"), CTRL), Some(CopyAction::HalfPageDown));
        assert_eq!(copy_step_of(ch("g"), NONE), Some(CopyAction::Top));
        assert_eq!(copy_step_of(ch("G"), SHIFT), Some(CopyAction::Bottom));
    }

    #[test]
    fn copy_mode_select_copy_and_exit() {
        use CopyAction::Select;
        assert_eq!(
            copy_step_of(ch("v"), NONE),
            Some(Select(SelectionType::Simple))
        );
        assert_eq!(
            copy_step_of(ch("V"), SHIFT),
            Some(Select(SelectionType::Lines))
        );
        assert_eq!(
            copy_step_of(ch("v"), CTRL),
            Some(Select(SelectionType::Block))
        );
        assert_eq!(
            copy_step_of(ch("y"), NONE),
            Some(CopyAction::Copy { stay: false })
        );
        assert_eq!(
            copy_step_of(named(NamedKey::Enter), NONE),
            Some(CopyAction::Copy { stay: false })
        );
        assert_eq!(
            copy_step_of(named(NamedKey::Escape), NONE),
            Some(CopyAction::Exit)
        );
        assert_eq!(copy_step_of(ch("q"), NONE), Some(CopyAction::Exit));
    }

    #[test]
    fn copy_mode_ignores_other_keys() {
        assert_eq!(copy_step_of(ch("x"), NONE), None);
        assert_eq!(copy_step_of(named(NamedKey::F5), NONE), None);
        assert_eq!(
            copy_key(&KeyInput {
                logical: &named(NamedKey::Shift),
                physical: PhysicalKey::Code(KeyCode::ShiftLeft),
                text: None,
                mods: SHIFT,
            }),
            None
        );
    }

    #[test]
    fn copy_mode_works_on_the_russian_layout() {
        // On the Russian layout the "K" key gives "л": we use the physical key.
        assert_eq!(
            copy_with(ch("л"), KeyCode::KeyK, NONE),
            Some(CopyStep::Run(CopyAction::Move(ViMotion::Up)))
        );
        assert_eq!(
            copy_with(ch("П"), KeyCode::KeyG, SHIFT),
            Some(CopyStep::Run(CopyAction::Bottom)),
            "Shift gives the capital letter"
        );
    }

    #[test]
    fn copy_mode_new_keys() {
        assert_eq!(
            copy_step_of(ch("/"), NONE),
            Some(CopyAction::SearchStart { forward: true })
        );
        assert_eq!(
            copy_step_of(ch("N"), SHIFT),
            Some(CopyAction::NextMatch { same_way: false })
        );
        assert_eq!(
            copy_with(ch("t"), KeyCode::KeyT, NONE),
            Some(CopyStep::WaitJump(Jump::TillForward))
        );
        assert_eq!(
            copy_step_of(ch("Y"), SHIFT),
            Some(CopyAction::CopyLine { stay: false })
        );
        assert_eq!(
            copy_step(CopyCommand::Copy, false),
            CopyStep::Run(CopyAction::Copy { stay: true }),
            "exit_after_copy = false"
        );
        assert_eq!(
            copy_step(CopyCommand::JumpTo(JumpKind::Backward, 'x'), true),
            CopyStep::Run(CopyAction::JumpTo(Jump::Backward, 'x'))
        );
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

    fn chord_of(physical: KeyCode, logical: Key, mods: ModifiersState) -> Option<KeyChord> {
        let key = KeyInput {
            logical: &logical,
            physical: PhysicalKey::Code(physical),
            text: None,
            mods,
        };
        key_chord(&key)
    }

    fn parsed(text: &str) -> Option<KeyChord> {
        Some(KeyChord::parse(text).unwrap())
    }

    const CTRL_SHIFT: ModifiersState = ModifiersState::CONTROL.union(ModifiersState::SHIFT);
    const ALT_SHIFT: ModifiersState = ModifiersState::ALT.union(ModifiersState::SHIFT);

    #[test]
    fn the_numpad_keys_with_numlock_off() {
        // Many laptops have Ins, Home, End, ... only on the numpad: the key is a numpad key,
        // its meaning is Insert (NumLock off, or Shift).
        for (code, named_key, text) in [
            (KeyCode::Numpad0, NamedKey::Insert, "shift+insert"),
            (KeyCode::NumpadDecimal, NamedKey::Delete, "shift+delete"),
            (KeyCode::Numpad7, NamedKey::Home, "shift+home"),
            (KeyCode::Numpad1, NamedKey::End, "shift+end"),
            (KeyCode::Numpad9, NamedKey::PageUp, "shift+pageup"),
            (KeyCode::Numpad3, NamedKey::PageDown, "shift+pagedown"),
            (KeyCode::Numpad8, NamedKey::ArrowUp, "shift+up"),
            (KeyCode::Numpad2, NamedKey::ArrowDown, "shift+down"),
            (KeyCode::Numpad4, NamedKey::ArrowLeft, "shift+left"),
            (KeyCode::Numpad6, NamedKey::ArrowRight, "shift+right"),
        ] {
            assert_eq!(
                chord_of(code, named(named_key), SHIFT),
                parsed(text),
                "{text}"
            );
        }
        assert_eq!(
            chord_of(KeyCode::Numpad0, named(NamedKey::Insert), CTRL),
            parsed("ctrl+insert")
        );
        // NumLock on: a numpad digit is a digit, not a key for the keymap.
        assert_eq!(chord_of(KeyCode::Numpad0, ch("0"), NONE), None);
    }

    #[test]
    fn chords_from_keys() {
        assert_eq!(
            chord_of(KeyCode::KeyT, ch("T"), CTRL_SHIFT),
            parsed("ctrl+shift+t")
        );
        assert_eq!(
            chord_of(KeyCode::Tab, named(NamedKey::Tab), CTRL),
            parsed("ctrl+tab")
        );
        assert_eq!(
            chord_of(KeyCode::PageUp, named(NamedKey::PageUp), SHIFT),
            parsed("shift+pageup")
        );
        assert_eq!(
            chord_of(KeyCode::ArrowLeft, named(NamedKey::ArrowLeft), ALT),
            parsed("alt+left")
        );
        assert_eq!(
            chord_of(KeyCode::F11, named(NamedKey::F11), NONE),
            parsed("f11")
        );
        assert_eq!(
            chord_of(KeyCode::Space, named(NamedKey::Space), CTRL_SHIFT),
            parsed("ctrl+shift+space")
        );
        assert_eq!(
            chord_of(KeyCode::Insert, named(NamedKey::Insert), SHIFT),
            parsed("shift+insert")
        );
    }

    #[test]
    fn symbols_and_digits_use_the_key_not_the_char() {
        // Shift+= gives "+" and Shift+1 gives "!", but the key is "=" and "1".
        assert_eq!(
            chord_of(KeyCode::Equal, ch("+"), ALT_SHIFT),
            parsed("alt+shift+=")
        );
        assert_eq!(
            chord_of(KeyCode::Minus, ch("_"), ALT_SHIFT),
            parsed("alt+shift+-")
        );
        assert_eq!(
            chord_of(KeyCode::Digit1, ch("!"), CTRL_SHIFT),
            parsed("ctrl+shift+1")
        );
        assert_eq!(
            chord_of(KeyCode::Comma, ch("<"), CTRL_SHIFT),
            parsed("ctrl+shift+,")
        );
    }

    #[test]
    fn chords_work_on_the_russian_layout() {
        assert_eq!(
            chord_of(KeyCode::KeyT, ch("Е"), CTRL_SHIFT),
            parsed("ctrl+shift+t")
        );
    }

    #[test]
    fn modifier_keys_alone_are_not_chords() {
        assert_eq!(
            chord_of(KeyCode::ShiftLeft, named(NamedKey::Shift), SHIFT),
            None
        );
        assert_eq!(
            chord_of(KeyCode::ControlLeft, named(NamedKey::Control), CTRL),
            None
        );
    }

    #[test]
    fn default_keymap_works_with_real_keys() {
        use fterm_config::keys::{Action, BuiltinAction, Keymap};
        let keys = Keymap::with_defaults();
        let action =
            |code, logical, mods| chord_of(code, logical, mods).and_then(|c| keys.get(&c).cloned());
        assert_eq!(
            action(KeyCode::KeyT, ch("Е"), CTRL_SHIFT),
            Some(Action::Builtin(BuiltinAction::NewTab))
        );
        assert_eq!(
            action(KeyCode::Equal, ch("+"), ALT_SHIFT),
            Some(Action::Builtin(BuiltinAction::SplitRight))
        );
        assert_eq!(
            action(KeyCode::Digit9, ch("("), CTRL_SHIFT),
            Some(Action::Builtin(BuiltinAction::LastTab))
        );
        // Plain Ctrl+T and Alt+B go to the app.
        assert_eq!(action(KeyCode::KeyT, ch("t"), CTRL), None);
        assert_eq!(action(KeyCode::KeyB, ch("b"), ALT), None);
    }
}
