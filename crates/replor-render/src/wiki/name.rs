//! A page's file name to the name it is shown by.

/// `stem` is the file name without `.md`. Azure DevOps writes a space in a
/// page name as `-` and a special character as its URI escape, so this
/// undoes both, the escapes once.
pub fn page_name(stem: &str) -> String {
    let spaced = stem.replace('-', " ");
    let name = percent_decoded(&spaced).unwrap_or(spaced);
    // The name is drawn as a tree row and as a child page item, where a
    // control would break the line.
    crate::text::into_visible(name)
}

/// `None` when the decoded bytes are not UTF-8. A `%` that does not start an
/// escape stays as it is.
fn percent_decoded(text: &str) -> Option<String> {
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
    fn dashes_become_spaces_and_escapes_are_decoded_once() {
        assert_eq!(page_name("Getting-Started"), "Getting Started");
        assert_eq!(page_name("API-%3A-v2"), "API : v2");
        assert_eq!(page_name("Set%2Dup-Guide"), "Set-up Guide");
        assert_eq!(page_name("Q%3F-A"), "Q? A");
        assert_eq!(page_name("FAQ%3A0525"), "FAQ:0525");
        assert_eq!(page_name("%252E"), "%2E");
        assert_eq!(page_name("日本語ページ"), "日本語ページ");
    }

    #[test]
    fn a_name_that_does_not_decode_keeps_its_escapes() {
        assert_eq!(page_name("a-%FF"), "a %FF");
        assert_eq!(page_name("100%-done"), "100% done");
        assert_eq!(page_name("%zz-%4"), "%zz %4");
    }

    #[test]
    fn controls_in_a_decoded_name_are_made_visible() {
        assert_eq!(page_name("a%07b"), "a\\x07b");
        assert_eq!(page_name("a%0Ab"), "a\\x0ab");
    }
}
