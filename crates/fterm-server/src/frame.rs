//! Frames on the socket: `[u32 length (little endian)][u8 kind][payload]`. The length counts the kind and the
//! payload.

use std::io::{self, Read, Write};

/// A control message (JSON, see `proto`).
pub const CONTROL: u8 = 0;
/// Output (server → window) or input (window → server) of a pane: `[u64 pane][bytes]`.
pub const DATA: u8 = 1;
/// The snapshot of a pane before the answer to `attach`: `[u64 pane][bytes]`.
pub const SNAPSHOT: u8 = 2;

/// The biggest frame we read (a snapshot of a big history can be some megabytes).
pub const MAX_FRAME: usize = 256 * 1024 * 1024;

pub fn write_frame(w: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len() + 1).map_err(|_| io::Error::other("frame too big"))?;
    let mut head = [0u8; 5];
    head[..4].copy_from_slice(&len.to_le_bytes());
    head[4] = kind;
    w.write_all(&head)?;
    w.write_all(payload)?;
    w.flush()
}

/// A data or snapshot frame of `pane`.
pub fn write_pane(w: &mut impl Write, kind: u8, pane: u64, bytes: &[u8]) -> io::Result<()> {
    let mut payload = Vec::with_capacity(8 + bytes.len());
    payload.extend_from_slice(&pane.to_le_bytes());
    payload.extend_from_slice(bytes);
    write_frame(w, kind, &payload)
}

/// The next frame: `(kind, payload)`. `Ok(None)` = the other side closed the socket.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("bad frame length {len}"),
        ));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let kind = body.remove(0);
    Ok(Some((kind, body)))
}

/// The pane and the bytes of a data or snapshot payload.
pub fn split_pane(payload: &[u8]) -> Option<(u64, &[u8])> {
    let (id, bytes) = payload.split_at_checked(8)?;
    Some((u64::from_le_bytes(id.try_into().ok()?), bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_go_through_in_order() {
        let mut wire = Vec::new();
        write_frame(&mut wire, CONTROL, br#"{"op":"list"}"#).unwrap();
        write_pane(&mut wire, DATA, 7, b"hello").unwrap();
        write_frame(&mut wire, CONTROL, b"").unwrap();
        let mut r = &wire[..];
        assert_eq!(
            read_frame(&mut r).unwrap(),
            Some((CONTROL, br#"{"op":"list"}"#.to_vec()))
        );
        let (kind, payload) = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(kind, DATA);
        assert_eq!(split_pane(&payload), Some((7, &b"hello"[..])));
        assert_eq!(read_frame(&mut r).unwrap(), Some((CONTROL, Vec::new())));
        assert_eq!(read_frame(&mut r).unwrap(), None, "the end");
    }

    #[test]
    fn bad_frames_are_errors() {
        let zero = [0u8, 0, 0, 0];
        assert!(read_frame(&mut &zero[..]).is_err());
        let huge = u32::MAX.to_le_bytes();
        assert!(read_frame(&mut &huge[..]).is_err());
        // Cut in the middle.
        let mut wire = Vec::new();
        write_pane(&mut wire, DATA, 1, b"abcdef").unwrap();
        assert!(read_frame(&mut &wire[..wire.len() - 2]).is_err());
        assert_eq!(split_pane(b"short"), None);
    }
}
