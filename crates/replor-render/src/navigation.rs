//! What a document can be navigated by: its headings' anchors and its links.
//!
//! Both indexes come from the IR, never from the rendered text, so a heading
//! or link is found the same way whatever the width or the theme.

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::markdown::model::{Block, Document, Inline, headings};
use crate::source::SourceRange;

/// A heading's anchor and the source line it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub id: String,
    pub range: SourceRange,
}

/// The anchors of every heading, in document order, deduplicated the way
/// GitHub does: a repeated id gets `-1`, `-2`, ...
pub fn anchors(document: &Document) -> Vec<Anchor> {
    let mut slugger = Slugger::default();
    headings(&document.blocks)
        .into_iter()
        .map(|heading| Anchor {
            id: slugger.next(&heading.plain),
            range: heading.range,
        })
        .collect()
}

/// Where a link leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A destination as written in the source.
    Dest(String),
    /// A heading in this document, from an Azure DevOps `[[_TOC_]]` entry.
    Heading(SourceRange),
}

/// Names one link of a document, on its rendered spans too. Both kinds are
/// source byte offsets: of the link itself, or of the heading a TOC entry
/// points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkKey {
    Inline(usize),
    Toc(usize),
}

/// A link and the block it sits in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub key: LinkKey,
    pub target: Target,
    /// The text of the link, for the status bar.
    pub label: String,
    /// The range the rendered lines of this link carry: the block holding it,
    /// a table row, or the heading a TOC entry points at.
    pub range: SourceRange,
    /// Set for a TOC entry, whose lines follow the TOC's own first line.
    pub toc: Option<SourceRange>,
}

/// Every link, in document order.
pub fn links(document: &Document) -> Vec<Link> {
    let mut out = Vec::new();
    collect_blocks(&document.blocks, &mut out);
    out
}

fn collect_blocks(blocks: &[Block], out: &mut Vec<Link>) {
    for block in blocks {
        match block {
            Block::Heading(b) => collect_inlines(&b.content, b.range, out),
            Block::Paragraph(b) => collect_inlines(&b.content, b.range, out),
            Block::Table(b) => {
                let rows = std::iter::once(&b.header).chain(&b.rows);
                for (row, range) in rows.zip(&b.row_ranges) {
                    for cell in row {
                        collect_inlines(&cell.content, *range, out);
                    }
                }
            }
            Block::Details(b) => collect_inlines(&b.summary, b.range, out),
            Block::Toc(b) => {
                for entry in &b.entries {
                    out.push(Link {
                        key: LinkKey::Toc(entry.range.byte_start),
                        target: Target::Heading(entry.range),
                        label: entry.text.clone(),
                        range: entry.range,
                        toc: Some(b.range),
                    });
                }
            }
            _ => {}
        }
        for children in block.children() {
            collect_blocks(children, out);
        }
    }
}

fn collect_inlines(inlines: &[Inline], range: SourceRange, out: &mut Vec<Link>) {
    for inline in inlines {
        match inline {
            Inline::Link(link) => out.push(Link {
                key: LinkKey::Inline(link.offset),
                target: Target::Dest(link.dest.clone()),
                label: plain(&link.content),
                range,
                toc: None,
            }),
            Inline::Strong(inner) | Inline::Emphasis(inner) | Inline::Strikethrough(inner) => {
                collect_inlines(inner, range, out)
            }
            _ => {}
        }
    }
}

fn plain(inlines: &[Inline]) -> String {
    crate::markdown::model::plain_text(inlines)
}

/// Heading ids as github-slugger makes them, which is how GitHub links to a
/// heading.
#[derive(Debug, Default)]
pub struct Slugger {
    used: HashSet<String>,
}

impl Slugger {
    /// The id for `text`, suffixed if an earlier heading already has it.
    pub fn next(&mut self, text: &str) -> String {
        let base = slug(text);
        let mut id = base.clone();
        let mut n = 1;
        while self.used.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        self.used.insert(id.clone());
        id
    }
}

/// Lower-cases with the full Unicode mapping, as JavaScript's `toLowerCase`
/// does, drops the characters github-slugger removes, and turns each space
/// into `-`.
pub fn slug(text: &str) -> String {
    let removed = removed();
    text.to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if is_removed(removed, c) => None,
            c => Some(c),
        })
        .collect()
}

fn is_removed(spans: &[(u32, u32)], c: char) -> bool {
    let c = u32::from(c);
    spans
        .binary_search_by(|&(lo, hi)| {
            if hi < c {
                std::cmp::Ordering::Less
            } else if lo > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// github-slugger's own definition of the characters it removes, so the
/// table is never written by hand.
const REGEX: &str = include_str!("../data/github-slugger/regex.js");

fn removed() -> &'static [(u32, u32)] {
    static TABLE: OnceLock<Vec<(u32, u32)>> = OnceLock::new();
    TABLE.get_or_init(|| parse_regex(REGEX).expect("github-slugger's regex.js should parse"))
}

/// Turns `regex.js` into sorted, merged code point ranges. The regex is
/// written in UTF-16 code units: a class of BMP characters, then pairs of high-
/// and low-surrogate classes for the other planes. Anything else is refused,
/// so a newer `regex.js` using other syntax fails the tests loudly.
fn parse_regex(source: &str) -> Result<Vec<(u32, u32)>, String> {
    let (_, body) = source
        .split_once("export const regex = /")
        .ok_or("regex not found")?;
    let body = body
        .trim()
        .strip_suffix("/g")
        .ok_or("regex end not found")?;
    let mut parser = Parser {
        s: body.as_bytes(),
        i: 0,
    };
    let mut out = Vec::new();
    loop {
        let first = parser.atom()?;
        if parser.more() && parser.peek() != b'|' {
            let second = parser.atom()?;
            for &(lo, hi) in &first {
                for high in lo..=hi {
                    for &(low_lo, low_hi) in &second {
                        out.push((pair(high, low_lo), pair(high, low_hi)));
                    }
                }
            }
        } else {
            // A lone surrogate cannot occur in a Rust string.
            for &(lo, hi) in &first {
                if lo <= hi.min(0xD7FF) {
                    out.push((lo, hi.min(0xD7FF)));
                }
                if lo.max(0xE000) <= hi {
                    out.push((lo.max(0xE000), hi));
                }
            }
        }
        if !parser.more() {
            break;
        }
        parser.expect(b'|')?;
    }
    out.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (lo, hi) in out {
        match merged.last_mut() {
            Some(last) if lo <= last.1 + 1 => last.1 = last.1.max(hi),
            _ => merged.push((lo, hi)),
        }
    }
    Ok(merged)
}

fn pair(high: u32, low: u32) -> u32 {
    0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn more(&self) -> bool {
        self.i < self.s.len()
    }

    fn peek(&self) -> u8 {
        self.s[self.i]
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.more() && self.peek() == c {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected {:?} at {}", c as char, self.i))
        }
    }

    /// One character or class, as code unit ranges.
    fn atom(&mut self) -> Result<Vec<(u32, u32)>, String> {
        if self.peek() != b'[' {
            let c = self.char()?;
            return Ok(vec![(c, c)]);
        }
        self.i += 1;
        if self.peek() == b'^' {
            return Err(format!("negated class at {}", self.i));
        }
        let mut out = Vec::new();
        while self.peek() != b']' {
            let lo = self.char()?;
            let mut hi = lo;
            if self.peek() == b'-' && self.s.get(self.i + 1) != Some(&b']') {
                self.i += 1;
                hi = self.char()?;
            }
            out.push((lo, hi));
        }
        self.i += 1;
        Ok(out)
    }

    /// One code unit: a literal, or an escape (`\uXXXX`, `\xXX`, `\0` or an
    /// escaped punctuation character).
    fn char(&mut self) -> Result<u32, String> {
        let c = self.peek();
        self.i += 1;
        if c != b'\\' {
            if c >= 0x80 || b"^$.*+?()[{}|".contains(&c) {
                return Err(format!(
                    "unsupported regex syntax {:?} at {}",
                    c as char, self.i
                ));
            }
            return Ok(u32::from(c));
        }
        let escape = self.peek();
        self.i += 1;
        match escape {
            b'u' | b'x' => {
                let n = if escape == b'u' { 4 } else { 2 };
                let digits = self
                    .s
                    .get(self.i..self.i + n)
                    .and_then(|d| std::str::from_utf8(d).ok())
                    .ok_or("truncated escape")?;
                self.i += n;
                u32::from_str_radix(digits, 16).map_err(|err| err.to_string())
            }
            b'0' => Ok(0),
            c if b"\\/^$.*+?()[]{}|-".contains(&c) => Ok(u32::from(c)),
            c => Err(format!("unknown escape \\{} at {}", c as char, self.i)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flavor::{self, Flavor};
    use crate::source::SourceText;

    #[derive(serde::Deserialize)]
    struct Fixture {
        name: String,
        input: String,
        expected: String,
    }

    /// The fixtures run through one slugger, as github-slugger's own test
    /// does, so the repeated inputs check the suffixes too.
    #[test]
    fn every_github_slugger_fixture_matches() {
        let fixtures: Vec<Fixture> =
            serde_json::from_str(include_str!("../data/github-slugger/fixtures.json")).unwrap();
        assert!(!fixtures.is_empty());
        let mut slugger = Slugger::default();
        let failures: Vec<String> = fixtures
            .iter()
            .filter_map(|f| {
                let got = slugger.next(&f.input);
                (got != f.expected).then(|| {
                    format!(
                        "{}: {:?} -> {:?}, want {:?}",
                        f.name, f.input, got, f.expected
                    )
                })
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    fn document(text: &str, flavor: Flavor) -> Document {
        flavor::parse(SourceText::new(text.to_string()), flavor)
    }

    #[test]
    fn anchors_come_from_the_headings_in_order() {
        let doc = document(
            "# S1: 普段の環境で画像を表示できるか\n\n## Intro\n\n> ## Intro\n",
            Flavor::Gfm,
        );
        let ids: Vec<_> = anchors(&doc).into_iter().map(|a| a.id).collect();
        assert_eq!(
            ids,
            ["s1-普段の環境で画像を表示できるか", "intro", "intro-1"]
        );
    }

    #[test]
    fn links_are_found_in_every_kind_of_block() {
        let doc = document(
            "See [a](a.md) and **[b](b.md#x)**.\n\n- [c](#c)\n\n| h |\n|---|\n| [d](d.md) |\n\n## [e](e.md)\n",
            Flavor::Gfm,
        );
        let dests: Vec<_> = links(&doc)
            .into_iter()
            .map(|link| match link.target {
                Target::Dest(dest) => dest,
                Target::Heading(_) => "toc".into(),
            })
            .collect();
        assert_eq!(dests, ["a.md", "b.md#x", "#c", "d.md", "e.md"]);
    }

    #[test]
    fn a_toc_entry_is_a_link_to_its_heading() {
        let doc = document("[[_TOC_]]\n\n# One\n\n## Two\n", Flavor::AzureDevops);
        let found = links(&doc);
        let anchors = anchors(&doc);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].target, Target::Heading(anchors[1].range));
        assert_eq!(found[1].label, "Two");
        assert!(found[1].toc.is_some());
    }

    #[test]
    fn a_regex_with_other_syntax_is_refused() {
        assert!(parse_regex("export const regex = /(a)/g").is_err());
        assert!(parse_regex("export const regex = /[^a]/g").is_err());
        assert_eq!(
            parse_regex("export const regex = /[a-c]|x/g").unwrap(),
            [(0x61, 0x63), (0x78, 0x78)]
        );
    }
}
