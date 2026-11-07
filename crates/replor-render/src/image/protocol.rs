//! Terminal graphics escape sequences.
//!
//! Both protocols take the original file bytes and decode them terminal side,
//! so nothing here interprets image data. Every sequence produced here leaves
//! the cursor where it was, and the caller decides the position.

/// Graphics protocol a terminal understands. Detection is by environment
/// variable only; the terminal is never queried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Kitty,
    Iterm2,
}

/// The largest base64 payload a single kitty escape may carry.
const KITTY_CHUNK: usize = 4096;

const ST: &str = "\x1b\\";

/// The placement id of every kitty image. `mdvu` draws each image once.
const PLACEMENT: u32 = 1;
/// DECSC / DECRC. The iTerm2 sequence advances the cursor past the image, so it
/// is bracketed to keep the "does not move the cursor" contract.
const SAVE_CURSOR: &str = "\x1b7";
const RESTORE_CURSOR: &str = "\x1b8";

/// Draws `bytes` at the cursor, occupying `cols` x `rows` cells.
pub fn place(protocol: Protocol, bytes: &[u8], cols: usize, rows: usize) -> String {
    let payload = base64(bytes);
    match protocol {
        Protocol::Kitty => kitty(&format!("a=T,f=100,c={cols},r={rows},C=1"), &payload),
        Protocol::Iterm2 => format!(
            "{SAVE_CURSOR}\x1b]1337;File=inline=1;size={};width={cols};height={rows};preserveAspectRatio=1:{payload}\x07{RESTORE_CURSOR}",
            bytes.len()
        ),
    }
}

/// Stores `bytes` in a kitty terminal under `id` without drawing it, so moving
/// the image later costs a [`show`] rather than the whole payload again.
/// `q=2` silences the reply an id would otherwise draw, which would arrive as
/// input.
pub fn transmit(bytes: &[u8], id: u32) -> String {
    kitty(&format!("a=t,f=100,i={id},q=2"), &base64(bytes))
}

/// Draws the kitty image stored under `id` at the cursor, occupying `cols` x
/// `rows` cells. Each image has one placement, so drawing it again replaces the
/// previous one: that is how an image moves.
pub fn show(id: u32, cols: usize, rows: usize) -> String {
    format!("\x1b_Ga=p,i={id},p={PLACEMENT},c={cols},r={rows},C=1,q=2{ST}")
}

/// Draws the `w` x `h` pixel rectangle at `x`, `y` of the kitty image stored
/// under `id`, scaled to `cols` x `rows` cells.
pub fn show_part(id: u32, cols: usize, rows: usize, (x, y, w, h): (u32, u32, u32, u32)) -> String {
    format!("\x1b_Ga=p,i={id},p={PLACEMENT},x={x},y={y},w={w},h={h},c={cols},r={rows},C=1,q=2{ST}")
}

/// Removes the kitty image `id` from the screen and keeps what the terminal
/// stores, so it can be drawn again without being sent.
///
/// Only this image's placement is named: an image placed again after `d=a`,
/// which clears every placement, may not be drawn (`docs/mermaid-image.md`).
pub fn remove(id: u32) -> String {
    format!("\x1b_Ga=d,d=i,i={id},p={PLACEMENT},q=2{ST}")
}

/// Removes the kitty image `id` and frees what the terminal stores for it,
/// whether or not it is on screen. `d=A` would free only images with a
/// placement on screen.
pub fn free(id: u32) -> String {
    format!("\x1b_Ga=d,d=I,i={id},q=2{ST}")
}

/// `f=100` says the payload is a complete image file, and `C=1` keeps the
/// cursor still. Payloads are sent in chunks because a single escape is length
/// limited; only the first chunk carries the keys, and every chunk carries
/// `q=2` because a reply would arrive as input.
fn kitty(keys: &str, payload: &str) -> String {
    let mut out = String::with_capacity(payload.len() + 64);
    let mut chunks = payload
        .as_bytes()
        .chunks(KITTY_CHUNK)
        .map(|chunk| std::str::from_utf8(chunk).expect("base64 is ascii"))
        .peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        if first {
            out.push_str(&format!("\x1b_G{keys},m={more};{chunk}{ST}"));
            first = false;
        } else {
            out.push_str(&format!("\x1b_Gm={more},q=2;{chunk}{ST}"));
        }
    }
    out
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding. Written out rather than taken from a crate:
/// this is the only place `mdvu` needs it.
fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let b = [
            group[0],
            *group.get(1).unwrap_or(&0),
            *group.get(2).unwrap_or(&0),
        ];
        let bits = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= group.len() {
                out.push(ALPHABET[((bits >> (18 - 6 * i)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_test_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn a_short_kitty_payload_is_one_escape() {
        let out = place(Protocol::Kitty, b"foobar", 10, 5);
        assert_eq!(out, "\x1b_Ga=T,f=100,c=10,r=5,C=1,m=0;Zm9vYmFy\x1b\\");
    }

    #[test]
    fn a_long_kitty_payload_is_chunked_and_terminated() {
        let bytes = vec![0u8; KITTY_CHUNK * 2];
        let out = place(Protocol::Kitty, &bytes, 4, 2);
        assert_eq!(out.matches("\x1b_G").count(), 3);
        assert!(out.starts_with("\x1b_Ga=T,f=100,c=4,r=2,C=1,m=1;"));
        assert!(out.contains("\x1b_Gm=1,q=2;"));
        // Only the final chunk clears the "more data follows" flag.
        assert_eq!(out.matches("m=0").count(), 1);
        assert!(out.ends_with(ST));
    }

    #[test]
    fn the_iterm2_sequence_carries_the_byte_count_and_cell_size() {
        let out = place(Protocol::Iterm2, b"foobar", 10, 5);
        assert!(out.contains("size=6;"));
        assert!(out.contains("width=10;height=5;"));
        assert!(out.contains(":Zm9vYmFy\x07"));
    }

    /// The caller positions the cursor itself, so neither sequence may leave it
    /// somewhere else.
    #[test]
    fn neither_protocol_moves_the_cursor() {
        let kitty = place(Protocol::Kitty, b"foobar", 1, 1);
        assert!(kitty.contains("C=1"));
        let iterm2 = place(Protocol::Iterm2, b"foobar", 1, 1);
        assert!(iterm2.starts_with(SAVE_CURSOR));
        assert!(iterm2.ends_with(RESTORE_CURSOR));
    }

    /// With an id the terminal would answer every command, and the answer
    /// would arrive as input; `q=2` silences it.
    #[test]
    fn a_kitty_transmission_stores_the_image_quietly_without_drawing_it() {
        let out = transmit(b"foobar", 7);
        assert_eq!(out, "\x1b_Ga=t,f=100,i=7,q=2,m=0;Zm9vYmFy\x1b\\");
    }

    #[test]
    fn a_long_kitty_transmission_is_chunked() {
        let out = transmit(&vec![0u8; KITTY_CHUNK * 2], 7);
        assert!(out.starts_with("\x1b_Ga=t,f=100,i=7,q=2,m=1;"));
        assert_eq!(out.matches("m=0").count(), 1);
    }

    /// A reply arrives as input, and the pager reads its `G` as "go to the
    /// end". Whether a terminal takes `q` from the first chunk or the last
    /// varies, so every chunk carries it.
    #[test]
    fn every_chunk_of_a_kitty_transmission_is_quiet() {
        let out = transmit(&vec![0u8; KITTY_CHUNK * 3], 7);
        let escapes = out.matches("\x1b_G").count();
        assert_eq!(escapes, 4);
        assert_eq!(out.matches("q=2").count(), escapes);
    }

    /// The fixed placement id makes a second `a=p` replace the first, which
    /// is how an image moves without being deleted.
    #[test]
    fn a_stored_image_is_placed_by_id_without_moving_the_cursor() {
        assert_eq!(show(7, 10, 5), "\x1b_Ga=p,i=7,p=1,c=10,r=5,C=1,q=2\x1b\\");
        assert_eq!(
            show_part(7, 10, 3, (0, 20, 100, 60)),
            "\x1b_Ga=p,i=7,p=1,x=0,y=20,w=100,h=60,c=10,r=3,C=1,q=2\x1b\\"
        );
    }

    /// Lowercase `d=i` removes the placement and keeps the stored data, so the
    /// image returns without being sent again.
    #[test]
    fn removing_one_placement_keeps_its_data() {
        assert_eq!(remove(7), "\x1b_Ga=d,d=i,i=7,p=1,q=2\x1b\\");
    }

    #[test]
    fn freeing_names_the_image_and_drops_its_data() {
        assert_eq!(free(7), "\x1b_Ga=d,d=I,i=7,q=2\x1b\\");
    }
}
