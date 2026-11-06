//! Known Azure DevOps Mermaid incompatibilities.
//!
//! `merman` targets Mermaid itself, which accepts syntax that Azure DevOps
//! rejects. These rules are deliberately narrow and data driven; this is not a
//! full Azure DevOps validator.

/// A single compatibility check over Mermaid source with code fences removed.
struct Rule {
    message: &'static str,
    applies: fn(&str) -> bool,
}

const RULES: &[Rule] = &[
    Rule {
        message: "Azure DevOps requires `graph`; `flowchart` is not supported",
        applies: uses_flowchart_keyword,
    },
    Rule {
        message: "Azure DevOps does not support long arrows such as `---->`",
        applies: uses_long_arrow,
    },
    Rule {
        message: "Azure DevOps Mermaid does not support Font Awesome icons",
        applies: uses_font_awesome,
    },
    Rule {
        message: "Azure DevOps does not support most HTML tags inside Mermaid labels",
        applies: uses_html_in_label,
    },
];

/// Warnings for `source`, in rule order.
pub fn warnings(source: &str) -> Vec<String> {
    RULES
        .iter()
        .filter(|rule| (rule.applies)(source))
        .map(|rule| rule.message.to_string())
        .collect()
}

/// The first non-empty, non-directive line, which carries the diagram keyword.
fn root_line(source: &str) -> Option<&str> {
    source
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("%%"))
}

fn uses_flowchart_keyword(source: &str) -> bool {
    root_line(source).is_some_and(|line| {
        line.split_whitespace()
            .next()
            .is_some_and(|word| word == "flowchart")
    })
}

fn uses_long_arrow(source: &str) -> bool {
    // `-->` is fine; four or more dashes before the head is not.
    source.contains("---->") || source.contains("====>") || source.contains("----|")
}

fn uses_font_awesome(source: &str) -> bool {
    source.contains("fa:fa-") || source.contains("fab:fa-") || source.contains("fas:fa-")
}

fn uses_html_in_label(source: &str) -> bool {
    const TAGS: &[&str] = &[
        "<b>", "<i>", "<u>", "<em>", "<strong>", "<span", "<font", "<img",
    ];
    let lowered = source.to_lowercase();
    TAGS.iter().any(|tag| lowered.contains(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_diagrams_produce_no_warnings() {
        assert!(warnings("graph LR\n  A --> B\n").is_empty());
    }

    #[test]
    fn flowchart_is_flagged() {
        let found = warnings("flowchart LR\n  A --> B\n");
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("graph"));
    }

    #[test]
    fn flowchart_only_counts_as_the_root_keyword() {
        // A node label mentioning the word must not trigger the rule.
        assert!(warnings("graph LR\n  A[flowchart of steps] --> B\n").is_empty());
    }

    #[test]
    fn a_leading_directive_does_not_hide_the_root_keyword() {
        assert_eq!(
            warnings("%%{init: {}}%%\nflowchart LR\n  A --> B\n").len(),
            1
        );
    }

    #[test]
    fn long_arrows_are_flagged_but_normal_ones_are_not() {
        assert!(
            !warnings("graph LR\n  A --> B\n")
                .iter()
                .any(|w| w.contains("long arrows"))
        );
        assert!(
            warnings("graph LR\n  A ----> B\n")
                .iter()
                .any(|w| w.contains("long arrows"))
        );
    }

    #[test]
    fn font_awesome_is_flagged() {
        assert!(
            warnings("graph LR\n  A[fa:fa-check done] --> B\n")
                .iter()
                .any(|w| w.contains("Font Awesome"))
        );
    }

    #[test]
    fn html_in_labels_is_flagged() {
        assert!(
            warnings("graph LR\n  A[<b>bold</b>] --> B\n")
                .iter()
                .any(|w| w.contains("HTML"))
        );
    }

    #[test]
    fn several_rules_can_fire_at_once() {
        assert_eq!(warnings("flowchart LR\n  A[<b>x</b>] ----> B\n").len(), 3);
    }

    #[test]
    fn japanese_labels_are_not_flagged() {
        assert!(warnings("graph LR\n  A[日本語のラベル] --> B[次の工程]\n").is_empty());
    }
}
