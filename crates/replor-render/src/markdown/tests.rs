use crate::markdown::model::*;
use crate::markdown::parse;
use crate::source::SourceText;

fn doc(text: &str) -> Document {
    parse(SourceText::new(text.to_string()))
}

fn lines(block: &Block) -> (usize, usize) {
    let r = block.range();
    (r.line_start, r.line_end)
}

#[test]
fn headings_carry_level_plain_text_and_source_lines() {
    let d = doc("# One\n\n## **Two** `code`\n");
    assert_eq!(d.blocks.len(), 2);
    match &d.blocks[0] {
        Block::Heading(h) => {
            assert_eq!(h.level, 1);
            assert_eq!(h.plain, "One");
            assert_eq!((h.range.line_start, h.range.line_end), (1, 1));
        }
        other => panic!("expected heading, got {other:?}"),
    }
    match &d.blocks[1] {
        Block::Heading(h) => {
            assert_eq!(h.level, 2);
            assert_eq!(h.plain, "Two code");
            assert_eq!((h.range.line_start, h.range.line_end), (3, 3));
        }
        other => panic!("expected heading, got {other:?}"),
    }
}

#[test]
fn paragraphs_map_to_their_own_lines() {
    let d = doc("first\n\nsecond\nstill second\n");
    assert_eq!(lines(&d.blocks[0]), (1, 1));
    assert_eq!(lines(&d.blocks[1]), (3, 4));
}

#[test]
fn inline_styles_are_preserved() {
    let d = doc("a **b** *c* ~~d~~ `e` [f](g)\n");
    let Block::Paragraph(p) = &d.blocks[0] else {
        panic!("expected paragraph");
    };
    assert!(matches!(p.content[1], Inline::Strong(_)));
    assert!(matches!(p.content[3], Inline::Emphasis(_)));
    assert!(matches!(p.content[5], Inline::Strikethrough(_)));
    assert!(matches!(p.content[7], Inline::Code(_)));
    match &p.content[9] {
        Inline::Link(l) => {
            assert_eq!(l.dest, "g");
            assert_eq!(plain_text(&l.content), "f");
        }
        other => panic!("expected link, got {other:?}"),
    }
}

#[test]
fn tight_nested_lists_keep_their_structure() {
    let d = doc("- a\n  - b\n  - c\n- d\n");
    let Block::List(list) = &d.blocks[0] else {
        panic!("expected list");
    };
    assert_eq!(list.start, None);
    assert_eq!(list.items.len(), 2);
    // The first item holds its own text plus the nested list.
    assert_eq!(list.items[0].blocks.len(), 2);
    let Block::List(inner) = &list.items[0].blocks[1] else {
        panic!("expected nested list");
    };
    assert_eq!(inner.items.len(), 2);
    assert_eq!(plain_of(&inner.items[1].blocks[0]), "c");
}

#[test]
fn ordered_lists_keep_their_start_value() {
    let d = doc("3. c\n4. d\n");
    let Block::List(list) = &d.blocks[0] else {
        panic!("expected list");
    };
    assert_eq!(list.start, Some(3));
    assert_eq!(list.items.len(), 2);
}

#[test]
fn task_list_markers_are_recorded() {
    let d = doc("- [x] done\n- [ ] todo\n- plain\n");
    let Block::List(list) = &d.blocks[0] else {
        panic!("expected list");
    };
    assert_eq!(list.items[0].task, Some(true));
    assert_eq!(list.items[1].task, Some(false));
    assert_eq!(list.items[2].task, None);
    assert_eq!(plain_of(&list.items[0].blocks[0]), "done");
}

#[test]
fn tables_keep_alignment_header_and_rows() {
    let d = doc("| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |\n");
    let Block::Table(t) = &d.blocks[0] else {
        panic!("expected table");
    };
    assert_eq!(
        t.alignments,
        vec![Alignment::Left, Alignment::Center, Alignment::Right]
    );
    assert_eq!(t.header.len(), 3);
    assert_eq!(plain_text(&t.header[0].content), "a");
    assert_eq!(t.rows.len(), 1);
    assert_eq!(plain_text(&t.rows[0][2].content), "3");
    assert_eq!((t.range.line_start, t.range.line_end), (1, 3));
}

#[test]
fn footnotes_become_their_own_block() {
    let d = doc("text[^1]\n\n[^1]: the note\n");
    let Block::Paragraph(p) = &d.blocks[0] else {
        panic!("expected paragraph");
    };
    assert!(matches!(&p.content[1], Inline::FootnoteRef(l) if l == "1"));
    let Block::Footnote(f) = &d.blocks[1] else {
        panic!("expected footnote");
    };
    assert_eq!(f.label, "1");
    assert_eq!(plain_of(&f.blocks[0]), "the note");
}

#[test]
fn fenced_code_keeps_language_and_body_verbatim() {
    let d = doc("```rust\nfn main() {}\n\n    indented\n```\n");
    let Block::Code(c) = &d.blocks[0] else {
        panic!("expected code");
    };
    assert_eq!(c.language.as_deref(), Some("rust"));
    assert_eq!(c.text, "fn main() {}\n\n    indented\n");
}

#[test]
fn indented_code_has_no_language() {
    let d = doc("    plain code\n");
    let Block::Code(c) = &d.blocks[0] else {
        panic!("expected code");
    };
    assert_eq!(c.language, None);
    assert_eq!(c.text, "plain code\n");
}

#[test]
fn azure_macros_inside_a_code_fence_stay_literal() {
    let d = doc("```\n[[_TOC_]]\n::: mermaid\ngraph LR\n:::\n```\n");
    assert_eq!(d.blocks.len(), 1);
    let Block::Code(c) = &d.blocks[0] else {
        panic!("expected code");
    };
    assert!(c.text.contains("[[_TOC_]]"));
    assert!(c.text.contains("::: mermaid"));
}

#[test]
fn block_quotes_nest() {
    let d = doc("> outer\n>\n> > inner\n");
    let Block::Quote(q) = &d.blocks[0] else {
        panic!("expected quote");
    };
    assert_eq!(plain_of(&q.blocks[0]), "outer");
    let Block::Quote(inner) = &q.blocks[1] else {
        panic!("expected nested quote");
    };
    assert_eq!(plain_of(&inner.blocks[0]), "inner");
}

#[test]
fn horizontal_rules_are_blocks() {
    let d = doc("a\n\n---\n\nb\n");
    assert!(matches!(d.blocks[1], Block::HorizontalRule(_)));
    assert_eq!(lines(&d.blocks[1]), (3, 3));
}

#[test]
fn images_keep_alt_text_and_destination() {
    let d = doc("![architecture diagram](.attachments/a.png)\n");
    let Block::Paragraph(p) = &d.blocks[0] else {
        panic!("expected paragraph");
    };
    match &p.content[0] {
        Inline::Image(i) => {
            assert_eq!(i.alt, "architecture diagram");
            assert_eq!(i.dest, ".attachments/a.png");
        }
        other => panic!("expected image, got {other:?}"),
    }
}

#[test]
fn raw_html_blocks_are_kept_verbatim() {
    let d = doc("<div class=\"x\">\nbody\n</div>\n");
    let Block::RawHtml(h) = &d.blocks[0] else {
        panic!("expected raw html");
    };
    assert!(h.text.contains("<div class=\"x\">"));
    assert!(h.text.contains("body"));
}

#[test]
fn inline_html_stays_inline() {
    let d = doc("text <br/> more\n");
    let Block::Paragraph(p) = &d.blocks[0] else {
        panic!("expected paragraph");
    };
    assert!(
        p.content
            .iter()
            .any(|i| matches!(i, Inline::RawHtml(h) if h.contains("br")))
    );
}

#[test]
fn crlf_documents_map_to_the_same_lines_as_lf() {
    let lf = doc("# One\n\npara\n");
    let crlf = doc("# One\r\n\r\npara\r\n");
    assert_eq!(lines(&lf.blocks[0]), lines(&crlf.blocks[0]));
    assert_eq!(lines(&lf.blocks[1]), lines(&crlf.blocks[1]));
    assert_eq!(lines(&crlf.blocks[1]), (3, 3));
}

#[test]
fn hard_and_soft_breaks_are_distinguished() {
    let d = doc("a\nb\\\nc\n");
    let Block::Paragraph(p) = &d.blocks[0] else {
        panic!("expected paragraph");
    };
    assert!(p.content.iter().any(|i| matches!(i, Inline::SoftBreak)));
    assert!(p.content.iter().any(|i| matches!(i, Inline::HardBreak)));
}

#[test]
fn a_document_with_only_whitespace_has_no_blocks() {
    assert!(doc("\n\n").blocks.is_empty());
}

fn plain_of(block: &Block) -> String {
    match block {
        Block::Paragraph(p) => plain_text(&p.content),
        Block::Heading(h) => h.plain.clone(),
        other => panic!("expected inline-bearing block, got {other:?}"),
    }
}
