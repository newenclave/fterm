//! Keyboard: turns a winit key press into bytes for the pty (xterm style).

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
    let PhysicalKey::Code(code) = key.physical else {
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
    from_char(letter)
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
}
