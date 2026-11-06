//! `pulldown-cmark` event stream to document IR.
//!
//! Ranges come from `into_offset_iter`: a `Start` event carries the byte range
//! of the whole element, so each frame records the range when it opens and uses
//! it again when it closes.

use pulldown_cmark::{
    Alignment as CmarkAlignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options,
    Parser, Tag, TagEnd,
};

use crate::diagnostic::Diagnostic;
use crate::markdown::model::*;
use crate::source::{SourceRange, SourceText};

pub fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        // The only thing this flag adds in 0.13 is the alert kind on a block
        // quote. An unrecognised `[!FOO]` stays literal text in a plain quote.
        | Options::ENABLE_GFM
}

pub fn parse(source: SourceText) -> Document {
    let (blocks, diagnostics) = parse_range(&source, 0, source.as_str().len());
    Document {
        blocks,
        source,
        diagnostics,
    }
}

/// Parse one slice of the document. Offsets are shifted back to absolute
/// positions so a segment parsed in isolation still maps to the real source.
pub fn parse_range(source: &SourceText, start: usize, end: usize) -> (Vec<Block>, Vec<Diagnostic>) {
    let text = source.as_str();
    let end = end.min(text.len());
    if start >= end {
        return (Vec::new(), Vec::new());
    }
    let mut builder = Builder::new(source);
    for (event, range) in Parser::new_ext(&text[start..end], options()).into_offset_iter() {
        builder.handle(event, start + range.start, start + range.end);
    }
    builder.finish()
}

enum Frame {
    Root {
        blocks: Vec<Block>,
    },
    Paragraph {
        start: usize,
        end: usize,
        /// Tight list items hold inline content without a `Paragraph` tag, so
        /// one is synthesised and closed when the container closes.
        implicit: bool,
        content: Vec<Inline>,
    },
    Heading {
        start: usize,
        end: usize,
        level: u8,
        content: Vec<Inline>,
    },
    Quote {
        start: usize,
        end: usize,
        kind: Option<AlertKind>,
        blocks: Vec<Block>,
    },
    List {
        start: usize,
        end: usize,
        ordered: Option<u64>,
        items: Vec<ListItem>,
    },
    Item {
        start: usize,
        end: usize,
        task: Option<bool>,
        blocks: Vec<Block>,
    },
    Footnote {
        start: usize,
        end: usize,
        label: String,
        blocks: Vec<Block>,
    },
    Code {
        start: usize,
        end: usize,
        language: Option<String>,
        text: String,
        /// Line of the first text event, which is where the body starts.
        body_line: Option<usize>,
    },
    HtmlBlock {
        start: usize,
        end: usize,
        text: String,
    },
    Table {
        start: usize,
        end: usize,
        alignments: Vec<Alignment>,
        header: Vec<TableCell>,
        rows: Vec<Vec<TableCell>>,
        row_ranges: Vec<SourceRange>,
        in_head: bool,
    },
    TableRow {
        start: usize,
        end: usize,
        cells: Vec<TableCell>,
    },
    TableCell {
        content: Vec<Inline>,
    },
    Emphasis {
        content: Vec<Inline>,
    },
    Strong {
        content: Vec<Inline>,
    },
    Strikethrough {
        content: Vec<Inline>,
    },
    Link {
        dest: String,
        title: Option<String>,
        content: Vec<Inline>,
    },
    Image {
        dest: String,
        alt: String,
    },
}

struct Builder<'a> {
    source: &'a SourceText,
    stack: Vec<Frame>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Builder<'a> {
    fn new(source: &'a SourceText) -> Self {
        Self {
            source,
            stack: vec![Frame::Root { blocks: Vec::new() }],
            diagnostics: Vec::new(),
        }
    }

    fn finish(mut self) -> (Vec<Block>, Vec<Diagnostic>) {
        self.close_implicit_paragraph();
        match self.stack.pop() {
            Some(Frame::Root { blocks }) => (blocks, self.diagnostics),
            _ => (Vec::new(), self.diagnostics),
        }
    }

    fn handle(&mut self, event: Event<'_>, start: usize, end: usize) {
        match event {
            Event::Start(tag) => {
                if starts_block(&tag) {
                    self.close_implicit_paragraph();
                }
                self.open(tag, start, end);
            }
            Event::End(tag) => self.close(tag),
            Event::Text(text) => self.text(&text, start, end),
            Event::Code(code) => self.inline(Inline::Code(code.into_string()), start, end),
            Event::InlineHtml(html) => self.inline(Inline::RawHtml(html.into_string()), start, end),
            Event::Html(html) => match self.stack.last_mut() {
                Some(Frame::HtmlBlock { text, .. }) => text.push_str(&html),
                _ => {
                    self.close_implicit_paragraph();
                    let range = self.source.range(start, end);
                    self.push_block(Block::RawHtml(RawHtmlBlock {
                        text: html.into_string(),
                        range,
                    }));
                }
            },
            Event::SoftBreak => self.inline(Inline::SoftBreak, start, end),
            Event::HardBreak => self.inline(Inline::HardBreak, start, end),
            Event::FootnoteReference(label) => {
                self.inline(Inline::FootnoteRef(label.into_string()), start, end)
            }
            Event::Rule => {
                self.close_implicit_paragraph();
                let range = self.source.range(start, end);
                self.push_block(Block::HorizontalRule(range));
            }
            Event::TaskListMarker(checked) => self.set_task(checked),
            _ => {}
        }
    }

    fn open(&mut self, tag: Tag<'_>, start: usize, end: usize) {
        let frame = match tag {
            Tag::Paragraph => Frame::Paragraph {
                start,
                end,
                implicit: false,
                content: Vec::new(),
            },
            Tag::Heading { level, .. } => Frame::Heading {
                start,
                end,
                level: heading_level(level),
                content: Vec::new(),
            },
            Tag::BlockQuote(kind) => Frame::Quote {
                start,
                end,
                kind: kind.map(alert_kind),
                blocks: Vec::new(),
            },
            Tag::CodeBlock(kind) => Frame::Code {
                start,
                end,
                language: match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                    CodeBlockKind::Indented => None,
                },
                text: String::new(),
                body_line: None,
            },
            Tag::HtmlBlock => Frame::HtmlBlock {
                start,
                end,
                text: String::new(),
            },
            Tag::List(ordered) => Frame::List {
                start,
                end,
                ordered,
                items: Vec::new(),
            },
            Tag::Item => Frame::Item {
                start,
                end,
                task: None,
                blocks: Vec::new(),
            },
            Tag::FootnoteDefinition(label) => Frame::Footnote {
                start,
                end,
                label: label.into_string(),
                blocks: Vec::new(),
            },
            Tag::Table(alignments) => Frame::Table {
                start,
                end,
                alignments: alignments.into_iter().map(alignment).collect(),
                header: Vec::new(),
                rows: Vec::new(),
                row_ranges: Vec::new(),
                in_head: false,
            },
            Tag::TableHead => {
                if let Some(Frame::Table { in_head, .. }) = self.stack.last_mut() {
                    *in_head = true;
                }
                Frame::TableRow {
                    start,
                    end,
                    cells: Vec::new(),
                }
            }
            Tag::TableRow => Frame::TableRow {
                start,
                end,
                cells: Vec::new(),
            },
            Tag::TableCell => Frame::TableCell {
                content: Vec::new(),
            },
            Tag::Emphasis => Frame::Emphasis {
                content: Vec::new(),
            },
            Tag::Strong => Frame::Strong {
                content: Vec::new(),
            },
            Tag::Strikethrough => Frame::Strikethrough {
                content: Vec::new(),
            },
            Tag::Link {
                dest_url, title, ..
            } => Frame::Link {
                dest: dest_url.into_string(),
                title: Some(title.into_string()).filter(|t| !t.is_empty()),
                content: Vec::new(),
            },
            Tag::Image {
                dest_url, title, ..
            } => {
                let _ = title;
                Frame::Image {
                    dest: dest_url.into_string(),
                    alt: String::new(),
                }
            }
            _ => return,
        };
        self.stack.push(frame);
    }

    fn close(&mut self, tag: TagEnd) {
        if closes_container(&tag) {
            self.close_implicit_paragraph();
        }
        let Some(frame) = self.stack.pop() else {
            return;
        };
        match frame {
            Frame::Paragraph {
                start,
                end,
                content,
                ..
            } => {
                let range = self.source.range(start, end);
                self.push_block(Block::Paragraph(ParagraphBlock { content, range }));
            }
            Frame::Heading {
                start,
                end,
                level,
                content,
            } => {
                let range = self.source.range(start, end);
                let plain = plain_text(&content);
                self.push_block(Block::Heading(HeadingBlock {
                    level,
                    atx: is_atx(&self.source.as_str()[start..end]),
                    content,
                    plain,
                    range,
                }));
            }
            Frame::Quote {
                start,
                end,
                kind,
                blocks,
            } => {
                let range = self.source.range(start, end);
                self.push_block(Block::Quote(QuoteBlock {
                    kind,
                    blocks,
                    range,
                }));
            }
            Frame::List {
                start,
                end,
                ordered,
                items,
            } => {
                let range = self.source.range(start, end);
                self.push_block(Block::List(ListBlock {
                    start: ordered,
                    items,
                    range,
                }));
            }
            Frame::Item {
                start,
                end,
                task,
                blocks,
            } => {
                let range = self.source.range(start, end);
                let item = ListItem {
                    task,
                    blocks,
                    range,
                };
                if let Some(Frame::List { items, .. }) = self.stack.last_mut() {
                    items.push(item);
                }
            }
            Frame::Footnote {
                start,
                end,
                label,
                blocks,
            } => {
                let range = self.source.range(start, end);
                self.push_block(Block::Footnote(FootnoteBlock {
                    label,
                    blocks,
                    range,
                }));
            }
            Frame::Code {
                start,
                end,
                language,
                text,
                body_line,
            } => {
                let range = self.source.range(start, end);
                self.push_block(Block::Code(CodeBlock {
                    language,
                    text,
                    range,
                    body_line: body_line.unwrap_or(range.line_start),
                }));
            }
            Frame::HtmlBlock { start, end, text } => {
                let range = self.source.range(start, end);
                self.push_block(Block::RawHtml(RawHtmlBlock { text, range }));
            }
            Frame::Table {
                start,
                end,
                alignments,
                header,
                rows,
                row_ranges,
                ..
            } => {
                let range = self.source.range(start, end);
                self.push_block(Block::Table(TableBlock {
                    alignments,
                    header,
                    rows,
                    range,
                    row_ranges,
                }));
            }
            Frame::TableRow { start, end, cells } => {
                let range = self.source.range(start, end);
                if let Some(Frame::Table {
                    header,
                    rows,
                    row_ranges,
                    in_head,
                    ..
                }) = self.stack.last_mut()
                {
                    row_ranges.push(range);
                    if *in_head {
                        *header = cells;
                        *in_head = false;
                    } else {
                        rows.push(cells);
                    }
                }
            }
            Frame::TableCell { content } => {
                if let Some(Frame::TableRow { cells, .. }) = self.stack.last_mut() {
                    cells.push(TableCell { content });
                }
            }
            Frame::Emphasis { content } => self.inline_now(Inline::Emphasis(content)),
            Frame::Strong { content } => self.inline_now(Inline::Strong(content)),
            Frame::Strikethrough { content } => self.inline_now(Inline::Strikethrough(content)),
            Frame::Link {
                dest,
                title,
                content,
            } => self.inline_now(Inline::Link(LinkInline {
                dest,
                title,
                content,
            })),
            Frame::Image { dest, alt } => self.inline_now(Inline::Image(ImageInline { dest, alt })),
            Frame::Root { blocks } => self.stack.push(Frame::Root { blocks }),
        }
    }

    fn text(&mut self, text: &str, start: usize, end: usize) {
        match self.stack.last_mut() {
            Some(Frame::Code {
                text: buf,
                body_line,
                ..
            }) => {
                buf.push_str(text);
                body_line.get_or_insert_with(|| self.source.line_of(start));
            }
            Some(Frame::HtmlBlock { text: buf, .. }) => buf.push_str(text),
            _ => self.inline(Inline::Text(text.to_string()), start, end),
        }
    }

    /// Append an inline, opening an implicit paragraph when the current
    /// container only accepts blocks.
    fn inline(&mut self, inline: Inline, start: usize, end: usize) {
        if self.needs_implicit_paragraph() {
            self.stack.push(Frame::Paragraph {
                start,
                end,
                implicit: true,
                content: Vec::new(),
            });
        }
        if let Some(Frame::Paragraph {
            end: frame_end,
            implicit: true,
            ..
        }) = self.stack.last_mut()
        {
            *frame_end = (*frame_end).max(end);
        }
        self.inline_now(inline);
    }

    fn inline_now(&mut self, inline: Inline) {
        match self.stack.last_mut() {
            Some(
                Frame::Paragraph { content, .. }
                | Frame::Heading { content, .. }
                | Frame::TableCell { content }
                | Frame::Emphasis { content }
                | Frame::Strong { content }
                | Frame::Strikethrough { content }
                | Frame::Link { content, .. },
            ) => content.push(inline),
            Some(Frame::Image { alt, .. }) => alt.push_str(&plain_text(&[inline])),
            _ => {}
        }
    }

    fn needs_implicit_paragraph(&self) -> bool {
        matches!(
            self.stack.last(),
            Some(
                Frame::Root { .. }
                    | Frame::Item { .. }
                    | Frame::Quote { .. }
                    | Frame::Footnote { .. }
            )
        )
    }

    fn close_implicit_paragraph(&mut self) {
        if let Some(Frame::Paragraph { implicit: true, .. }) = self.stack.last() {
            self.close(TagEnd::Paragraph);
        }
    }

    fn push_block(&mut self, block: Block) {
        if let Some(
            Frame::Root { blocks }
            | Frame::Quote { blocks, .. }
            | Frame::Item { blocks, .. }
            | Frame::Footnote { blocks, .. },
        ) = self.stack.last_mut()
        {
            blocks.push(block);
        }
    }

    fn set_task(&mut self, checked: bool) {
        for frame in self.stack.iter_mut().rev() {
            if let Frame::Item { task, .. } = frame {
                *task = Some(checked);
                return;
            }
        }
    }
}

fn starts_block(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::HtmlBlock
            | Tag::List(_)
            | Tag::Item
            | Tag::FootnoteDefinition(_)
            | Tag::Table(_)
    )
}

/// A setext heading's text may itself start with `#`, as in `#hashtag` over
/// `===`; only a run of `#` followed by whitespace or the line end opens an ATX
/// heading.
fn is_atx(heading: &str) -> bool {
    let rest = heading.trim_start().trim_start_matches('#');
    rest.len() < heading.trim_start().len() && rest.chars().next().is_none_or(char::is_whitespace)
}

fn closes_container(tag: &TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Item | TagEnd::BlockQuote(_) | TagEnd::FootnoteDefinition | TagEnd::List(_)
    )
}

fn alert_kind(kind: BlockQuoteKind) -> AlertKind {
    match kind {
        BlockQuoteKind::Note => AlertKind::Note,
        BlockQuoteKind::Tip => AlertKind::Tip,
        BlockQuoteKind::Important => AlertKind::Important,
        BlockQuoteKind::Warning => AlertKind::Warning,
        BlockQuoteKind::Caution => AlertKind::Caution,
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn alignment(value: CmarkAlignment) -> Alignment {
    match value {
        CmarkAlignment::None => Alignment::None,
        CmarkAlignment::Left => Alignment::Left,
        CmarkAlignment::Center => Alignment::Center,
        CmarkAlignment::Right => Alignment::Right,
    }
}
