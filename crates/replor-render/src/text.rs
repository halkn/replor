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

#[cfg(test)]
mod tests {
    use super::*;

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
