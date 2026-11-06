//! Block IR to rendered lines.
//!
//! Container blocks lay their children out at a reduced width and then prepend
//! a prefix, so quote borders and list indentation compose naturally and
//! continuation lines align with content rather than with the marker.

use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

use crate::layout::inline::{segments, spans};
use crate::layout::table::layout_table;
use crate::layout::wrap::{display_width, wrap_spans};
use crate::layout::{LayoutOptions, RenderedDocument, RenderedLine, RenderedSpan, StyleRole};
use crate::layout::{PreparedDocument, diagram};
use crate::markdown::model::{
    Block, CodeBlock, DetailsBlock, FootnoteBlock, HeadingBlock, ListBlock, ParagraphBlock,
    PlaceholderBlock, QuoteBlock, RawHtmlBlock, TocBlock,
};
use crate::source::SourceRange;

const TAB_STOP: usize = 4;

pub(super) const BORDER: &str = "│ ";

pub fn layout_document(document: &PreparedDocument, options: LayoutOptions) -> RenderedDocument {
    let mut lines = Vec::new();
    layout_blocks(
        &document.document().blocks,
        options.width,
        0,
        document,
        &mut lines,
    );
    RenderedDocument {
        lines,
        diagnostics: document.document().diagnostics.clone(),
    }
}

/// `depth` is how many bullet lists enclose these blocks. Only `list` raises it,
/// so a list inside a quote or a `<details>` keeps counting from where it was.
fn layout_blocks(
    blocks: &[Block],
    width: usize,
    depth: usize,
    ctx: &PreparedDocument,
    out: &mut Vec<RenderedLine>,
) {
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 && needs_separator(&blocks[index - 1], block) {
            out.push(RenderedLine::blank());
        }
        layout_block(block, width, depth, ctx, out);
    }
}

/// A list that follows its item's own text belongs to that text, so no blank
/// line separates them. Everything else is separated.
fn needs_separator(previous: &Block, current: &Block) -> bool {
    !matches!((previous, current), (Block::Paragraph(_), Block::List(_)))
}

fn layout_block(
    block: &Block,
    width: usize,
    depth: usize,
    ctx: &PreparedDocument,
    out: &mut Vec<RenderedLine>,
) {
    match block {
        Block::Heading(h) => heading(h, width, ctx, out),
        Block::Paragraph(p) => paragraph(p, width, ctx, out),
        Block::List(l) => list(l, width, depth, ctx, out),
        Block::Quote(q) => quote(q, width, depth, ctx, out),
        Block::Code(c) => code(c, ctx, out),
        Block::Table(t) => out.extend(layout_table(t, width, ctx.inline(), &ctx.document().source)),
        Block::HorizontalRule(range) => out.push(line(
            vec![RenderedSpan::new("─".repeat(width), StyleRole::Muted)],
            *range,
        )),
        Block::Footnote(f) => footnote(f, width, depth, ctx, out),
        Block::RawHtml(h) => raw_html(h, ctx, out),
        Block::Toc(t) => toc(t, width, out),
        Block::Details(d) => details(d, width, depth, ctx, out),
        Block::Placeholder(p) => placeholder(p, width, out),
        Block::Diagram(d) => diagram::diagram(d, width, ctx, out),
    }
}

fn toc(t: &TocBlock, width: usize, out: &mut Vec<RenderedLine>) {
    out.push(line(
        vec![RenderedSpan::new("Contents", StyleRole::Strong)],
        t.range,
    ));
    if t.entries.is_empty() {
        out.push(line(
            vec![RenderedSpan::new("  (no headings)", StyleRole::Muted)],
            t.range,
        ));
        return;
    }
    let base = t.entries.iter().map(|e| e.level).min().unwrap_or(1);
    for entry in &t.entries {
        let indent = "  ".repeat(usize::from(entry.level.saturating_sub(base)) + 1);
        let marker = RenderedSpan::new(format!("{indent}• "), StyleRole::ListMarker);
        let available = width.saturating_sub(display_width(&marker.text)).max(1);
        let text = wrap_spans(
            &[RenderedSpan::new(
                crate::text::spaced(&entry.text),
                StyleRole::Link,
            )],
            available,
        );
        for (index, mut spans) in text.into_iter().enumerate() {
            let prefix = if index == 0 {
                marker.clone()
            } else {
                RenderedSpan::new(" ".repeat(display_width(&marker.text)), StyleRole::Normal)
            };
            let mut all = vec![prefix];
            all.append(&mut spans);
            // TOC entries map back to the heading they came from.
            out.push(line(all, entry.range));
        }
    }
}

fn details(
    d: &DetailsBlock,
    width: usize,
    depth: usize,
    ctx: &PreparedDocument,
    out: &mut Vec<RenderedLine>,
) {
    let marker = "▾ ";
    let marker_width = display_width(marker);
    let summary = spans(&d.summary, ctx.inline());
    for (index, mut line_spans) in wrap_spans(&summary, width.saturating_sub(marker_width).max(1))
        .into_iter()
        .enumerate()
    {
        let prefix = if index == 0 {
            RenderedSpan::new(marker, StyleRole::ListMarker)
        } else {
            RenderedSpan::new(" ".repeat(marker_width), StyleRole::Normal)
        };
        let mut all = vec![prefix];
        for span in &mut line_spans {
            if span.role == StyleRole::Normal {
                span.role = StyleRole::Strong;
            }
        }
        all.append(&mut line_spans);
        out.push(line(all, d.range));
    }

    let border_width = display_width(BORDER);
    let mut child = Vec::new();
    layout_blocks(
        &d.blocks,
        width.saturating_sub(border_width).max(1),
        depth,
        ctx,
        &mut child,
    );
    for mut rendered in child {
        let mut all = vec![RenderedSpan::new(BORDER, StyleRole::CodeBorder)];
        all.append(&mut rendered.spans);
        out.push(RenderedLine {
            spans: all,
            source_range: rendered.source_range.or(Some(d.range)),
            no_wrap: rendered.no_wrap,
            image: rendered.image,
        });
    }
}

fn placeholder(p: &PlaceholderBlock, width: usize, out: &mut Vec<RenderedLine>) {
    let label = format!("[{}] ", p.kind);
    let marker_width = display_width(&label);
    let message = wrap_spans(
        &[RenderedSpan::new(&p.message, StyleRole::Muted)],
        width.saturating_sub(marker_width).max(1),
    );
    for (index, mut spans) in message.into_iter().enumerate() {
        let prefix = if index == 0 {
            RenderedSpan::new(label.clone(), StyleRole::Warning)
        } else {
            RenderedSpan::new(" ".repeat(marker_width), StyleRole::Normal)
        };
        let mut all = vec![prefix];
        all.append(&mut spans);
        out.push(line(all, p.range));
    }
    // Unsupported bodies are preserved rather than dropped.
    if let Some(source) = &p.source {
        for text in source.lines() {
            out.push(RenderedLine {
                spans: vec![
                    RenderedSpan::new(BORDER, StyleRole::CodeBorder),
                    RenderedSpan::new(expand_tabs(text), StyleRole::Muted),
                ],
                source_range: Some(p.range),
                no_wrap: true,
                image: None,
            });
        }
    }
}

fn heading(h: &HeadingBlock, width: usize, ctx: &PreparedDocument, out: &mut Vec<RenderedLine>) {
    let level = h.level.min(6);
    let marker = format!("{} ", "#".repeat(level as usize));
    let marker_width = display_width(&marker);
    let content = spans(&h.content, ctx.inline());
    let wrapped = wrap_spans(&content, width.saturating_sub(marker_width).max(1));

    for (index, mut line_spans) in wrapped.into_iter().enumerate() {
        for span in &mut line_spans {
            // Heading text keeps its own inline roles only where they add
            // meaning; plain runs take the heading role.
            if span.role == StyleRole::Normal {
                span.role = StyleRole::Heading(level);
            }
        }
        let prefix = if index == 0 {
            RenderedSpan::new(marker.clone(), StyleRole::Muted)
        } else {
            RenderedSpan::new(" ".repeat(marker_width), StyleRole::Normal)
        };
        let mut all = vec![prefix];
        all.extend(line_spans);
        out.push(line(all, h.range));
    }

    if level <= 2 {
        out.push(line(
            vec![RenderedSpan::new("─".repeat(width), StyleRole::Muted)],
            h.range,
        ));
    }
}

fn paragraph(
    p: &ParagraphBlock,
    width: usize,
    ctx: &PreparedDocument,
    out: &mut Vec<RenderedLine>,
) {
    // Only a paragraph that is one image has one prepared; an image among words
    // would need a multi-row box inside a wrapped line.
    if let Some(placement) = ctx.image(p.range, width) {
        reserve(&placement, p.range, out);
        return;
    }
    for segment in segments(&p.content, ctx.inline()) {
        for line_spans in wrap_spans(&segment, width) {
            out.push(line(line_spans, p.range));
        }
    }
}

/// The image covers cells the backend paints over; the reserved lines stay
/// blank and carry the block's range so `--line` and `--watch` still find their
/// way back to the source.
pub(super) fn reserve(
    placement: &Rc<crate::image::Placement>,
    range: SourceRange,
    out: &mut Vec<RenderedLine>,
) {
    for row in 0..placement.rows {
        out.push(RenderedLine {
            spans: Vec::new(),
            source_range: Some(range),
            no_wrap: true,
            image: (row == 0).then(|| Rc::clone(placement)),
        });
    }
}

/// Bullets by nesting depth. All three are East Asian Width Ambiguous, the same
/// class as the `•` used at every depth before, so the measured width is
/// unchanged.
const BULLETS: [&str; 3] = ["•", "◦", "▪"];

fn list(
    l: &ListBlock,
    width: usize,
    depth: usize,
    ctx: &PreparedDocument,
    out: &mut Vec<RenderedLine>,
) {
    // A list is loose when some item carries more than one block of its own,
    // ignoring nested lists, which attach to the item's text.
    let loose = l.items.iter().any(|item| {
        item.blocks
            .iter()
            .filter(|b| !matches!(b, Block::List(_)))
            .count()
            > 1
    });
    for (index, item) in l.items.iter().enumerate() {
        if index > 0 && loose {
            out.push(RenderedLine::blank());
        }
        let (marker, marker_role) = match (l.start, item.task) {
            (_, Some(true)) => ("[x] ".to_string(), StyleRole::TaskChecked),
            (_, Some(false)) => ("[ ] ".to_string(), StyleRole::TaskUnchecked),
            (Some(start), None) => (
                format!("{}. ", start.saturating_add(index as u64)),
                StyleRole::ListMarker,
            ),
            (None, None) => (
                format!("{} ", BULLETS[depth % BULLETS.len()]),
                StyleRole::ListMarker,
            ),
        };
        let marker_width = display_width(&marker);

        let mut child = Vec::new();
        layout_blocks(
            &item.blocks,
            width.saturating_sub(marker_width).max(1),
            depth + 1,
            ctx,
            &mut child,
        );
        if child.is_empty() {
            child.push(RenderedLine::blank());
        }

        for (line_index, mut rendered) in child.into_iter().enumerate() {
            let prefix = if line_index == 0 {
                RenderedSpan::new(marker.clone(), marker_role)
            } else {
                RenderedSpan::new(" ".repeat(marker_width), StyleRole::Normal)
            };
            let mut all = vec![prefix];
            all.append(&mut rendered.spans);
            out.push(RenderedLine {
                spans: all,
                source_range: rendered.source_range.or(Some(item.range)),
                no_wrap: rendered.no_wrap,
                image: rendered.image,
            });
        }
    }
}

/// A quote, or a GFM alert when the quote carries a kind.
///
/// An alert reuses the chrome code blocks and diagrams already draw, so the
/// label survives `--color never`, where colour cannot tell an alert from a quote.
fn quote(
    q: &QuoteBlock,
    width: usize,
    depth: usize,
    ctx: &PreparedDocument,
    out: &mut Vec<RenderedLine>,
) {
    let role = match q.kind {
        Some(kind) => StyleRole::Alert(kind),
        None => StyleRole::Quote,
    };
    if let Some(kind) = q.kind {
        out.push(RenderedLine {
            spans: vec![RenderedSpan::new(
                format!("╭─ {}{}", ctx.inline().icons.alert(kind), kind.label()),
                role,
            )],
            source_range: Some(q.range),
            no_wrap: true,
            image: None,
        });
    }

    let border_width = display_width(BORDER);
    let mut child = Vec::new();
    layout_blocks(
        &q.blocks,
        width.saturating_sub(border_width).max(1),
        depth,
        ctx,
        &mut child,
    );
    for mut rendered in child {
        let mut all = vec![RenderedSpan::new(BORDER, role)];
        all.append(&mut rendered.spans);
        out.push(RenderedLine {
            spans: all,
            source_range: rendered.source_range.or(Some(q.range)),
            no_wrap: rendered.no_wrap,
            image: rendered.image,
        });
    }

    if q.kind.is_some() {
        out.push(RenderedLine {
            spans: vec![RenderedSpan::new("╰─", role)],
            source_range: Some(q.range),
            no_wrap: true,
            image: None,
        });
    }
}

fn code(c: &CodeBlock, ctx: &PreparedDocument, out: &mut Vec<RenderedLine>) {
    let source = &ctx.document().source;
    let header = match &c.language {
        Some(language) => format!("╭─ {}{language}", ctx.inline().icons.language(language)),
        None => "╭─".to_string(),
    };
    out.push(RenderedLine {
        spans: vec![RenderedSpan::new(header, StyleRole::CodeBorder)],
        source_range: Some(source.line_range(c.range.line_start)),
        no_wrap: true,
        image: None,
    });
    for (index, content) in ctx.code(c).iter().enumerate() {
        let mut spans = vec![RenderedSpan::new(BORDER, StyleRole::CodeBorder)];
        spans.extend(content.iter().cloned());
        out.push(RenderedLine {
            spans,
            source_range: Some(source.line_range(c.body_line + index)),
            no_wrap: true,
            image: None,
        });
    }
    out.push(RenderedLine {
        spans: vec![RenderedSpan::new("╰─", StyleRole::CodeBorder)],
        source_range: Some(source.line_range(c.range.line_end)),
        no_wrap: true,
        image: None,
    });
}

fn footnote(
    f: &FootnoteBlock,
    width: usize,
    depth: usize,
    ctx: &PreparedDocument,
    out: &mut Vec<RenderedLine>,
) {
    let marker = format!("[^{}] ", f.label);
    let marker_width = display_width(&marker);
    let mut child = Vec::new();
    layout_blocks(
        &f.blocks,
        width.saturating_sub(marker_width).max(1),
        depth,
        ctx,
        &mut child,
    );
    for (index, mut rendered) in child.into_iter().enumerate() {
        let prefix = if index == 0 {
            RenderedSpan::new(marker.clone(), StyleRole::Muted)
        } else {
            RenderedSpan::new(" ".repeat(marker_width), StyleRole::Normal)
        };
        let mut all = vec![prefix];
        all.append(&mut rendered.spans);
        out.push(RenderedLine {
            spans: all,
            source_range: rendered.source_range.or(Some(f.range)),
            no_wrap: rendered.no_wrap,
            image: rendered.image,
        });
    }
}

fn raw_html(h: &RawHtmlBlock, ctx: &PreparedDocument, out: &mut Vec<RenderedLine>) {
    let source = &ctx.document().source;
    for (index, text) in h.text.lines().enumerate() {
        let line = (h.range.line_start + index).min(h.range.line_end);
        out.push(RenderedLine {
            spans: vec![RenderedSpan::new(expand_tabs(text), StyleRole::Muted)],
            source_range: Some(source.line_range(line)),
            no_wrap: true,
            image: None,
        });
    }
}

pub(super) fn line(spans: Vec<RenderedSpan>, range: SourceRange) -> RenderedLine {
    RenderedLine {
        spans,
        source_range: Some(range),
        no_wrap: false,
        image: None,
    }
}

/// Expand tabs to fixed tab stops so code indentation survives in a terminal
/// that would otherwise apply its own tab handling.
pub(super) fn expand_tabs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut column = 0usize;
    for grapheme in text.graphemes(true) {
        if grapheme == "\t" {
            let advance = TAB_STOP - (column % TAB_STOP);
            out.extend(std::iter::repeat_n(' ', advance));
            column += advance;
        } else {
            let visible = crate::text::visible(grapheme);
            out.push_str(&visible);
            column += display_width(&visible);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flavor::Flavor;
    use crate::image::{CellSize, ImageSupport, Protocol};
    use crate::layout::icons::IconSet;
    use crate::layout::inline::InlineContext;
    use crate::markdown::model::{AlertKind, DiagramOutput, DiagramPng};
    use crate::source::SourceText;
    use std::io::Write;
    use std::path::Path;

    fn png(dir: &Path, name: &str, width: u32, height: u32) {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        std::fs::File::create(dir.join(name))
            .expect("creating the fixture")
            .write_all(&bytes)
            .expect("writing the fixture");
    }

    fn context(base_dir: &Path, images: bool) -> InlineContext {
        InlineContext {
            base_dir: Some(base_dir.to_path_buf()),
            images: images.then_some(ImageSupport {
                protocol: Protocol::Kitty,
                cell: CellSize {
                    width: 10,
                    height: 20,
                },
            }),
            ..InlineContext::default()
        }
    }

    fn render(markdown: &str, ctx: &InlineContext) -> RenderedDocument {
        let document = crate::flavor::parse(SourceText::new(markdown.to_string()), Flavor::Gfm);
        layout_document(
            &PreparedDocument::new(document, ctx.clone()),
            LayoutOptions::new(80),
        )
    }

    #[test]
    fn a_paragraph_holding_only_an_image_becomes_reserved_lines() {
        let dir = tempfile::tempdir().expect("temporary directory");
        png(dir.path(), "a.png", 200, 100);
        let rendered = render("![alt](a.png)\n", &context(dir.path(), true));

        assert_eq!(rendered.lines.len(), 5);
        let placement = rendered.lines[0]
            .image
            .as_ref()
            .expect("the first line carries the image");
        assert_eq!((placement.cols, placement.rows), (20, 5));
        // The rest of the block is reserved space, and every line points back
        // at the source so `--line` and `--watch` still work.
        assert!(rendered.lines[1..].iter().all(|l| l.image.is_none()));
        assert!(rendered.lines.iter().all(|l| l.source_range.is_some()));
        assert!(rendered.lines.iter().all(|l| l.text().is_empty()));
    }

    fn render_diagram(ctx: &InlineContext) -> RenderedDocument {
        let mut document = crate::flavor::parse(
            SourceText::new("```mermaid\ngraph LR\n  A --> B\n```\n".to_string()),
            Flavor::Gfm,
        );
        let Block::Diagram(d) = &mut document.blocks[0] else {
            panic!("expected a diagram");
        };
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&300u32.to_be_bytes());
        bytes.extend_from_slice(&60u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        d.output = DiagramOutput::Image(DiagramPng {
            bytes: bytes.into(),
            css_width: 150,
        });
        layout_document(
            &PreparedDocument::new(document, ctx.clone()),
            LayoutOptions::new(80),
        )
    }

    #[test]
    fn a_diagram_picture_becomes_reserved_lines() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let rendered = render_diagram(&context(dir.path(), true));
        assert_eq!(rendered.lines.len(), 2);
        let placement = rendered.lines[0].image.as_ref().expect("picture");
        assert_eq!((placement.cols, placement.rows), (19, 2));
        assert!(rendered.lines.iter().all(|l| l.source_range.is_some()));
    }

    /// Without a protocol there is no picture to draw, and the source is shown
    /// rather than an empty gap.
    #[test]
    fn a_diagram_picture_without_a_protocol_shows_the_source() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let rendered = render_diagram(&context(dir.path(), false));
        assert!(rendered.lines.iter().all(|l| l.image.is_none()));
        assert!(rendered.lines[0].text().contains("(source)"));
    }

    #[test]
    fn an_image_among_words_keeps_its_placeholder() {
        let dir = tempfile::tempdir().expect("temporary directory");
        png(dir.path(), "a.png", 200, 100);
        let rendered = render("see ![alt](a.png) here\n", &context(dir.path(), true));

        assert!(rendered.lines.iter().all(|l| l.image.is_none()));
        assert!(rendered.lines[0].text().contains("[image: alt]"));
    }

    /// Without a protocol the layout is byte for byte what it was before
    /// images existed, which is what the golden snapshots assert.
    #[test]
    fn a_terminal_without_images_gets_the_placeholder_layout() {
        let dir = tempfile::tempdir().expect("temporary directory");
        png(dir.path(), "a.png", 200, 100);
        let rendered = render("![alt](a.png)\n", &context(dir.path(), false));

        assert_eq!(rendered.lines.len(), 1);
        assert!(rendered.lines[0].image.is_none());
        assert!(rendered.lines[0].text().starts_with("[image: alt]"));
    }

    #[test]
    fn an_alert_is_a_labelled_box() {
        let rendered = render("> [!WARNING]\n> Careful.\n", &InlineContext::default());
        let text: Vec<String> = rendered.lines.iter().map(RenderedLine::text).collect();
        assert_eq!(text, vec!["╭─ Warning", "│ Careful.", "╰─"]);
        // Chrome and label share one role, so a backend colours the whole box
        // from the kind alone.
        assert!(
            rendered
                .lines
                .iter()
                .all(|l| l.spans[0].role == StyleRole::Alert(AlertKind::Warning))
        );
    }

    /// The header and footer exist to name the kind, so they are drawn even
    /// when the author wrote no body.
    #[test]
    fn an_empty_alert_still_shows_its_label() {
        let rendered = render("> [!NOTE]\n", &InlineContext::default());
        let text: Vec<String> = rendered.lines.iter().map(RenderedLine::text).collect();
        assert_eq!(text, vec!["╭─ Note", "╰─"]);
    }

    /// The glyph sits inside the existing box, so an alert keeps one role and
    /// one shape whichever set is in use.
    #[test]
    fn the_nerd_set_puts_a_glyph_before_the_alert_label() {
        let ctx = InlineContext {
            icons: IconSet::Nerd,
            ..InlineContext::default()
        };
        let rendered = render("> [!WARNING]\n> Careful.\n", &ctx);
        let text: Vec<String> = rendered.lines.iter().map(RenderedLine::text).collect();
        assert_eq!(
            text[0],
            format!("╭─ {}Warning", IconSet::Nerd.alert(AlertKind::Warning))
        );
        assert_eq!(
            rendered.lines[0].spans[0].role,
            StyleRole::Alert(AlertKind::Warning)
        );
    }

    #[test]
    fn the_nerd_set_puts_a_glyph_before_the_fence_language() {
        let ctx = InlineContext {
            icons: IconSet::Nerd,
            ..InlineContext::default()
        };
        let rendered = render("```rust\nfn main() {}\n```\n", &ctx);
        let text: Vec<String> = rendered.lines.iter().map(RenderedLine::text).collect();
        assert_eq!(
            text[0],
            format!("╭─ {}rust", IconSet::Nerd.language("rust"))
        );
        // A fence without a language has nothing to name, so it gains no glyph.
        let bare = render("```\nplain\n```\n", &ctx);
        assert_eq!(bare.lines[0].text(), "╭─");
    }

    /// A quote without a kind renders exactly as it did before alerts existed,
    /// which is what the golden snapshots assert.
    #[test]
    fn a_plain_quote_keeps_its_bar() {
        let rendered = render("> Quoted.\n", &InlineContext::default());
        let text: Vec<String> = rendered.lines.iter().map(RenderedLine::text).collect();
        assert_eq!(text, vec!["│ Quoted."]);
        assert_eq!(rendered.lines[0].spans[0].role, StyleRole::Quote);
    }

    #[test]
    fn bullets_change_with_nesting_depth() {
        let rendered = render("- one\n  - two\n    - three\n", &InlineContext::default());
        let text: Vec<String> = rendered.lines.iter().map(RenderedLine::text).collect();
        assert_eq!(text, vec!["• one", "  ◦ two", "    ▪ three"]);
    }

    /// Depth counts enclosing lists of either kind, so a bullet under a number
    /// is still one level in.
    #[test]
    fn an_ordered_list_raises_the_depth_of_the_bullets_inside_it() {
        let rendered = render("1. one\n   - two\n", &InlineContext::default());
        let text: Vec<String> = rendered.lines.iter().map(RenderedLine::text).collect();
        assert_eq!(text, vec!["1. one", "   ◦ two"]);
    }

    /// A quote is not a list, so it must not shift the bullet under it.
    #[test]
    fn a_quote_does_not_raise_the_bullet_depth() {
        let rendered = render("> - one\n", &InlineContext::default());
        assert_eq!(rendered.lines[0].text(), "│ • one");
    }

    /// The source line each rendered line containing `needle` maps to.
    fn source_lines_of(rendered: &RenderedDocument, needle: &str) -> Vec<(usize, usize)> {
        rendered
            .lines
            .iter()
            .filter(|line| line.text().contains(needle))
            .map(|line| {
                let range = line.source_range.expect("mapped");
                (range.line_start, range.line_end)
            })
            .collect()
    }

    #[test]
    fn every_code_line_maps_to_its_own_source_line() {
        let markdown =
            "intro\n\n```text\nfirst\nsecond\n```\n\n> ```\n> quoted\n> ```\n\n    indented\n";
        let rendered = render(markdown, &InlineContext::default());
        assert_eq!(source_lines_of(&rendered, "╭─ text"), vec![(3, 3)]);
        assert_eq!(source_lines_of(&rendered, "first"), vec![(4, 4)]);
        assert_eq!(source_lines_of(&rendered, "second"), vec![(5, 5)]);
        assert_eq!(source_lines_of(&rendered, "quoted"), vec![(9, 9)]);
        assert_eq!(source_lines_of(&rendered, "indented"), vec![(12, 12)]);
        let footers: Vec<usize> = rendered
            .lines
            .iter()
            .filter(|line| line.text().ends_with("╰─"))
            .map(|line| line.source_range.expect("mapped").line_start)
            .collect();
        assert_eq!(footers, vec![6, 10, 12]);
    }

    #[test]
    fn every_table_row_maps_to_its_own_source_line() {
        let markdown = "| head |\n|------|\n| one |\n| two |\n";
        let grid = render(markdown, &InlineContext::default());
        assert_eq!(source_lines_of(&grid, "head"), vec![(1, 1)]);
        assert_eq!(source_lines_of(&grid, "one"), vec![(3, 3)]);
        assert_eq!(source_lines_of(&grid, "two"), vec![(4, 4)]);

        // Too narrow for a grid, so each row becomes a list of label: value.
        let wide = "| aaaaaaaaaaaa | bbbbbbbbbbbb |\n|---|---|\n| cccccccccccc | dddddddddddd |\n";
        let document = crate::flavor::parse(SourceText::new(wide.to_string()), Flavor::Gfm);
        let list = layout_document(
            &PreparedDocument::new(document, InlineContext::default()),
            LayoutOptions::new(20),
        );
        assert_eq!(source_lines_of(&list, "Row 1"), vec![(3, 3)]);
        assert_eq!(source_lines_of(&list, "dddd"), vec![(3, 3)]);
    }

    #[test]
    fn every_raw_html_line_maps_to_its_own_source_line() {
        let rendered = render("<div>\n<p>a</p>\n</div>\n", &InlineContext::default());
        assert_eq!(source_lines_of(&rendered, "<p>"), vec![(2, 2)]);
        assert_eq!(source_lines_of(&rendered, "</div>"), vec![(3, 3)]);
    }

    #[test]
    fn an_image_inside_a_quote_keeps_its_border_and_its_picture() {
        let dir = tempfile::tempdir().expect("temporary directory");
        png(dir.path(), "a.png", 200, 100);
        let rendered = render("> ![alt](a.png)\n", &context(dir.path(), true));

        assert!(rendered.lines[0].image.is_some());
        // The border is what the pager measures to place the image.
        assert_eq!(rendered.lines[0].text(), "│ ");
    }
}
