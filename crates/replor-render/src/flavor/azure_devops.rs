//! Azure DevOps Wiki extensions.
//!
//! The source is scanned line by line and split into ordinary Markdown
//! segments and virtual extension blocks. Ordinary segments keep their original
//! byte offsets and go to `pulldown-cmark` untouched, so source mapping
//! survives. Nothing is rewritten into synthetic Markdown.

mod scanner;

use crate::diagnostic::Diagnostic;
use crate::markdown::model::*;
use crate::markdown::parser::parse_range;
use crate::source::SourceText;
use scanner::{Container, Details, Segment, scan};

pub const TOSP_MESSAGE: &str = "Child pages unavailable in single-file mode";

pub fn parse(source: SourceText) -> Document {
    let mut blocks = Vec::new();
    let mut diagnostics = Vec::new();
    assemble(
        &source,
        0,
        source.as_str().len(),
        &mut blocks,
        &mut diagnostics,
    );
    expand_toc(&mut blocks, &mut diagnostics);

    Document {
        blocks,
        source,
        diagnostics,
    }
}

/// Scan one byte range and turn it into blocks. Recursive so a `<details>` body
/// gets the same extension handling as the top level.
fn assemble(
    source: &SourceText,
    range_start: usize,
    range_end: usize,
    blocks: &mut Vec<Block>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for segment in scan(source.as_str(), range_start, range_end) {
        match segment {
            Segment::Markdown { start, end } => {
                let (mut parsed, mut found) = parse_range(source, start, end);
                post_process_blocks(&mut parsed);
                blocks.append(&mut parsed);
                diagnostics.append(&mut found);
            }
            Segment::Toc { start, end } => blocks.push(Block::Toc(TocBlock {
                entries: Vec::new(),
                range: source.range(start, end),
            })),
            Segment::Tosp { start, end } => blocks.push(Block::Placeholder(PlaceholderBlock {
                kind: "child pages".to_string(),
                message: TOSP_MESSAGE.to_string(),
                source: None,
                range: source.range(start, end),
            })),
            Segment::Container(container) => {
                blocks.push(container_block(source, &container, diagnostics));
            }
            Segment::Details(details) => {
                blocks.push(details_block(source, &details, diagnostics));
            }
        }
    }
}

fn container_block(
    source: &SourceText,
    container: &Container,
    diagnostics: &mut Vec<Diagnostic>,
) -> Block {
    let range = source.range(container.start, container.end);
    if container.unterminated {
        diagnostics.push(Diagnostic::warning(
            format!("unterminated ::: {} block", container.name),
            Some(range),
        ));
    }
    match container.name.as_str() {
        "mermaid" => Block::Diagram(DiagramBlock::new(
            "mermaid".to_string(),
            container.body.clone(),
            range,
        )),
        "video" => Block::Placeholder(PlaceholderBlock {
            kind: "video".to_string(),
            message: "Video embeds are not supported in a terminal".to_string(),
            source: Some(container.body.trim().to_string()).filter(|s| !s.is_empty()),
            range,
        }),
        "query-table" => Block::Placeholder(PlaceholderBlock {
            kind: "query-table".to_string(),
            message: match query_id(&container.body) {
                Some(id) => format!("Work item query {id} requires Azure DevOps"),
                None => "Work item query requires Azure DevOps".to_string(),
            },
            source: None,
            range,
        }),
        other => {
            diagnostics.push(Diagnostic::warning(
                format!("unsupported ::: {other} block"),
                Some(range),
            ));
            Block::Placeholder(PlaceholderBlock {
                kind: other.to_string(),
                message: format!("Unsupported ::: {other} block"),
                // Malformed and unknown containers keep their body so nothing
                // is silently dropped.
                source: Some(container.body.clone()).filter(|s| !s.trim().is_empty()),
                range,
            })
        }
    }
}

fn query_id(body: &str) -> Option<String> {
    body.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .find(|token| token.len() >= 32 || (token.contains('-') && token.len() >= 8))
        .map(str::to_string)
}

fn details_block(
    source: &SourceText,
    details: &Details,
    diagnostics: &mut Vec<Diagnostic>,
) -> Block {
    let range = source.range(details.start, details.end);
    if details.unterminated {
        diagnostics.push(Diagnostic::warning(
            "unterminated <details> block",
            Some(range),
        ));
    }
    // The body is scanned recursively so a nested `<details>` or `:::` block
    // inside it is handled rather than leaking as raw HTML.
    let mut blocks = Vec::new();
    assemble(
        source,
        details.body_start,
        details.body_end,
        &mut blocks,
        diagnostics,
    );
    let summary = if details.summary.trim().is_empty() {
        vec![Inline::Text("Details".to_string())]
    } else {
        inline_text(&details.summary)
    };
    Block::Details(DetailsBlock {
        summary,
        blocks,
        range,
    })
}

/// Fill the first `[[_TOC_]]` from the document headings and mark any later one
/// as ignored, matching Azure DevOps behaviour.
fn expand_toc(blocks: &mut [Block], diagnostics: &mut Vec<Diagnostic>) {
    let entries: Vec<TocEntry> = headings(blocks)
        .into_iter()
        .filter(|h| h.atx)
        .map(|h| TocEntry {
            level: h.level,
            text: h.plain.clone(),
            range: h.range,
        })
        .collect();

    let mut seen = false;
    fill_toc(blocks, &entries, &mut seen, diagnostics);
}

fn fill_toc(
    blocks: &mut [Block],
    entries: &[TocEntry],
    seen: &mut bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for block in blocks.iter_mut() {
        let Block::Toc(toc) = block else {
            for children in block.children_mut() {
                fill_toc(children, entries, seen, diagnostics);
            }
            continue;
        };
        if *seen {
            let range = toc.range;
            diagnostics.push(Diagnostic::warning(
                "only the first [[_TOC_]] is expanded",
                Some(range),
            ));
            *block = Block::Placeholder(PlaceholderBlock {
                kind: "toc".to_string(),
                message: "Ignored: only the first [[_TOC_]] is expanded".to_string(),
                source: None,
                range,
            });
            continue;
        }
        *seen = true;
        toc.entries = entries.to_vec();
    }
}

// ------------------------------------------------------- inline extensions

/// Rewrite plain text runs into Azure inline constructs. Only `Inline::Text`
/// is touched, so code spans and link destinations are never reinterpreted.
fn post_process_blocks(blocks: &mut [Block]) {
    for block in blocks.iter_mut() {
        match block {
            Block::Heading(h) => post_process_inlines(&mut h.content),
            Block::Paragraph(p) => post_process_inlines(&mut p.content),
            Block::Table(t) => {
                for cell in t.header.iter_mut().chain(t.rows.iter_mut().flatten()) {
                    post_process_inlines(&mut cell.content);
                    expand_cell_breaks(&mut cell.content);
                }
            }
            _ => {}
        }
        for children in block.children_mut() {
            post_process_blocks(children);
        }
    }
}

fn post_process_inlines(inlines: &mut Vec<Inline>) {
    let mut out = Vec::with_capacity(inlines.len());
    for inline in inlines.drain(..) {
        match inline {
            Inline::Text(text) => out.extend(split_text(&text)),
            Inline::Strong(mut c) => {
                post_process_inlines(&mut c);
                out.push(Inline::Strong(c));
            }
            Inline::Emphasis(mut c) => {
                post_process_inlines(&mut c);
                out.push(Inline::Emphasis(c));
            }
            Inline::Strikethrough(mut c) => {
                post_process_inlines(&mut c);
                out.push(Inline::Strikethrough(c));
            }
            Inline::Link(mut l) => {
                post_process_inlines(&mut l.content);
                out.push(Inline::Link(l));
            }
            other => out.push(other),
        }
    }
    *inlines = out;
}

/// `<br/>` inside a table cell becomes a real line break.
fn expand_cell_breaks(inlines: &mut Vec<Inline>) {
    let mut out = Vec::with_capacity(inlines.len());
    for inline in inlines.drain(..) {
        match &inline {
            Inline::RawHtml(html) if is_br(html) => out.push(Inline::HardBreak),
            _ => out.push(inline),
        }
    }
    *inlines = out;
}

fn is_br(html: &str) -> bool {
    let normalized: String = html
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '/')
        .collect();
    normalized.eq_ignore_ascii_case("<br>")
}

/// Split a text run on `#123`, `@alias` and `$math$`.
fn split_text(text: &str) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    let mut literal = String::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;

    while i < text.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        let at_start = i == 0 || is_boundary(bytes[i - 1]);

        if at_start && let Some(taken) = take_math(rest) {
            flush(&mut literal, &mut out);
            out.push(Inline::Math(taken.to_string()));
            i += taken.len();
            continue;
        }
        // An id that does not fit in u64 is not a work item reference; keeping
        // it as literal text preserves what the author wrote.
        if at_start
            && rest.starts_with('#')
            && let Some(id) = take_work_item(rest)
            && let Ok(number) = id.parse::<u64>()
        {
            flush(&mut literal, &mut out);
            out.push(Inline::WorkItem(number));
            i += id.len() + 1;
            continue;
        }
        if at_start
            && rest.starts_with('@')
            && let Some(alias) = take_mention(rest)
        {
            flush(&mut literal, &mut out);
            out.push(Inline::Mention(alias.to_string()));
            i += alias.len() + 1;
            continue;
        }

        let ch = rest.chars().next().expect("char boundary");
        literal.push(ch);
        i += ch.len_utf8();
    }
    flush(&mut literal, &mut out);
    out
}

fn flush(literal: &mut String, out: &mut Vec<Inline>) {
    if !literal.is_empty() {
        out.push(Inline::Text(std::mem::take(literal)));
    }
}

fn is_boundary(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'(' | b'[' | b'{' | b',' | b';' | b':')
}

fn take_work_item(rest: &str) -> Option<&str> {
    let digits: &str = rest[1..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or("");
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

fn take_mention(rest: &str) -> Option<&str> {
    let alias: &str = rest[1..]
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '@')))
        .next()
        .unwrap_or("");
    if alias.len() < 2 { None } else { Some(alias) }
}

/// Match `$...$` or `$$...$$`, returning the whole span including delimiters.
fn take_math(rest: &str) -> Option<&str> {
    let delimiter = if rest.starts_with("$$") { "$$" } else { "$" };
    if !rest.starts_with(delimiter) {
        return None;
    }
    let body_start = delimiter.len();
    let body = &rest[body_start..];
    // A digit right after the delimiter is far more likely to be a price than
    // the start of a formula, and prose with two prices would otherwise style
    // everything between them as math.
    if body.is_empty()
        || body.starts_with(char::is_whitespace)
        || body.starts_with(|c: char| c.is_ascii_digit())
    {
        return None;
    }
    let end = body.find(delimiter)?;
    if end == 0 {
        return None;
    }
    Some(&rest[..body_start + end + delimiter.len()])
}

/// Parse a summary string as inline Markdown without a full document pass.
fn inline_text(summary: &str) -> Vec<Inline> {
    let source = SourceText::new(summary.to_string());
    let (blocks, _) = parse_range(&source, 0, summary.len());
    let mut inlines = match blocks.into_iter().next() {
        Some(Block::Paragraph(p)) => p.content,
        Some(Block::Heading(h)) => h.content,
        _ => vec![Inline::Text(summary.to_string())],
    };
    post_process_inlines(&mut inlines);
    inlines
}
