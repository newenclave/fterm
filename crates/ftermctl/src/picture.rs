//! `ftermctl image`: a picture in the output of this pane (the iTerm2 sequence OSC 1337).

/// The biggest file: its base64 must fit the limit of fterm (32 MiB).
pub const MAX_FILE: usize = 24 * 1024 * 1024;

/// The bytes to write: the picture, `height` rows high (its width keeps the shape), then the room for it.
/// `ftermRoom=1` tells fterm that the new lines make the room (with and without ConPTY), so it does not move the
/// cursor itself. Other terminals skip a key they do not know.
pub fn sequence(name: &str, file: &[u8], height: u64) -> Vec<u8> {
    let mut out = format!(
        "\x1b]1337;File=name={};size={};height={height};inline=1;ftermRoom=1:",
        crate::mcp::base64(name.as_bytes()),
        file.len()
    )
    .into_bytes();
    out.extend(crate::mcp::base64(file).as_bytes());
    out.push(0x07);
    // Down past the picture (a new line only, so its cells stay), then to the start of the line.
    out.extend(std::iter::repeat_n(b'\n', height as usize));
    out.push(b'\r');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sequence_and_the_room() {
        let out = sequence("a.png", b"ABC", 3);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text,
            "\x1b]1337;File=name=YS5wbmc=;size=3;height=3;inline=1;ftermRoom=1:QUJD\x07\n\n\n\r"
        );
    }
}
