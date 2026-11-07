//! Code block highlighting.
//!
//! `syntect` is used as a parser only: its scopes are mapped onto `mdvu`'s own
//! semantic roles, and the theme keeps deciding the colours. Nothing here emits
//! a colour, so the 16-colour palette, both themes and the plain backend are
//! unaffected.
//!
//! This is the only module that mentions `syntect`, in the same way `merman` is
//! confined to `diagram::mermaid`.

use std::sync::OnceLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

use crate::layout::{RenderedSpan, StyleRole, SyntaxKind};

/// Scopes that describe a delimiter rather than a token: the quote around a
/// string, the `//` starting a comment. Skipping them lets the construct they
/// belong to colour its own punctuation, which is what a reader expects.
const TRANSPARENT: &[&str] = &["punctuation.definition"];

/// Scope prefixes, most specific first. The first prefix that matches a scope
/// decides the role.
const RULES: &[(&str, SyntaxKind)] = &[
    ("comment", SyntaxKind::Comment),
    ("string", SyntaxKind::String),
    ("constant.numeric", SyntaxKind::Number),
    ("entity.name.function", SyntaxKind::Function),
    ("variable.function", SyntaxKind::Function),
    ("support.function", SyntaxKind::Function),
    ("entity.name.type", SyntaxKind::Type),
    ("entity.name.class", SyntaxKind::Type),
    ("entity.name.struct", SyntaxKind::Type),
    ("support.type", SyntaxKind::Type),
    ("support.class", SyntaxKind::Type),
    ("keyword", SyntaxKind::Keyword),
    ("storage", SyntaxKind::Keyword),
    ("constant", SyntaxKind::Keyword),
    ("punctuation", SyntaxKind::Punctuation),
];

fn syntax_set() -> &'static SyntaxSet {
    // Loading the default definitions costs tens of milliseconds, so a document
    // without code blocks never pays for it.
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_nonewlines)
}

fn matchers() -> &'static [(Scope, SyntaxKind)] {
    static MATCHERS: OnceLock<Vec<(Scope, SyntaxKind)>> = OnceLock::new();
    MATCHERS.get_or_init(|| {
        RULES
            .iter()
            .filter_map(|(prefix, kind)| Scope::new(prefix).ok().map(|scope| (scope, *kind)))
            .collect()
    })
}

fn transparent() -> &'static [Scope] {
    static SCOPES: OnceLock<Vec<Scope>> = OnceLock::new();
    SCOPES.get_or_init(|| {
        TRANSPARENT
            .iter()
            .filter_map(|s| Scope::new(s).ok())
            .collect()
    })
}

/// Split each line of `text` into styled spans.
///
/// Returns `None` when the language is unknown, so the caller can fall back to
/// a single uniform `Code` span. Lines are returned in order and their
/// concatenated text is always identical to the input.
pub fn highlight(language: &str, text: &str) -> Option<Vec<Vec<RenderedSpan>>> {
    let set = syntax_set();
    let syntax = set
        .find_syntax_by_token(language)
        .or_else(|| set.find_syntax_by_extension(language))?;

    let mut parse = ParseState::new(syntax);
    // The stack carries over between lines so a block comment or a multi-line
    // string keeps its role.
    let mut stack = ScopeStack::new();
    let mut out = Vec::new();

    for line in text.lines() {
        // A failed line is not worth failing the document over: it is emitted
        // unstyled and parsing continues with the next one.
        let Ok(ops) = parse.parse_line(line, set) else {
            out.push(vec![RenderedSpan::new(line, StyleRole::Normal)]);
            continue;
        };
        let mut spans: Vec<RenderedSpan> = Vec::new();
        let mut last = 0usize;
        for (offset, op) in ops {
            let offset = offset.min(line.len());
            if offset > last {
                push(&mut spans, &line[last..offset], kind(&stack));
                last = offset;
            }
            if stack.apply(&op).is_err() {
                break;
            }
        }
        if last < line.len() {
            push(&mut spans, &line[last..], kind(&stack));
        }
        out.push(spans);
    }
    Some(out)
}

fn push(spans: &mut Vec<RenderedSpan>, text: &str, kind: Option<SyntaxKind>) {
    if text.is_empty() {
        return;
    }
    // Text the grammar did not classify is left unstyled. Keeping the uniform
    // `Code` colour here would paint every identifier, drowning out the tokens
    // that were classified.
    let role = match kind {
        Some(kind) => StyleRole::Syntax(kind),
        None => StyleRole::Normal,
    };
    match spans.last_mut() {
        Some(last) if last.role == role => last.text.push_str(text),
        _ => spans.push(RenderedSpan::new(text, role)),
    }
}

/// The role for the innermost scope that any rule matches.
fn kind(stack: &ScopeStack) -> Option<SyntaxKind> {
    stack
        .scopes
        .iter()
        .rev()
        .filter(|scope| !transparent().iter().any(|t| t.is_prefix_of(**scope)))
        .find_map(|scope| {
            matchers()
                .iter()
                .find(|(prefix, _)| prefix.is_prefix_of(*scope))
                .map(|(_, kind)| *kind)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(language: &str, text: &str) -> Vec<Vec<(String, StyleRole)>> {
        highlight(language, text)
            .expect("known language")
            .into_iter()
            .map(|line| {
                line.into_iter()
                    .map(|span| (span.text, span.role))
                    .collect()
            })
            .collect()
    }

    fn role_of(lines: &[Vec<(String, StyleRole)>], needle: &str) -> StyleRole {
        lines
            .iter()
            .flatten()
            .find(|(text, _)| text.contains(needle))
            .unwrap_or_else(|| panic!("no span containing {needle:?} in {lines:?}"))
            .1
    }

    #[test]
    fn an_unknown_language_is_not_highlighted() {
        assert!(highlight("no-such-language", "x = 1\n").is_none());
    }

    #[test]
    fn rust_keywords_strings_numbers_and_comments_get_distinct_roles() {
        let lines = roles(
            "rust",
            "// note\nfn main() {\n    let x = 42;\n    let s = \"hi\";\n}\n",
        );
        assert_eq!(
            role_of(&lines, "note"),
            StyleRole::Syntax(SyntaxKind::Comment)
        );
        assert_eq!(
            role_of(&lines, "fn"),
            StyleRole::Syntax(SyntaxKind::Keyword)
        );
        assert_eq!(role_of(&lines, "42"), StyleRole::Syntax(SyntaxKind::Number));
        assert_eq!(role_of(&lines, "hi"), StyleRole::Syntax(SyntaxKind::String));
    }

    #[test]
    fn text_is_preserved_exactly() {
        let source = "fn main() {\n    let x = 42; // ok\n}\n";
        let joined: String = highlight("rust", source)
            .expect("known language")
            .iter()
            .map(|line| line.iter().map(|s| s.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(joined, source.trim_end_matches('\n'));
    }

    #[test]
    fn a_block_comment_keeps_its_role_across_lines() {
        let lines = roles("rust", "/* first\n   second */\nfn x() {}\n");
        assert_eq!(lines[1][0].1, StyleRole::Syntax(SyntaxKind::Comment));
    }

    #[test]
    fn japanese_content_is_not_corrupted() {
        let lines = roles("rust", "// 日本語のコメント\nlet s = \"あいう\";\n");
        assert_eq!(
            role_of(&lines, "日本語"),
            StyleRole::Syntax(SyntaxKind::Comment)
        );
        assert_eq!(
            role_of(&lines, "あいう"),
            StyleRole::Syntax(SyntaxKind::String)
        );
    }

    #[test]
    fn common_languages_resolve() {
        for language in ["rust", "json", "python", "bash", "sh", "yaml", "sql", "c"] {
            assert!(
                highlight(language, "x\n").is_some(),
                "{language} should be known"
            );
        }
    }

    #[test]
    fn a_delimiter_takes_the_role_of_what_it_delimits() {
        // `//` belongs to the comment and the quotes belong to the string,
        // rather than both showing up as bare punctuation.
        let lines = roles("rust", "// note\nlet s = \"hi\";\n");
        assert_eq!(lines[0][0].1, StyleRole::Syntax(SyntaxKind::Comment));
        assert_eq!(
            role_of(&lines, "\"hi\""),
            StyleRole::Syntax(SyntaxKind::String)
        );
    }
}
