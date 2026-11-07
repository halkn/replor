//! Diagram blocks to rendered lines: the rendered text, a picture, the source
//! or an error with the source underneath.

use crate::diagram::MermaidMode;
use crate::layout::PreparedDocument;
use crate::layout::document::{BORDER, expand_tabs, line, reserve};
use crate::layout::wrap::{display_width, wrap_spans};
use crate::layout::{RenderedLine, RenderedSpan, StyleRole};
use crate::markdown::model::{DiagramBlock, DiagramOutput};

pub(super) fn diagram(
    d: &DiagramBlock,
    width: usize,
    ctx: &PreparedDocument,
    out: &mut Vec<RenderedLine>,
) {
    if ctx.inline().mermaid == MermaidMode::Off {
        out.push(line(
            vec![RenderedSpan::new(
                format!("[{} diagram omitted]", d.language),
                StyleRole::Muted,
            )],
            d.range,
        ));
        return;
    }

    // Compatibility warnings sit directly above the diagram they describe.
    for warning in &d.warnings {
        for spans in wrap_spans(
            &[RenderedSpan::new(
                format!("! {warning}"),
                StyleRole::Warning,
            )],
            width,
        ) {
            out.push(line(spans, d.range));
        }
    }

    if let DiagramOutput::Failed {
        message,
        unsupported,
    } = &d.output
    {
        error_fallback(d, message, *unsupported, width, out);
        return;
    }

    if let Some(placement) = ctx.image(d.range, width) {
        reserve(&placement, d.range, out);
        return;
    }

    let (header, body): (String, Vec<String>) = match &d.output {
        DiagramOutput::Text(lines) => (format!("╭─ {}", d.language), lines.clone()),
        _ => (
            format!("╭─ {} (source)", d.language),
            d.source.lines().map(str::to_string).collect(),
        ),
    };

    out.push(preformatted(header, StyleRole::DiagramBorder, d, true));
    for text in body {
        out.push(RenderedLine {
            spans: vec![
                RenderedSpan::new(BORDER, StyleRole::DiagramBorder),
                // Diagram lines keep their leading whitespace and are never
                // re-wrapped; the pager scrolls horizontally instead.
                RenderedSpan::new(expand_tabs(&text), StyleRole::Diagram),
            ],
            source_range: Some(d.range),
            no_wrap: true,
            image: None,
        });
    }
    out.push(preformatted(
        "╰─".to_string(),
        StyleRole::DiagramBorder,
        d,
        true,
    ));
}

/// Show a short error and the original source, so a broken diagram never hides
/// what the author wrote.
fn error_fallback(
    d: &DiagramBlock,
    message: &str,
    unsupported: bool,
    width: usize,
    out: &mut Vec<RenderedLine>,
) {
    let (title, role) = if unsupported {
        ("Mermaid diagram not supported as text", StyleRole::Warning)
    } else {
        ("Mermaid render error", StyleRole::Error)
    };
    let dashes = width.saturating_sub(display_width(title) + 4).max(2);
    out.push(preformatted(
        format!("╭─ {title} {}", "─".repeat(dashes)),
        role,
        d,
        true,
    ));
    for spans in wrap_spans(
        &[RenderedSpan::new(message, role)],
        width.saturating_sub(2).max(1),
    ) {
        let mut all = vec![RenderedSpan::new(BORDER, role)];
        all.extend(spans);
        out.push(RenderedLine {
            spans: all,
            source_range: Some(d.range),
            no_wrap: false,
            image: None,
        });
    }
    out.push(preformatted(
        format!("├{}", "─".repeat(width.saturating_sub(1).max(2))),
        role,
        d,
        true,
    ));
    for text in d.source.lines() {
        out.push(RenderedLine {
            spans: vec![
                RenderedSpan::new(BORDER, role),
                RenderedSpan::new(expand_tabs(text), StyleRole::Diagram),
            ],
            source_range: Some(d.range),
            no_wrap: true,
            image: None,
        });
    }
    out.push(preformatted(
        format!("╰{}", "─".repeat(width.saturating_sub(1).max(2))),
        role,
        d,
        true,
    ));
}

fn preformatted(text: String, role: StyleRole, d: &DiagramBlock, no_wrap: bool) -> RenderedLine {
    RenderedLine {
        spans: vec![RenderedSpan::new(text, role)],
        source_range: Some(d.range),
        no_wrap,
        image: None,
    }
}
