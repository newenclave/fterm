//! Clipboard: copy and paste, and how pasted text goes to the shell.

/// Bytes to send to the pty for a paste.
/// - ESC is removed, so the text cannot end the paste bracket early or send key codes.
/// - Line ends become `\r`, like the Enter key.
/// - In bracketed paste mode (the app asked for it) the text is put between `ESC[200~` and `ESC[201~`.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let clean = text
        .replace("\r\n", "\r")
        .replace('\n', "\r")
        .replace('\x1b', "");
    if bracketed {
        format!("\x1b[200~{clean}\x1b[201~").into_bytes()
    } else {
        clean.into_bytes()
    }
}

/// The system clipboard. It can fail (for example, another app holds it), so errors are only logged.
pub struct Clipboard {
    inner: Option<arboard::Clipboard>,
}

impl Clipboard {
    pub fn new() -> Self {
        let inner = arboard::Clipboard::new()
            .inspect_err(|err| tracing::warn!("no clipboard: {err}"))
            .ok();
        Self { inner }
    }

    pub fn copy(&mut self, text: &str) {
        if let Some(clipboard) = &mut self.inner
            && let Err(err) = clipboard.set_text(text)
        {
            tracing::warn!("cannot copy: {err}");
        }
    }

    pub fn paste(&mut self) -> Option<String> {
        let clipboard = self.inner.as_mut()?;
        clipboard
            .get_text()
            .inspect_err(|err| tracing::warn!("cannot paste: {err}"))
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_sent_as_is() {
        assert_eq!(paste_bytes("echo hi", false), b"echo hi");
        assert_eq!(paste_bytes("привет", false), "привет".as_bytes());
    }

    #[test]
    fn line_ends_become_carriage_returns() {
        assert_eq!(paste_bytes("a\r\nb\nc", false), b"a\rb\rc");
    }

    #[test]
    fn escape_is_removed() {
        assert_eq!(paste_bytes("a\x1b[201~b\x1bc", false), b"a[201~bc");
    }

    #[test]
    fn bracketed_paste_wraps_the_text() {
        assert_eq!(paste_bytes("ls\n", true), b"\x1b[200~ls\r\x1b[201~");
    }

    #[test]
    fn text_cannot_end_the_bracket_early() {
        let bytes = paste_bytes("x\x1b[201~rm -rf /\n", true);
        let text = String::from_utf8(bytes).unwrap();
        // Only one end marker: the real one at the end.
        assert_eq!(text.matches("\x1b[201~").count(), 1);
        assert!(text.ends_with("\x1b[201~"));
    }
}
