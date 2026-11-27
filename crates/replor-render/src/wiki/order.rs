//! Reading `.order`: one page name to a line.

use std::collections::HashSet;

/// The names in the order `.order` gives them, each once, and how many lines
/// named a page again.
pub(super) fn parse(text: &str) -> (Vec<&str>, usize) {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    let mut duplicates = 0;
    for name in text.lines().map(str::trim).filter(|name| !name.is_empty()) {
        match seen.insert(name) {
            true => names.push(name),
            false => duplicates += 1,
        }
    }
    (names, duplicates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_trimmed_and_blank_lines_skipped() {
        let (names, duplicates) = parse("Home\r\n  Getting-Started \n\n\t\nDesign");
        assert_eq!(names, ["Home", "Getting-Started", "Design"]);
        assert_eq!(duplicates, 0);
    }

    #[test]
    fn a_leading_bom_is_not_part_of_the_first_name() {
        let (names, _) = parse("\u{FEFF}Home\nDesign\n");
        assert_eq!(names, ["Home", "Design"]);
    }

    #[test]
    fn a_name_given_again_keeps_its_first_place() {
        let (names, duplicates) = parse("Home\nDesign\nHome\nHome\n");
        assert_eq!(names, ["Home", "Design"]);
        assert_eq!(duplicates, 2);
    }

    #[test]
    fn names_are_compared_with_their_case() {
        let (names, duplicates) = parse("home\nHome\n");
        assert_eq!(names, ["home", "Home"]);
        assert_eq!(duplicates, 0);
    }

    #[test]
    fn a_line_starting_with_hash_is_a_name() {
        let (names, _) = parse("# note\nHome\n");
        assert_eq!(names, ["# note", "Home"]);
    }
}
