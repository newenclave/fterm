//! The keys of a text UI: the app turns winit keys into these.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key<'a> {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Left,
    Right,
    Enter {
        ctrl: bool,
        shift: bool,
    },
    Escape,
    Backspace,
    Delete,
    /// Typed text (Space too).
    Text(&'a str),
    /// Text from the clipboard (Ctrl+V, Ctrl+Shift+V, Shift+Insert).
    Paste(&'a str),
    /// Tab: the next field of a form.
    Tab,
    /// Shift+Tab: the field before.
    BackTab,
    /// Ctrl+S: save a form.
    Save,
}
