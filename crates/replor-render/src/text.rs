use std::borrow::Cow;

/// Terminal controls from a document must remain text, including inside code.
pub fn visible(text: &str) -> Cow<'_, str> {
    if !text.chars().any(char::is_control) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_control() {
            use std::fmt::Write;
            write!(out, "\\x{:02x}", ch as u32).expect("writing to a string");
        } else {
            out.push(ch);
        }
    }
    Cow::Owned(out)
}

pub fn into_visible(text: String) -> String {
    match visible(&text) {
        Cow::Borrowed(_) => text,
        Cow::Owned(visible) => visible,
    }
}

/// Prose has no columns to keep, so whitespace controls become one space
/// instead of an escape; any other control is left for `visible`.
pub fn spaced(text: &str) -> Cow<'_, str> {
    if !text.contains(['\t', '\n', '\r']) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(text.replace("\r\n", "\n").replace(['\t', '\n', '\r'], " "))
}

/// `%XX` decoded once, or `None` when the decoded bytes are not UTF-8. A `%`
/// that does not start an escape stays as it is.
pub fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |b: Option<&u8>| b.and_then(|b| char::from(*b).to_digit(16));
        match (bytes[at], hex(bytes.get(at + 1)), hex(bytes.get(at + 2))) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                at += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_are_decoded_once_and_a_stray_percent_stays() {
        assert_eq!(percent_decoded("a%3Ab%252E").as_deref(), Some("a:b%2E"));
        assert_eq!(
            percent_decoded("100%-%zz-%4").as_deref(),
            Some("100%-%zz-%4")
        );
        assert_eq!(percent_decoded("%E6%97%A5").as_deref(), Some("日"));
        assert_eq!(percent_decoded("%FF"), None);
    }

    #[test]
    fn controls_are_visible_but_unicode_text_is_preserved() {
        assert_eq!(
            visible("日\x1b[31m\x07\u{009b}\x7f"),
            "日\\x1b[31m\\x07\\x9b\\x7f"
        );
        assert!(matches!(visible("日本語 👩‍💻"), Cow::Borrowed(_)));
    }

    #[test]
    fn whitespace_controls_become_single_spaces() {
        assert_eq!(spaced("a\tb\r\nc\nd\x1b"), "a b c d\x1b");
        assert!(matches!(spaced("a b"), Cow::Borrowed(_)));
    }
}
