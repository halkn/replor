use crate::flavor::{Flavor, parse};
use crate::markdown::model::*;
use crate::source::SourceText;

fn azure(text: &str) -> Document {
    parse(SourceText::new(text.to_string()), Flavor::AzureDevops)
}

fn gfm(text: &str) -> Document {
    parse(SourceText::new(text.to_string()), Flavor::Gfm)
}

fn kinds(doc: &Document) -> Vec<&'static str> {
    doc.blocks.iter().map(kind).collect()
}

fn kind(block: &Block) -> &'static str {
    match block {
        Block::Heading(_) => "heading",
        Block::Paragraph(_) => "paragraph",
        Block::List(_) => "list",
        Block::Quote(_) => "quote",
        Block::Code(_) => "code",
        Block::Table(_) => "table",
        Block::HorizontalRule(_) => "rule",
        Block::Footnote(_) => "footnote",
        Block::RawHtml(_) => "html",
        Block::Toc(_) => "toc",
        Block::Details(_) => "details",
        Block::Placeholder(_) => "placeholder",
        Block::Diagram(_) => "diagram",
    }
}

#[test]
fn toc_lists_headings_in_document_order() {
    let doc = azure("# Title\n\n[[_TOC_]]\n\n## One\n\n### Deep\n\n## Two\n");
    let Some(Block::Toc(toc)) = doc.blocks.iter().find(|b| matches!(b, Block::Toc(_))) else {
        panic!("expected a TOC, got {:?}", kinds(&doc));
    };
    let labels: Vec<&str> = toc.entries.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(labels, vec!["Title", "One", "Deep", "Two"]);
    assert_eq!(toc.entries[2].level, 3);
}

#[test]
fn toc_entries_map_back_to_their_heading() {
    let doc = azure("[[_TOC_]]\n\n# Title\n\n## Second\n");
    let Some(Block::Toc(toc)) = doc.blocks.iter().find(|b| matches!(b, Block::Toc(_))) else {
        panic!("expected a TOC");
    };
    assert_eq!(toc.entries[0].range.line_start, 3);
    assert_eq!(toc.entries[1].range.line_start, 5);
}

#[test]
fn toc_labels_drop_markdown_decoration() {
    let doc = azure("[[_TOC_]]\n\n# **Bold** and `code`\n");
    let Some(Block::Toc(toc)) = doc.blocks.iter().find(|b| matches!(b, Block::Toc(_))) else {
        panic!("expected a TOC");
    };
    assert_eq!(toc.entries[0].text, "Bold and code");
}

#[test]
fn only_the_first_toc_is_expanded() {
    let doc = azure("[[_TOC_]]\n\n# One\n\n[[_TOC_]]\n");
    let tocs = doc
        .blocks
        .iter()
        .filter(|b| matches!(b, Block::Toc(_)))
        .count();
    assert_eq!(tocs, 1);
    assert!(
        doc.blocks
            .iter()
            .any(|b| matches!(b, Block::Placeholder(p) if p.kind == "toc"))
    );
    assert_eq!(doc.diagnostics.len(), 1);
}

#[test]
fn the_toc_macro_is_case_sensitive() {
    let doc = azure("[[_toc_]]\n");
    assert_eq!(kinds(&doc), vec!["paragraph"]);
}

#[test]
fn tosp_becomes_a_placeholder() {
    let doc = azure("[[_TOSP_]]\n");
    let Block::Placeholder(p) = &doc.blocks[0] else {
        panic!("expected a placeholder");
    };
    assert_eq!(p.message, super::azure_devops::TOSP_MESSAGE);
}

#[test]
fn azure_macros_inside_a_code_fence_stay_literal() {
    let doc = azure("```\n[[_TOC_]]\n[[_TOSP_]]\n::: mermaid\ngraph LR\n:::\n```\n");
    assert_eq!(kinds(&doc), vec!["code"]);
}

#[test]
fn mermaid_containers_and_fences_both_become_diagrams() {
    let container = azure("::: mermaid\ngraph LR\n  A --> B\n:::\n");
    assert_eq!(kinds(&container), vec!["diagram"]);
    let Block::Diagram(d) = &container.blocks[0] else {
        panic!("expected a diagram");
    };
    assert!(d.source.contains("graph LR"));
    assert!(d.source.contains("A --> B"));

    let fenced = azure("```mermaid\ngraph TD\n```\n");
    assert_eq!(kinds(&fenced), vec!["diagram"]);
}

#[test]
fn fenced_mermaid_works_in_gfm_flavor_too() {
    assert_eq!(kinds(&gfm("```mermaid\ngraph LR\n```\n")), vec!["diagram"]);
}

#[test]
fn fenced_mermaid_in_a_gfm_footnote_becomes_a_diagram() {
    let doc = gfm("Text[^1]\n\n[^1]: Note\n\n    ```mermaid\n    graph LR\n    ```\n");
    let Some(Block::Footnote(footnote)) =
        doc.blocks.iter().find(|b| matches!(b, Block::Footnote(_)))
    else {
        panic!("expected a footnote, got {:?}", kinds(&doc));
    };
    assert_eq!(
        footnote.blocks.iter().map(kind).collect::<Vec<_>>(),
        vec!["paragraph", "diagram"]
    );
}

#[test]
fn video_and_query_containers_become_placeholders() {
    let doc = azure(
        "::: video\n<iframe src=\"x\"></iframe>\n:::\n\n::: query-table 6ff9c8d0-2b6c-4e0e-a4a0-1d1a5b6f0c11\n:::\n",
    );
    assert_eq!(kinds(&doc), vec!["placeholder", "placeholder"]);
    let Block::Placeholder(query) = &doc.blocks[1] else {
        panic!("expected a placeholder");
    };
    assert_eq!(query.kind, "query-table");
}

#[test]
fn an_unknown_container_keeps_its_body() {
    let doc = azure("::: unknown\nkeep me\n:::\n");
    let Block::Placeholder(p) = &doc.blocks[0] else {
        panic!("expected a placeholder");
    };
    assert!(p.source.as_deref().unwrap().contains("keep me"));
    assert_eq!(doc.diagnostics.len(), 1);
}

#[test]
fn a_closing_marker_inside_a_code_fence_does_not_close_a_container() {
    let doc = azure("::: mermaid\n```text\n:::\n```\ngraph LR\n:::\n\nAfter.\n");
    assert_eq!(kinds(&doc), vec!["diagram", "paragraph"]);
    let Block::Diagram(d) = &doc.blocks[0] else {
        panic!("expected a diagram");
    };
    assert!(d.source.contains("graph LR"));
}

#[test]
fn an_unterminated_container_stops_at_the_blank_line() {
    // Without this bound the container would swallow the rest of the document.
    let doc = azure("::: mermaid\ngraph LR\n  A --> B\n\n# After\n\nBody.\n");
    assert_eq!(kinds(&doc), vec!["diagram", "heading", "paragraph"]);
    let Block::Diagram(d) = &doc.blocks[0] else {
        panic!("expected a diagram");
    };
    assert!(d.source.contains("A --> B"));
    assert!(!d.source.contains("After"));
    assert_eq!(doc.blocks[1].range().line_start, 5);
}

#[test]
fn an_unterminated_container_is_diagnosed_and_kept() {
    let doc = azure("::: mermaid\ngraph LR\n");
    assert_eq!(kinds(&doc), vec!["diagram"]);
    assert_eq!(doc.diagnostics.len(), 1);
    let Block::Diagram(d) = &doc.blocks[0] else {
        panic!("expected a diagram");
    };
    assert!(d.source.contains("graph LR"));
}

#[test]
fn details_expands_with_its_summary_and_markdown_body() {
    let doc = azure("<details><summary>More info</summary>\n\n- one\n- two\n\n</details>\n");
    let Block::Details(d) = &doc.blocks[0] else {
        panic!("expected details, got {:?}", kinds(&doc));
    };
    assert_eq!(plain_text(&d.summary), "More info");
    assert!(d.blocks.iter().any(|b| matches!(b, Block::List(_))));
}

#[test]
fn multiline_details_opening_tags_keep_the_summary_and_body() {
    for opening in ["<details\nopen>", "<details open\n>", "<details\r\nopen>"] {
        let doc = azure(&format!(
            "{opening}<summary>More info</summary>\n\n- one\n- two\n\n</details>\n\nafter\n"
        ));
        assert_eq!(kinds(&doc), vec!["details", "paragraph"]);
        let Block::Details(details) = &doc.blocks[0] else {
            panic!("expected details");
        };
        assert_eq!(plain_text(&details.summary), "More info");
        assert_eq!(details.blocks.len(), 1);
        assert!(matches!(&details.blocks[0], Block::List(_)));
        assert!(doc.diagnostics.is_empty());
    }
}

#[test]
fn multiline_details_without_a_summary_keeps_its_body() {
    let doc = azure("<details\nopen\n>body text</details>\n");
    let Block::Details(details) = &doc.blocks[0] else {
        panic!("expected details, got {:?}", kinds(&doc));
    };
    assert_eq!(plain_text(&details.summary), "Details");
    assert_eq!(details.blocks.len(), 1);
    assert!(matches!(&details.blocks[0], Block::Paragraph(p)
        if plain_text(&p.content) == "body text"));
}

#[test]
fn nested_multiline_details_keep_the_outer_body() {
    let doc = azure(concat!(
        "<details><summary>Outer</summary>\n\n",
        "<details\nopen>\n<summary>Inner</summary>\n\ninner body\n</details>\n\n",
        "outer body\n</details>\n"
    ));
    assert_eq!(kinds(&doc), vec!["details"]);
    let Block::Details(outer) = &doc.blocks[0] else {
        panic!("expected details");
    };
    assert_eq!(plain_text(&outer.summary), "Outer");
    assert_eq!(outer.blocks.len(), 2);
    let Block::Details(inner) = &outer.blocks[0] else {
        panic!("expected nested details");
    };
    assert_eq!(plain_text(&inner.summary), "Inner");
    assert!(matches!(&outer.blocks[1], Block::Paragraph(p)
        if plain_text(&p.content) == "outer body"));
    assert!(doc.diagnostics.is_empty());
}

#[test]
fn details_without_a_summary_keeps_its_body() {
    let doc = azure("<details>\nHidden content here\n</details>\n");
    let Block::Details(d) = &doc.blocks[0] else {
        panic!("expected details, got {:?}", kinds(&doc));
    };
    assert_eq!(plain_text(&d.summary), "Details");
    assert_eq!(d.blocks.len(), 1);
    assert!(matches!(&d.blocks[0], Block::Paragraph(p)
        if plain_text(&p.content) == "Hidden content here"));
}

#[test]
fn a_one_line_details_keeps_its_body() {
    let doc = azure("<details><summary>S</summary>body text here</details>\n");
    let Block::Details(d) = &doc.blocks[0] else {
        panic!("expected details, got {:?}", kinds(&doc));
    };
    assert_eq!(plain_text(&d.summary), "S");
    assert!(matches!(&d.blocks[0], Block::Paragraph(p)
        if plain_text(&p.content) == "body text here"));
}

#[test]
fn nested_details_keeps_the_outer_summary_and_body() {
    let doc = azure(concat!(
        "<details><summary>Outer</summary>\n\nouter body\n\n",
        "<details><summary>Inner</summary>\n\ninner body\n\n</details>\n\n</details>\n"
    ));
    assert_eq!(kinds(&doc), vec!["details"]);
    let Block::Details(outer) = &doc.blocks[0] else {
        panic!("expected details");
    };
    assert_eq!(plain_text(&outer.summary), "Outer");
    assert!(matches!(&outer.blocks[0], Block::Paragraph(p)
        if plain_text(&p.content) == "outer body"));
    let Block::Details(inner) = &outer.blocks[1] else {
        panic!(
            "expected a nested details, got {:?}",
            kind(&outer.blocks[1])
        );
    };
    assert_eq!(plain_text(&inner.summary), "Inner");
    // The closing tag must not survive as literal text.
    assert!(!outer.blocks.iter().any(|b| matches!(b, Block::RawHtml(h)
        if h.text.contains("</details>"))));
}

#[test]
fn extensions_inside_a_details_body_are_handled() {
    let doc = azure("<details><summary>S</summary>\n\n::: mermaid\ngraph LR\n:::\n\n</details>\n");
    let Block::Details(d) = &doc.blocks[0] else {
        panic!("expected details");
    };
    assert!(d.blocks.iter().any(|b| matches!(b, Block::Diagram(_))));
}

#[test]
fn prices_are_not_read_as_math() {
    let doc = azure("The item costs $5 and the other $10 today.\n");
    let Block::Paragraph(p) = &doc.blocks[0] else {
        panic!("expected a paragraph");
    };
    assert!(!p.content.iter().any(|i| matches!(i, Inline::Math(_))));
    assert_eq!(
        plain_text(&p.content),
        "The item costs $5 and the other $10 today."
    );
}

#[test]
fn a_work_item_id_too_large_for_u64_stays_literal() {
    let doc = azure("See #99999999999999999999999 and #42 now.\n");
    let Block::Paragraph(p) = &doc.blocks[0] else {
        panic!("expected a paragraph");
    };
    assert!(p.content.iter().any(|i| matches!(i, Inline::WorkItem(42))));
    assert!(!p.content.iter().any(|i| matches!(i, Inline::WorkItem(0))));
    assert!(plain_text(&p.content).contains("#99999999999999999999999"));
}

#[test]
fn work_items_and_mentions_are_recognised() {
    let doc = azure("Fixed #1234 with help from @jane.doe today.\n");
    let Block::Paragraph(p) = &doc.blocks[0] else {
        panic!("expected a paragraph");
    };
    assert!(
        p.content
            .iter()
            .any(|i| matches!(i, Inline::WorkItem(1234)))
    );
    assert!(
        p.content
            .iter()
            .any(|i| matches!(i, Inline::Mention(a) if a == "jane.doe"))
    );
}

#[test]
fn a_heading_is_not_read_as_a_work_item() {
    let doc = azure("# Title\n\nSee #42.\n");
    assert_eq!(kinds(&doc), vec!["heading", "paragraph"]);
    let Block::Paragraph(p) = &doc.blocks[1] else {
        panic!("expected a paragraph");
    };
    assert!(p.content.iter().any(|i| matches!(i, Inline::WorkItem(42))));
}

#[test]
fn math_is_kept_as_source() {
    let doc = azure("Inline $E = mc^2$ and more.\n");
    let Block::Paragraph(p) = &doc.blocks[0] else {
        panic!("expected a paragraph");
    };
    assert!(
        p.content
            .iter()
            .any(|i| matches!(i, Inline::Math(m) if m.contains("E = mc^2")))
    );
}

#[test]
fn table_cell_breaks_become_hard_breaks() {
    let doc = azure("| a | b |\n|---|---|\n| one<br/>two | x |\n");
    let Block::Table(t) = &doc.blocks[0] else {
        panic!("expected a table");
    };
    assert!(
        t.rows[0][0]
            .content
            .iter()
            .any(|i| matches!(i, Inline::HardBreak))
    );
}

#[test]
fn gfm_flavor_ignores_azure_syntax() {
    let doc = gfm("[[_TOC_]]\n\n[[_TOSP_]]\n\nSee #42 and @jane.\n\n::: video\nx\n:::\n");
    assert!(!doc.blocks.iter().any(|b| matches!(b, Block::Toc(_))));
    assert!(
        !doc.blocks
            .iter()
            .any(|b| matches!(b, Block::Placeholder(_)))
    );
    let has_azure_inline = doc.blocks.iter().any(|b| match b {
        Block::Paragraph(p) => p.content.iter().any(|i| {
            matches!(
                i,
                Inline::WorkItem(_) | Inline::Mention(_) | Inline::Math(_)
            )
        }),
        _ => false,
    });
    assert!(!has_azure_inline);
}

#[test]
fn japanese_headings_appear_in_the_toc() {
    let doc = azure("[[_TOC_]]\n\n# 日本語の見出し\n\n## 二番目\n");
    let Some(Block::Toc(toc)) = doc.blocks.iter().find(|b| matches!(b, Block::Toc(_))) else {
        panic!("expected a TOC");
    };
    assert_eq!(toc.entries[0].text, "日本語の見出し");
    assert_eq!(toc.entries[1].text, "二番目");
}

#[test]
fn markdown_around_extensions_keeps_its_source_lines() {
    let doc = azure("# One\n\n::: mermaid\ngraph LR\n:::\n\nAfter the diagram.\n");
    assert_eq!(kinds(&doc), vec!["heading", "diagram", "paragraph"]);
    assert_eq!(doc.blocks[0].range().line_start, 1);
    assert_eq!(doc.blocks[1].range().line_start, 3);
    assert_eq!(doc.blocks[2].range().line_start, 7);
}

#[test]
fn alerts_are_recognised_in_both_flavors() {
    for doc in [gfm("> [!NOTE]\n> Body.\n"), azure("> [!NOTE]\n> Body.\n")] {
        let Some(Block::Quote(q)) = doc.blocks.first() else {
            panic!("expected a quote, got {:?}", kinds(&doc));
        };
        assert_eq!(q.kind, Some(AlertKind::Note));
    }
}

#[test]
fn every_alert_kind_maps_to_its_own_variant() {
    let kinds = [
        ("NOTE", AlertKind::Note),
        ("TIP", AlertKind::Tip),
        ("IMPORTANT", AlertKind::Important),
        ("WARNING", AlertKind::Warning),
        ("CAUTION", AlertKind::Caution),
    ];
    for (label, expected) in kinds {
        let doc = gfm(&format!("> [!{label}]\n> Body.\n"));
        let Some(Block::Quote(q)) = doc.blocks.first() else {
            panic!("expected a quote for [!{label}]");
        };
        assert_eq!(q.kind, Some(expected), "[!{label}]");
    }
}

/// An unknown kind is not an error: the quote renders as a quote and the marker
/// stays visible, so nothing the author wrote disappears.
#[test]
fn an_unknown_alert_kind_stays_a_plain_quote() {
    let doc = gfm("> [!FOO]\n> Body.\n");
    let Some(Block::Quote(q)) = doc.blocks.first() else {
        panic!("expected a quote, got {:?}", kinds(&doc));
    };
    assert_eq!(q.kind, None);
    assert!(plain_text_of(&q.blocks).contains("[!FOO]"));
}

fn plain_text_of(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(plain_text(&p.content)),
            _ => None,
        })
        .collect()
}
#[test]
fn long_fences_keep_short_markers_and_macros_literal() {
    let source =
        crate::source::SourceText::new("````text\n```\n[[_TOC_]]\n````\n\n# After\n".into());
    let doc = super::azure_devops::parse(source);
    assert!(matches!(&doc.blocks[0], Block::Code(c) if c.text.contains("[[_TOC_]]")));
    assert!(matches!(&doc.blocks[1], Block::Heading(h) if h.plain == "After"));
}

#[test]
fn details_closers_inside_fences_are_literal() {
    let doc = super::azure_devops::parse(crate::source::SourceText::new(
        "<details>\n<summary>Title</summary>\n\n```text\n</details>\n```\n\nbody\n</details>\n"
            .into(),
    ));
    let Block::Details(details) = &doc.blocks[0] else {
        panic!("details expected")
    };
    assert!(matches!(&details.blocks[0], Block::Code(c) if c.text.contains("</details>")));
    assert!(matches!(&details.blocks[1], Block::Paragraph(_)));
    assert_eq!(doc.blocks.len(), 1);
}

#[test]
fn nested_toc_expands_first_and_ignores_later_macros() {
    let doc = super::azure_devops::parse(crate::source::SourceText::new(
        "<details>\n<summary>Title</summary>\n\n[[_TOC_]]\n\n# Inside\n</details>\n\n[[_TOC_]]\n\nSetext\n===\n\n# Outside\n".into(),
    ));
    let Block::Details(details) = &doc.blocks[0] else {
        panic!("details expected")
    };
    let Block::Toc(toc) = &details.blocks[0] else {
        panic!("toc expected")
    };
    assert_eq!(
        toc.entries
            .iter()
            .map(|e| e.text.as_str())
            .collect::<Vec<_>>(),
        ["Inside", "Outside"]
    );
    assert!(matches!(&doc.blocks[1], Block::Placeholder(_)));
    assert_eq!(doc.diagnostics.len(), 1);
}

#[test]
fn a_details_opening_tag_that_never_closes_stays_text() {
    let doc = azure("<details\nfoo\n");
    assert!(!kinds(&doc).contains(&"details"), "{:?}", kinds(&doc));
}

#[test]
fn an_outer_details_without_a_summary_does_not_take_the_inner_one() {
    let doc = azure(concat!(
        "<details>\nOuter body\n\n",
        "<details><summary>Inner</summary>\n\ninner text\n</details>\n",
        "</details>\n"
    ));
    assert_eq!(kinds(&doc), vec!["details"]);
    let Block::Details(outer) = &doc.blocks[0] else {
        panic!("expected details");
    };
    assert_eq!(plain_text(&outer.summary), "Details");
    assert!(matches!(&outer.blocks[0], Block::Paragraph(p)
        if plain_text(&p.content) == "Outer body"));
    let Block::Details(inner) = &outer.blocks[1] else {
        panic!("expected nested details, got {:?}", outer.blocks);
    };
    assert_eq!(plain_text(&inner.summary), "Inner");
    assert_eq!(outer.blocks.len(), 2);
}

#[test]
fn toc_skips_setext_headings_that_start_with_a_hash() {
    let doc = azure("[[_TOC_]]\n\n#hashtag\n===\n\n# Real\n");
    let Some(Block::Toc(toc)) = doc.blocks.iter().find(|b| matches!(b, Block::Toc(_))) else {
        panic!("expected a TOC");
    };
    let labels: Vec<&str> = toc.entries.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(labels, vec!["Real"]);
}
