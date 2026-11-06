//! Renderer-neutral document IR. Nothing here may depend on ratatui or on any
//! terminal type; the layout engine converts this into a rendered surface.

use crate::diagnostic::Diagnostic;
use crate::source::{SourceRange, SourceText};
use std::rc::Rc;

#[derive(Debug)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub source: SourceText,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(HeadingBlock),
    Paragraph(ParagraphBlock),
    List(ListBlock),
    Quote(QuoteBlock),
    Code(CodeBlock),
    Table(TableBlock),
    HorizontalRule(SourceRange),
    Footnote(FootnoteBlock),
    RawHtml(RawHtmlBlock),
    /// Azure DevOps `[[_TOC_]]`, expanded from the document's headings.
    Toc(TocBlock),
    /// `<details>` shown expanded; the pager has no folding.
    Details(DetailsBlock),
    /// Recognised but intentionally not rendered, such as `::: video`.
    Placeholder(PlaceholderBlock),
    /// A diagram source awaiting the diagram renderer.
    Diagram(DiagramBlock),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramBlock {
    /// Diagram language, currently always `mermaid`.
    pub language: String,
    pub source: String,
    pub range: SourceRange,
    pub output: DiagramOutput,
    /// Compatibility warnings shown immediately above the diagram.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DiagramOutput {
    #[default]
    Source,
    Text(Vec<String>),
    Image(DiagramPng),
    Failed {
        message: String,
        unsupported: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramPng {
    pub bytes: Rc<[u8]>,
    /// Width in CSS pixels. The picture's own resolution varies with its
    /// size, so this is what decides its size on screen.
    pub css_width: u32,
}

impl DiagramBlock {
    pub fn new(language: String, source: String, range: SourceRange) -> Self {
        Self {
            language,
            source,
            range,
            output: DiagramOutput::Source,
            warnings: Vec::new(),
        }
    }
}

#[cfg(test)]
impl DiagramBlock {
    pub fn rendered(&self) -> Option<&Vec<String>> {
        match &self.output {
            DiagramOutput::Text(lines) => Some(lines),
            _ => None,
        }
    }

    pub fn png(&self) -> Option<&DiagramPng> {
        match &self.output {
            DiagramOutput::Image(png) => Some(png),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match &self.output {
            DiagramOutput::Failed { message, .. } => Some(message),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocBlock {
    pub entries: Vec<TocEntry>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocEntry {
    pub level: u8,
    pub text: String,
    /// Range of the heading this entry was generated from.
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailsBlock {
    pub summary: Vec<Inline>,
    pub blocks: Vec<Block>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceholderBlock {
    /// Short label such as `video` or `query-table`.
    pub kind: String,
    pub message: String,
    /// Original source, preserved so nothing is silently dropped.
    pub source: Option<String>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingBlock {
    pub level: u8,
    pub atx: bool,
    pub content: Vec<Inline>,
    /// Decoration-free text, used for the table of contents.
    pub plain: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphBlock {
    pub content: Vec<Inline>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListBlock {
    /// `Some(start)` for an ordered list, `None` for a bullet list.
    pub start: Option<u64>,
    pub items: Vec<ListItem>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
    pub range: SourceRange,
}

/// GFM alert kind carried by a block quote that opens with `[!NOTE]` and
/// friends. Azure DevOps Wiki uses the same syntax, so this stays flavor-neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl AlertKind {
    pub fn label(self) -> &'static str {
        match self {
            AlertKind::Note => "Note",
            AlertKind::Tip => "Tip",
            AlertKind::Important => "Important",
            AlertKind::Warning => "Warning",
            AlertKind::Caution => "Caution",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteBlock {
    /// `Some` for a GFM alert. An alert is a quote with a label, not a block of
    /// its own, so every traversal that already walks `Block::Quote` keeps
    /// working unchanged.
    pub kind: Option<AlertKind>,
    pub blocks: Vec<Block>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub text: String,
    pub range: SourceRange,
    /// Source line of the first line of `text`. Each later line of `text` is
    /// the next source line, so a rendered code line maps back to its own line.
    pub body_line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableBlock {
    pub alignments: Vec<Alignment>,
    pub header: Vec<TableCell>,
    pub rows: Vec<Vec<TableCell>>,
    pub range: SourceRange,
    /// The header's range followed by one per row in `rows`.
    pub row_ranges: Vec<SourceRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableCell {
    pub content: Vec<Inline>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FootnoteBlock {
    pub label: String,
    pub blocks: Vec<Block>,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHtmlBlock {
    pub text: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Link(LinkInline),
    Image(ImageInline),
    FootnoteRef(String),
    SoftBreak,
    HardBreak,
    /// Inline HTML kept verbatim; never interpreted.
    RawHtml(String),
    /// Azure DevOps `#123` work item reference. No API lookup is performed.
    WorkItem(u64),
    /// Azure DevOps `@alias` mention. No identity lookup is performed.
    Mention(String),
    /// Math shown as its own source; KaTeX is never evaluated.
    Math(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInline {
    pub dest: String,
    pub title: Option<String>,
    pub content: Vec<Inline>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInline {
    pub dest: String,
    pub alt: String,
}

impl Block {
    /// The block sequences nested directly inside this block. Every traversal
    /// descends through this, so a container is never missed by one of them.
    pub fn children(&self) -> impl Iterator<Item = &[Block]> {
        let (own, items): (Option<&[Block]>, &[ListItem]) = match self {
            Block::Quote(q) => (Some(&q.blocks), &[]),
            Block::Details(d) => (Some(&d.blocks), &[]),
            Block::Footnote(f) => (Some(&f.blocks), &[]),
            Block::List(l) => (None, &l.items),
            _ => (None, &[]),
        };
        own.into_iter()
            .chain(items.iter().map(|item| item.blocks.as_slice()))
    }

    pub fn children_mut(&mut self) -> impl Iterator<Item = &mut [Block]> {
        let (own, items): (Option<&mut [Block]>, &mut [ListItem]) = match self {
            Block::Quote(q) => (Some(&mut q.blocks), &mut []),
            Block::Details(d) => (Some(&mut d.blocks), &mut []),
            Block::Footnote(f) => (Some(&mut f.blocks), &mut []),
            Block::List(l) => (None, &mut l.items),
            _ => (None, &mut []),
        };
        own.into_iter()
            .chain(items.iter_mut().map(|item| item.blocks.as_mut_slice()))
    }
}

#[cfg(test)]
impl Block {
    pub fn range(&self) -> SourceRange {
        match self {
            Block::Heading(b) => b.range,
            Block::Paragraph(b) => b.range,
            Block::List(b) => b.range,
            Block::Quote(b) => b.range,
            Block::Code(b) => b.range,
            Block::Table(b) => b.range,
            Block::HorizontalRule(r) => *r,
            Block::Footnote(b) => b.range,
            Block::RawHtml(b) => b.range,
            Block::Toc(b) => b.range,
            Block::Details(b) => b.range,
            Block::Placeholder(b) => b.range,
            Block::Diagram(b) => b.range,
        }
    }
}

/// Headings in document order, used to build a table of contents.
pub fn headings(blocks: &[Block]) -> Vec<&HeadingBlock> {
    let mut out = Vec::new();
    collect_headings(blocks, &mut out);
    out
}

fn collect_headings<'a>(blocks: &'a [Block], out: &mut Vec<&'a HeadingBlock>) {
    for block in blocks {
        if let Block::Heading(h) = block {
            out.push(h);
        }
        for children in block.children() {
            collect_headings(children, out);
        }
    }
}

/// Concatenated visible text of an inline sequence, with decoration removed.
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    push_plain_text(inlines, &mut out);
    out
}

fn push_plain_text(inlines: &[Inline], out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(t) | Inline::Code(t) => out.push_str(t),
            Inline::Strong(c) | Inline::Emphasis(c) | Inline::Strikethrough(c) => {
                push_plain_text(c, out);
            }
            Inline::Link(l) => push_plain_text(&l.content, out),
            Inline::Image(i) => out.push_str(&i.alt),
            Inline::SoftBreak | Inline::HardBreak => out.push(' '),
            Inline::WorkItem(id) => out.push_str(&format!("#{id}")),
            Inline::Mention(alias) => out.push_str(&format!("@{alias}")),
            Inline::Math(source) => out.push_str(source),
            Inline::FootnoteRef(_) | Inline::RawHtml(_) => {}
        }
    }
}
