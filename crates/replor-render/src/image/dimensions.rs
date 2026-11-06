//! Pixel dimensions read straight from a file header.
//!
//! Both graphics protocols hand the original bytes to the terminal, which
//! decodes them. Only the pixel size is needed here, to work out how many cells
//! the image should occupy, so no decoder is involved.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    WebP,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
}

/// The format the bytes actually are, regardless of the file name.
pub fn format_of(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(Format::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(Format::Gif)
    } else if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
        Some(Format::WebP)
    } else {
        None
    }
}

/// Pixel size, or `None` when the header is truncated or malformed.
pub fn dimensions(bytes: &[u8]) -> Option<Pixels> {
    match format_of(bytes)? {
        Format::Png => png(bytes),
        Format::Jpeg => jpeg(bytes),
        Format::Gif => gif(bytes),
        Format::WebP => webp(bytes),
    }
}

fn png(bytes: &[u8]) -> Option<Pixels> {
    // IHDR is required to be the first chunk, so the size sits at a fixed offset.
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    sized(be32(bytes, 16)?, be32(bytes, 20)?)
}

fn gif(bytes: &[u8]) -> Option<Pixels> {
    if bytes.len() < 10 {
        return None;
    }
    sized(u32::from(le16(bytes, 6)?), u32::from(le16(bytes, 8)?))
}

/// Walks the marker segments to the frame header. The size is not at a fixed
/// offset because any number of metadata segments may precede it.
fn jpeg(bytes: &[u8]) -> Option<Pixels> {
    let mut at = 2;
    loop {
        // Skip fill bytes; a marker is `0xff` followed by a non-`0xff` code.
        while bytes.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = *bytes.get(at)?;
        at += 1;
        // Standalone markers carry no length field.
        if matches!(marker, 0x01 | 0xd0..=0xd9) {
            continue;
        }
        let length = usize::from(be16(bytes, at)?);
        // A start-of-frame marker, excluding the entropy coding tables that
        // share the same high nibble.
        if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            return sized(
                u32::from(be16(bytes, at + 5)?),
                u32::from(be16(bytes, at + 3)?),
            );
        }
        // Entropy coded data follows the scan header and is not segmented.
        if marker == 0xda {
            return None;
        }
        at = at.checked_add(length)?;
    }
}

fn webp(bytes: &[u8]) -> Option<Pixels> {
    let chunk = bytes.get(12..16)?;
    match chunk {
        b"VP8 " => {
            // Key frame header: a 3-byte tag, the sync code, then 14-bit sizes.
            if bytes.get(23..26)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            sized(
                u32::from(le16(bytes, 26)? & 0x3fff),
                u32::from(le16(bytes, 28)? & 0x3fff),
            )
        }
        b"VP8L" => {
            if bytes.get(20) != Some(&0x2f) {
                return None;
            }
            // 14 bits of width-1 followed by 14 bits of height-1, little endian.
            let bits = u32::from_le_bytes([
                *bytes.get(21)?,
                *bytes.get(22)?,
                *bytes.get(23)?,
                *bytes.get(24)?,
            ]);
            sized((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
        }
        b"VP8X" => {
            // Canvas size as two 24-bit little endian values of size-1.
            let width = le24(bytes, 24)? + 1;
            let height = le24(bytes, 27)? + 1;
            sized(width, height)
        }
        _ => None,
    }
}

/// Rejects a zero dimension, which no protocol can place.
fn sized(width: u32, height: u32) -> Option<Pixels> {
    (width > 0 && height > 0).then_some(Pixels { width, height })
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let bytes = bytes.get(at..at + 3)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0]);
        out
    }

    fn jpeg_bytes(width: u16, height: u16) -> Vec<u8> {
        let mut out = vec![0xff, 0xd8, 0xff];
        // An APP0 segment before the frame header, so the size is not found at
        // a fixed offset.
        out.extend_from_slice(&[0xe0, 0x00, 0x06, b'J', b'F', b'I', b'F']);
        out.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        out
    }

    fn gif_bytes(width: u16, height: u16) -> Vec<u8> {
        let mut out = b"GIF89a".to_vec();
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0]);
        out
    }

    fn riff(chunk: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&((12 + body.len()) as u32).to_le_bytes());
        out.extend_from_slice(b"WEBP");
        out.extend_from_slice(chunk);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn png_size_comes_from_ihdr() {
        assert_eq!(
            dimensions(&png_bytes(640, 480)),
            Some(Pixels {
                width: 640,
                height: 480
            })
        );
    }

    #[test]
    fn jpeg_size_is_found_after_metadata_segments() {
        assert_eq!(
            dimensions(&jpeg_bytes(320, 200)),
            Some(Pixels {
                width: 320,
                height: 200
            })
        );
    }

    #[test]
    fn gif_size_is_little_endian() {
        assert_eq!(
            dimensions(&gif_bytes(12, 34)),
            Some(Pixels {
                width: 12,
                height: 34
            })
        );
    }

    #[test]
    fn lossy_webp_size_comes_from_the_key_frame_header() {
        let mut body = vec![0x00, 0x00, 0x00, 0x9d, 0x01, 0x2a];
        body.extend_from_slice(&100u16.to_le_bytes());
        body.extend_from_slice(&50u16.to_le_bytes());
        assert_eq!(
            dimensions(&riff(b"VP8 ", &body)),
            Some(Pixels {
                width: 100,
                height: 50
            })
        );
    }

    #[test]
    fn lossless_webp_size_is_bit_packed() {
        // 14 bits of width-1 then 14 bits of height-1: 8x4 pixels.
        let packed: u32 = 7 | (3 << 14);
        let mut body = vec![0x2f];
        body.extend_from_slice(&packed.to_le_bytes());
        assert_eq!(
            dimensions(&riff(b"VP8L", &body)),
            Some(Pixels {
                width: 8,
                height: 4
            })
        );
    }

    #[test]
    fn extended_webp_size_comes_from_the_canvas() {
        let mut body = vec![0; 4];
        body.extend_from_slice(&[0x3f, 0x00, 0x00]);
        body.extend_from_slice(&[0x1f, 0x00, 0x00]);
        assert_eq!(
            dimensions(&riff(b"VP8X", &body)),
            Some(Pixels {
                width: 64,
                height: 32
            })
        );
    }

    #[test]
    fn the_format_is_taken_from_the_bytes() {
        assert_eq!(format_of(&png_bytes(1, 1)), Some(Format::Png));
        assert_eq!(format_of(&gif_bytes(1, 1)), Some(Format::Gif));
        assert_eq!(
            format_of(b"<svg xmlns=\"http://www.w3.org/2000/svg\">"),
            None
        );
        assert_eq!(format_of(b""), None);
    }

    #[test]
    fn a_truncated_header_is_rejected() {
        let png = png_bytes(10, 10);
        assert_eq!(dimensions(&png[..20]), None);
        assert_eq!(dimensions(&jpeg_bytes(10, 10)[..8]), None);
    }

    #[test]
    fn a_zero_dimension_is_rejected() {
        assert_eq!(dimensions(&png_bytes(0, 10)), None);
        assert_eq!(dimensions(&gif_bytes(10, 0)), None);
    }
}
