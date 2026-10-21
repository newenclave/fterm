//! `ftermctl image`: a picture in the output of this pane (the iTerm2 sequence OSC 1337).

/// The biggest file: its base64 must fit the limit of fterm (32 MiB).
pub const MAX_FILE: usize = 24 * 1024 * 1024;

/// The bytes to write: the picture, `height` rows high (its width keeps the shape), its mark, then the room for it.
/// - `ftermRoom=1`: the new lines make the room (with and without ConPTY), so fterm does not move the cursor.
/// - `ftermMark=<mark>` and an OSC 8 link `fterm-picture:<mark>` on a space at the first cell: the mark of the
///   picture. ConPTY keeps OSC 8 links in its buffer and paints them again (after a resize it paints all lines
///   again, and a mark that only fterm knew would be gone).
///
/// Other terminals skip the keys they do not know, and show a link on one space.
pub fn sequence(name: &str, file: &[u8], height: u64, mark: u32) -> Vec<u8> {
    let mut out = format!(
        "\x1b]1337;File=name={};size={};height={height};inline=1;ftermRoom=1;ftermMark={mark}:",
        crate::mcp::base64(name.as_bytes()),
        file.len()
    )
    .into_bytes();
    out.extend(crate::mcp::base64(file).as_bytes());
    out.push(0x07);
    out.extend(format!("\x1b]8;id={mark};fterm-picture:{mark}\x07 \x1b]8;;\x07").as_bytes());
    // To the start of the line, then down past the picture (new lines only, so its cells stay).
    out.push(b'\r');
    out.extend(std::iter::repeat_n(b'\n', height as usize));
    out
}

/// A mark for a new picture: big (fterm's own marks are below 2^31) and different each time.
pub fn new_mark() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32).rotate_left(16))
        .unwrap_or(0);
    0x8000_0000 | ((nanos ^ std::process::id().rotate_left(7)) & 0x7fff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sequence_the_mark_and_the_room() {
        let out = sequence("a.png", b"ABC", 3, 2_147_483_700);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text,
            concat!(
                "\x1b]1337;File=name=YS5wbmc=;size=3;height=3;inline=1;ftermRoom=1;ftermMark=2147483700:QUJD\x07",
                "\x1b]8;id=2147483700;fterm-picture:2147483700\x07 \x1b]8;;\x07",
                "\r\n\n\n"
            )
        );
    }

    #[test]
    fn marks_are_big() {
        assert!(new_mark() >= 0x8000_0000);
    }
}
