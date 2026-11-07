use crate::diagnostic::Severity;
use crate::diagram::MermaidMode;
use crate::diagram::resolve;
use crate::flavor;
use crate::flavor::Flavor;
use crate::markdown::model::{Block, DiagramBlock, Document};
use crate::source::SourceText;

fn document(text: &str, mode: MermaidMode, flavor_option: Flavor) -> Document {
    let mut doc = flavor::parse(SourceText::new(text.to_string()), flavor_option);
    resolve(&mut doc, mode, flavor_option);
    doc
}

fn only_diagram(doc: &Document) -> &DiagramBlock {
    doc.blocks
        .iter()
        .find_map(|b| match b {
            Block::Diagram(d) => Some(d),
            _ => None,
        })
        .expect("expected a diagram block")
}

const GRAPH: &str = "::: mermaid\ngraph LR\n  A[Start] --> B[Done]\n:::\n";

#[test]
fn unicode_is_the_default_rendering() {
    let doc = document(GRAPH, MermaidMode::Unicode, Flavor::AzureDevops);
    let d = only_diagram(&doc);
    let text = d.rendered().expect("should render").join("\n");
    assert!(text.contains("Start"), "{text}");
    assert!(!text.is_ascii(), "expected Unicode drawing: {text}");
    assert!(d.error().is_none());
}

#[test]
fn ascii_mode_renders_without_wide_characters() {
    let doc = document(GRAPH, MermaidMode::Ascii, Flavor::AzureDevops);
    let text = only_diagram(&doc)
        .rendered()
        .expect("should render")
        .join("\n");
    assert!(text.is_ascii(), "{text}");
}

#[test]
fn source_mode_never_renders() {
    let d = document(GRAPH, MermaidMode::Source, Flavor::AzureDevops);
    let diagram = only_diagram(&d);
    assert!(diagram.rendered().is_none());
    assert!(diagram.source.contains("graph LR"));
}

#[test]
fn off_mode_never_renders() {
    let d = document(GRAPH, MermaidMode::Off, Flavor::AzureDevops);
    assert!(only_diagram(&d).rendered().is_none());
}

#[test]
fn fenced_and_container_syntax_render_the_same() {
    let container = document(GRAPH, MermaidMode::Unicode, Flavor::AzureDevops);
    let fenced = document(
        "```mermaid\ngraph LR\n  A[Start] --> B[Done]\n```\n",
        MermaidMode::Unicode,
        Flavor::AzureDevops,
    );
    assert_eq!(
        only_diagram(&container).rendered(),
        only_diagram(&fenced).rendered()
    );
}

#[test]
fn japanese_labels_are_preserved() {
    let doc = document(
        "::: mermaid\ngraph LR\n  A[開始] --> B[完了]\n:::\n",
        MermaidMode::Unicode,
        Flavor::AzureDevops,
    );
    let text = only_diagram(&doc)
        .rendered()
        .expect("should render")
        .join("\n");
    assert!(text.contains("開始"), "{text}");
    assert!(text.contains("完了"), "{text}");
}

#[test]
fn invalid_syntax_falls_back_to_source_without_failing_the_document() {
    let doc = document(
        "# Before\n\n::: mermaid\ngraph LR\n  A -->\n  --> \n:::\n\nAfter.\n",
        MermaidMode::Unicode,
        Flavor::AzureDevops,
    );
    let d = only_diagram(&doc);
    assert!(d.rendered().is_none());
    assert!(d.error().is_some());
    // The body survives and the rest of the document still renders.
    assert!(d.source.contains("graph LR"));
    assert!(doc.blocks.iter().any(|b| matches!(b, Block::Heading(_))));
    assert!(
        doc.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    );
}

#[test]
fn an_unsupported_diagram_family_falls_back_to_source() {
    let doc = document(
        "::: mermaid\nstateDiagram-v2\n  [*] --> Still\n:::\n",
        MermaidMode::Unicode,
        Flavor::AzureDevops,
    );
    let d = only_diagram(&doc);
    assert!(d.error().is_some(), "expected a fallback");
    assert!(d.source.contains("stateDiagram-v2"));
}

#[test]
fn azure_flavor_warns_about_flowchart_but_graph_is_clean() {
    let flowchart = document(
        "::: mermaid\nflowchart LR\n  A --> B\n:::\n",
        MermaidMode::Unicode,
        Flavor::AzureDevops,
    );
    assert_eq!(only_diagram(&flowchart).warnings.len(), 1);
    assert!(
        flowchart
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Warning)
    );

    let graph = document(GRAPH, MermaidMode::Unicode, Flavor::AzureDevops);
    assert!(only_diagram(&graph).warnings.is_empty());
}

#[test]
fn a_flagged_diagram_still_renders() {
    let doc = document(
        "::: mermaid\nflowchart LR\n  A --> B\n:::\n",
        MermaidMode::Unicode,
        Flavor::AzureDevops,
    );
    let d = only_diagram(&doc);
    assert!(!d.warnings.is_empty());
    assert!(d.rendered().is_some(), "warnings must not block rendering");
}

#[test]
fn gfm_flavor_emits_no_azure_warnings() {
    let doc = document(
        "```mermaid\nflowchart LR\n  A ----> B\n```\n",
        MermaidMode::Unicode,
        Flavor::Gfm,
    );
    assert!(only_diagram(&doc).warnings.is_empty());
}

#[test]
fn diagrams_inside_containers_are_resolved() {
    let doc = document(
        "> quoted\n>\n> ```mermaid\n> graph LR\n>   A --> B\n> ```\n",
        MermaidMode::Unicode,
        Flavor::AzureDevops,
    );
    let Block::Quote(q) = &doc.blocks[0] else {
        panic!("expected a quote");
    };
    let has_rendered = q
        .blocks
        .iter()
        .any(|b| matches!(b, Block::Diagram(d) if d.rendered().is_some()));
    assert!(has_rendered);
}

#[test]
fn image_mode_draws_a_png_instead_of_text() {
    let doc = document(GRAPH, MermaidMode::Image, Flavor::AzureDevops);
    let d = only_diagram(&doc);
    let png = d.png().expect("should rasterise");
    assert!(png.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(png.css_width > 0);
    assert!(d.rendered().is_none());
    assert!(d.error().is_none());
}

#[test]
fn a_diagram_that_cannot_be_a_picture_reports_like_text() {
    let broken = "::: mermaid\ngraph LR\n  A -->\n  -->\n:::\n";
    let doc = document(broken, MermaidMode::Image, Flavor::AzureDevops);
    let d = only_diagram(&doc);
    assert!(d.png().is_none());
    assert!(d.error().is_some());
    assert!(
        doc.diagnostics
            .iter()
            .any(|x| x.severity == Severity::Error)
    );
}
