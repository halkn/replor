use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;

use crate::image::{ImageAsset, Placement};
use crate::markdown::model::{Block, CodeBlock, DiagramOutput, Document, Inline};
use crate::source::SourceRange;

use super::document::expand_tabs;
use super::inline::InlineContext;
use super::{RenderedSpan, StyleRole, highlight};

/// Prepared resources are keyed by the block's whole byte range: leaf blocks
/// never share one, while a start offset alone can be shared.
type Key = (usize, usize);

fn key(range: SourceRange) -> Key {
    (range.byte_start, range.byte_end)
}

pub struct PreparedDocument {
    document: Document,
    inline: InlineContext,
    code: HashMap<Key, Vec<Vec<RenderedSpan>>>,
    images: HashMap<Key, Rc<ImageAsset>>,
}

impl PreparedDocument {
    pub fn new(document: Document, inline: InlineContext) -> Self {
        let mut code = HashMap::new();
        let mut images = HashMap::new();
        prepare(&document.blocks, &inline, &mut code, &mut images);
        Self {
            document,
            inline,
            code,
            images,
        }
    }

    /// A block that preparation did not reach is highlighted here instead, so
    /// layout never depends on visiting the tree the same way.
    pub fn code(&self, block: &CodeBlock) -> Cow<'_, [Vec<RenderedSpan>]> {
        match self.code.get(&key(block.range)) {
            Some(spans) => Cow::Borrowed(spans),
            None => Cow::Owned(code_spans(block, &self.inline)),
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn inline(&self) -> &InlineContext {
        &self.inline
    }

    pub fn image(&self, range: SourceRange, width: usize) -> Option<Rc<Placement>> {
        self.images
            .get(&key(range))?
            .place(self.inline.images?, width)
    }
}

fn prepare(
    blocks: &[Block],
    inline: &InlineContext,
    code: &mut HashMap<Key, Vec<Vec<RenderedSpan>>>,
    images: &mut HashMap<Key, Rc<ImageAsset>>,
) {
    for block in blocks {
        match block {
            Block::Code(block) => {
                code.insert(key(block.range), code_spans(block, inline));
            }
            Block::Paragraph(block) if inline.images.is_some() => {
                if let [Inline::Image(image)] = block.content.as_slice()
                    && let Some(asset) = crate::image::load(
                        &image.dest,
                        inline.base_dir.as_deref(),
                        inline.content_root.as_deref(),
                    )
                {
                    images.insert(key(block.range), asset);
                }
            }
            Block::Diagram(block) if inline.images.is_some() => {
                if let DiagramOutput::Image(png) = &block.output
                    && let Some(asset) = ImageAsset::from_png(Rc::clone(&png.bytes), png.css_width)
                {
                    images.insert(key(block.range), asset);
                }
            }
            _ => {}
        }
        for children in block.children() {
            prepare(children, inline, code, images);
        }
    }
}

fn code_spans(block: &CodeBlock, inline: &InlineContext) -> Vec<Vec<RenderedSpan>> {
    let body = block
        .text
        .lines()
        .map(expand_tabs)
        .collect::<Vec<_>>()
        .join("\n");
    inline
        .highlight
        .then_some(block.language.as_deref())
        .flatten()
        .and_then(|language| highlight::highlight(language, &body))
        .unwrap_or_else(|| {
            body.lines()
                .map(|text| vec![RenderedSpan::new(text, StyleRole::Code)])
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flavor::Flavor;
    use crate::image::{CellSize, ImageSupport, Protocol};
    use crate::layout::{LayoutOptions, layout_document};

    #[test]
    fn a_code_block_preparation_missed_is_still_laid_out() {
        let prepared = PreparedDocument::new(
            crate::document::build(
                "text\n".into(),
                Flavor::Gfm,
                crate::diagram::MermaidMode::Off,
            ),
            InlineContext::default(),
        );
        let block = CodeBlock {
            language: None,
            text: "a\tb\n".into(),
            range: prepared.document().blocks[0].range(),
            body_line: 1,
        };
        let spans = prepared.code(&block);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0][0].text, "a   b");
    }

    #[test]
    fn controls_are_visible_without_changing_source_ranges_or_tab_columns() {
        let text = "```text\n\x1b\t日\n👩‍💻\tend\n```\n";
        let document =
            crate::document::build(text.into(), Flavor::Gfm, crate::diagram::MermaidMode::Off);
        let prepared = PreparedDocument::new(document, InlineContext::default());
        let rendered = layout_document(&prepared, LayoutOptions::new(80));
        assert_eq!(prepared.document().source.as_str(), text);
        assert_eq!(rendered.lines[1].text(), "│ \\x1b    日");
        assert_eq!(rendered.lines[2].text(), "│ 👩‍💻  end");
        let range = rendered.lines[1].source_range.unwrap();
        assert_eq!(&text[range.byte_start..range.byte_end], "\x1b\t日\n");
        assert_eq!((range.line_start, range.line_end), (2, 2));
    }

    #[test]
    fn relayout_reuses_image_data_and_reload_refreshes_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        let bytes = include_bytes!("../../tests/fixtures/gfm/.attachments/architecture.png");
        std::fs::write(&path, bytes).unwrap();
        let context = InlineContext {
            base_dir: Some(dir.path().into()),
            images: Some(ImageSupport {
                protocol: Protocol::Kitty,
                cell: CellSize {
                    width: 10,
                    height: 20,
                },
            }),
            ..Default::default()
        };
        let document = || {
            crate::document::build(
                "![a](a.png)\n".into(),
                Flavor::Gfm,
                crate::diagram::MermaidMode::Off,
            )
        };
        let prepared = PreparedDocument::new(document(), context.clone());
        let wide = layout_document(&prepared, LayoutOptions::new(80));
        // A resize must not reopen files; changes become visible on reload.
        std::fs::write(&path, b"invalid").unwrap();
        let narrow = layout_document(&prepared, LayoutOptions::new(20));
        let wide = wide.lines[0].image.as_ref().unwrap();
        let narrow = narrow.lines[0].image.as_ref().unwrap();
        assert_eq!(wide.id, narrow.id);
        assert!(narrow.cols <= 20);
        let reloaded = PreparedDocument::new(document(), context);
        let rendered = layout_document(&reloaded, LayoutOptions::new(80));
        assert!(rendered.lines[0].image.is_none());
        assert!(rendered.lines[0].text().contains("[image: a]"));
    }
}
