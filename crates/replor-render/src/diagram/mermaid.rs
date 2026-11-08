//! The only module that may reference `merman`.
//!
//! Keeping the dependency behind this boundary means a `merman` update can only
//! break this file. Output is terminal text, or a PNG for a terminal graphics
//! protocol; SVG never leaves this module.

use std::any::Any;
use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};

use merman::ascii::{
    AsciiError, AsciiOutput, AsciiProjection, AsciiRenderOptions, TerminalWidthProfile,
};
use merman::svg::SvgPipeline;
use merman::{
    AsciiRequest, OperationControl, RenderError, RenderOutput, RenderRequest, Renderer, SvgRequest,
};

use crate::diagram::{MermaidMode, MermaidWidth};
use crate::markdown::model::DiagramPng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagramRenderOptions {
    pub mode: MermaidMode,
    pub width: MermaidWidth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedDiagram {
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramError {
    /// Already normalised for stable snapshots.
    pub message: String,
    /// The diagram parsed but has no terminal text renderer, as opposed to
    /// being malformed. Presented differently, since nothing is wrong with the
    /// author's source.
    pub unsupported: bool,
}

pub trait DiagramRenderer {
    fn render(
        &self,
        source: &str,
        options: &DiagramRenderOptions,
    ) -> Result<RenderedDiagram, DiagramError>;

    fn render_png(&self, source: &str) -> Result<DiagramPng, DiagramError>;
}

type AsciiCall = fn(&str, AsciiRenderOptions) -> Result<Option<AsciiOutput>, RenderError>;
type SvgCall = fn(&str) -> Result<Option<String>, RenderError>;

/// The calls into `merman` are fields so a test can stand in one that panics,
/// since the guard around them has to be exercised without a real input that
/// triggers it.
#[derive(Debug, Clone, Copy)]
pub struct MermanRenderer {
    ascii: AsciiCall,
    svg: SvgCall,
}

impl Default for MermanRenderer {
    fn default() -> Self {
        Self {
            ascii: merman_ascii,
            svg: merman_svg,
        }
    }
}

impl DiagramRenderer for MermanRenderer {
    fn render(
        &self,
        source: &str,
        options: &DiagramRenderOptions,
    ) -> Result<RenderedDiagram, DiagramError> {
        let charset = match options.mode {
            MermaidMode::Ascii => AsciiRenderOptions::ascii(),
            // Unicode is the default; source and off never reach the renderer.
            _ => AsciiRenderOptions::unicode(),
        };
        let profile = match options.width {
            MermaidWidth::Unicode => TerminalWidthProfile::Unicode,
            MermaidWidth::Cjk => TerminalWidthProfile::Cjk,
        };
        let request = charset.with_terminal_width_profile(profile);
        match guarded(|| (self.ascii)(source, request))? {
            // A structured text projection lists the parsed fields instead of
            // drawing the diagram; the author's source reads better than that.
            Ok(Some(output)) if output.projection == AsciiProjection::StructuredText => {
                Err(DiagramError {
                    message: format!("`{}` has no text drawing", output.family),
                    unsupported: true,
                })
            }
            Ok(Some(output)) => Ok(RenderedDiagram {
                lines: output.text.lines().map(str::to_string).collect(),
            }),
            Ok(None) => Err(no_diagram()),
            Err(err) => Err(from_render_error(&err)),
        }
    }

    fn render_png(&self, source: &str) -> Result<DiagramPng, DiagramError> {
        // The rasteriser runs inside the guard too: it consumes the SVG
        // `merman` produced, so a bad diagram can reach it as well.
        guarded(|| match (self.svg)(source) {
            Ok(Some(svg)) => super::raster::svg_to_png(&svg)
                .map(|raster| DiagramPng {
                    bytes: raster.png.into(),
                    css_width: raster.css_width,
                })
                .map_err(|message| DiagramError {
                    message: normalize(&message),
                    unsupported: false,
                }),
            Ok(None) => Err(no_diagram()),
            Err(err) => Err(from_render_error(&err)),
        })?
    }
}

fn merman_ascii(
    source: &str,
    options: AsciiRenderOptions,
) -> Result<Option<AsciiOutput>, RenderError> {
    // The default resource policy is `merman`'s interactive profile. It stays,
    // because a PR head is untrusted input.
    let request = AsciiRequest {
        options,
        ..AsciiRequest::default()
    };
    match Renderer::new().render(RenderRequest::ascii(
        source,
        OperationControl::new(),
        request,
    ))? {
        RenderOutput::Ascii(output) => Ok(output),
        _ => unreachable!("an ASCII request yields ASCII output"),
    }
}

fn merman_svg(source: &str) -> Result<Option<String>, RenderError> {
    // `resvg_safe` drops the markup `resvg` cannot draw; `raster.rs` relies on
    // it alongside its own href resolver.
    let request = SvgRequest {
        pipeline: Some(SvgPipeline::resvg_safe()),
        ..SvgRequest::default()
    };
    match Renderer::new().render(RenderRequest::svg(source, OperationControl::new(), request))? {
        RenderOutput::Svg(output) => Ok(output.map(|svg| svg.into_parts().0)),
        _ => unreachable!("an SVG request yields SVG output"),
    }
}

/// Turns a panic inside `merman` into an error for that one diagram, so the
/// rest of the document still renders.
///
/// `AssertUnwindSafe` holds because every call builds its own `Renderer` and
/// nothing it touched outlives the call.
fn guarded<T>(call: impl FnOnce() -> T) -> Result<T, DiagramError> {
    let outer = GUARDED.replace(true);
    let result = panic::catch_unwind(AssertUnwindSafe(call));
    GUARDED.set(outer);
    result.map_err(|payload| DiagramError {
        message: normalize(&format!("renderer panicked: {}", panic_message(&*payload))),
        unsupported: false,
    })
}

thread_local! {
    static GUARDED: Cell<bool> = const { Cell::new(false) };
}

/// Whether a panic on this thread now would be caught by `guarded`. A panic
/// hook runs before the unwind reaches `catch_unwind`, so a hook that restores
/// the terminal or prints must ask first, or a diagram that failed would
/// tear down a TUI that goes on running.
pub fn panic_is_caught() -> bool {
    GUARDED.get()
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown cause")
}

fn no_diagram() -> DiagramError {
    DiagramError {
        message: "no diagram detected".to_string(),
        unsupported: true,
    }
}

const NO_DIAGRAM_CODE: &str = "merman.parse.no_diagram_detected";
const UNSUPPORTED_DIAGRAM_CODE: &str = "merman.parse.unsupported_diagram";

/// Classified by the error's type or stable diagnostic code rather than its
/// text, which changes between `merman` versions. Unsupported covers
/// everything that leaves the author's source valid: a family or feature
/// without a text layout, and a diagram too large for the resource budget.
fn from_render_error(err: &RenderError) -> DiagramError {
    let code = match err {
        RenderError::Parse(diagnostic) => Some(diagnostic.terminal_diagnostic_details().code),
        _ => None,
    };
    if matches!(err, RenderError::NoDiagram) || code.as_deref() == Some(NO_DIAGRAM_CODE) {
        return no_diagram();
    }
    let unsupported = code.as_deref() == Some(UNSUPPORTED_DIAGRAM_CODE)
        || matches!(
            err,
            RenderError::ResourceLimitExceeded(_)
                | RenderError::Ascii(
                    AsciiError::UnsupportedDiagram { .. } | AsciiError::UnsupportedFeature { .. }
                )
        );
    DiagramError {
        message: normalize(&err.to_string()),
        unsupported,
    }
}

/// Reduce a renderer error to a short, stable single line. Backtraces, absolute
/// paths and volatile detail must not reach a snapshot.
fn normalize(message: &str) -> String {
    const LIMIT: usize = 120;
    let first = message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("render failed");
    let collapsed = first.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = collapsed.replace('\\', "/");
    if cleaned.chars().count() <= LIMIT {
        return cleaned;
    }
    let truncated: String = cleaned.chars().take(LIMIT).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flavor::{self, Flavor};
    use crate::markdown::model::{Block, DiagramOutput};
    use crate::source::SourceText;

    fn render(source: &str, mode: MermaidMode) -> Result<RenderedDiagram, DiagramError> {
        render_with_width(source, mode, MermaidWidth::Unicode)
    }

    fn render_with_width(
        source: &str,
        mode: MermaidMode,
        width: MermaidWidth,
    ) -> Result<RenderedDiagram, DiagramError> {
        MermanRenderer::default().render(source, &DiagramRenderOptions { mode, width })
    }

    /// A flowchart `merman` can draw as SVG but not lay out as text: the
    /// returning edge would cross the reserved graph geometry.
    const LOOP: &str = "flowchart LR\n  A[Start] --> B{Review?}\n  B -->|ok| C[Merge]\n  B -->|ng| D[Fix]\n  D --> B\n";

    fn panicking() -> MermanRenderer {
        MermanRenderer {
            ascii: |_, _| panic!("boom"),
            svg: |_| panic!("boom"),
        }
    }

    #[test]
    fn a_graph_renders_to_text() {
        let out = render("graph LR\n  A --> B\n", MermaidMode::Unicode).expect("should render");
        assert!(!out.lines.is_empty());
        let text = out.lines.join("\n");
        assert!(text.contains('A'), "{text}");
        assert!(text.contains('B'), "{text}");
    }

    #[test]
    fn ascii_mode_stays_within_seven_bit_output() {
        let out = render("graph LR\n  A --> B\n", MermaidMode::Ascii).expect("should render");
        let text = out.lines.join("\n");
        assert!(text.is_ascii(), "non-ASCII output: {text}");
    }

    #[test]
    fn unicode_mode_uses_box_drawing() {
        let out = render("graph LR\n  A --> B\n", MermaidMode::Unicode).expect("should render");
        let text = out.lines.join("\n");
        assert!(!text.is_ascii(), "expected box drawing characters: {text}");
    }

    #[test]
    fn the_cjk_width_draws_the_frame_in_ascii() {
        let source = "graph LR\n  A[開始] --> B[完了]\n";
        let out = render_with_width(source, MermaidMode::Unicode, MermaidWidth::Cjk)
            .expect("should render");
        let text = out.lines.join("\n");
        assert!(text.contains("開始"), "{text}");
        // Box drawing characters are East Asian Ambiguous, so a terminal that
        // draws them two columns wide would misalign the frame.
        assert!(
            !text.chars().any(|c| ('\u{2500}'..='\u{257F}').contains(&c)),
            "expected ASCII structure: {text}"
        );
        assert_ne!(
            out,
            render(source, MermaidMode::Unicode).expect("should render")
        );
    }

    #[test]
    fn a_sequence_diagram_renders() {
        let out = render("sequenceDiagram\n  A->>B: Hello\n", MermaidMode::Unicode)
            .expect("should render");
        assert!(out.lines.join("\n").contains("Hello"));
    }

    #[test]
    fn japanese_labels_survive() {
        let out = render("graph LR\n  A[開始] --> B[完了]\n", MermaidMode::Unicode)
            .expect("should render");
        let text = out.lines.join("\n");
        assert!(text.contains("開始"), "{text}");
        assert!(text.contains("完了"), "{text}");
    }

    #[test]
    fn a_flowchart_crossing_its_own_layout_is_unsupported_as_text() {
        let err = render(LOOP, MermaidMode::Unicode).expect_err("should not render as text");
        assert!(err.unsupported, "{err:?}");
    }

    #[test]
    fn a_flowchart_crossing_its_own_layout_still_rasterises() {
        let png = MermanRenderer::default()
            .render_png(LOOP)
            .expect("should rasterise");
        assert!(png.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn unsupported_is_decided_by_the_error_type_not_its_text() {
        let feature = RenderError::Ascii(AsciiError::UnsupportedFeature {
            diagram_type: "flowchart",
            feature: "anything",
        });
        assert!(from_render_error(&feature).unsupported);
        let diagram = RenderError::Ascii(AsciiError::UnsupportedDiagram {
            diagram_type: "pie".to_string(),
        });
        assert!(from_render_error(&diagram).unsupported);
        let other = RenderError::UnsupportedTarget("does not support diagram type");
        assert!(!from_render_error(&other).unsupported);
    }

    #[test]
    fn a_family_with_only_structured_text_is_unsupported() {
        let gantt = "gantt\n  dateFormat YYYY-MM-DD\n  section S\n  A :a1, 2024-01-01, 3d\n";
        let err = render(gantt, MermaidMode::Unicode).expect_err("should not render as text");
        assert!(err.unsupported, "{err:?}");
        assert_eq!(err.message, "`gantt` has no text drawing");
    }

    #[test]
    fn a_diagram_beyond_the_resource_budget_is_unsupported() {
        let mut graph = String::from("graph TD\n");
        // A tree, so no edge crosses another and only its size can fail it.
        for i in 0..200 {
            graph.push_str(&format!("  N{i} --> N{}\n", 2 * i + 1));
            graph.push_str(&format!("  N{i} --> N{}\n", 2 * i + 2));
        }
        let err = render(&graph, MermaidMode::Unicode).expect_err("should exceed the budget");
        assert!(err.unsupported, "{err:?}");
        assert!(err.message.contains("resource limit"), "{err:?}");
    }

    #[test]
    fn an_empty_diagram_is_reported_as_no_diagram() {
        let err = render("%% todo\n", MermaidMode::Unicode).expect_err("nothing to render");
        assert_eq!(err, no_diagram());
    }

    #[test]
    fn invalid_syntax_reports_a_short_error() {
        let err =
            render("graph LR\n  A -->\n  -->\n", MermaidMode::Unicode).expect_err("should fail");
        assert!(!err.unsupported);
        assert!(!err.message.is_empty());
        assert!(!err.message.contains('\n'));
        assert!(err.message.chars().count() <= 121);
    }

    #[test]
    fn nonsense_input_does_not_panic() {
        let _ = render("this is not mermaid at all", MermaidMode::Unicode);
    }

    #[test]
    fn a_panic_becomes_an_error_for_that_diagram() {
        let options = DiagramRenderOptions {
            mode: MermaidMode::Unicode,
            width: MermaidWidth::Unicode,
        };
        let err = panicking()
            .render("graph LR\n  A --> B\n", &options)
            .expect_err("a panic should be an error");
        assert_eq!(err.message, "renderer panicked: boom");
        assert!(!err.unsupported);
        let err = panicking()
            .render_png("graph LR\n  A --> B\n")
            .expect_err("a panic should be an error");
        assert_eq!(err.message, "renderer panicked: boom");
    }

    #[test]
    fn a_hook_can_tell_the_panic_will_be_caught() {
        assert!(!panic_is_caught());
        let seen = guarded(panic_is_caught).expect("no panic");
        assert!(seen);
        assert!(!panic_is_caught());
        let _ = guarded(|| panic!("boom"));
        assert!(!panic_is_caught());
    }

    #[test]
    fn a_panicking_diagram_keeps_the_rest_of_the_document() {
        let text = "# Before\n\n```mermaid\ngraph LR\n  A --> B\n```\n\nAfter the diagram.\n";
        let flavor = Flavor::AzureDevops;
        let mut document = flavor::parse(SourceText::new(text.to_string()), flavor);
        super::super::resolve_with(
            &mut document,
            &panicking(),
            MermaidMode::Unicode,
            MermaidWidth::Unicode,
            flavor,
        );
        assert!(matches!(document.blocks.first(), Some(Block::Heading(_))));
        assert!(matches!(document.blocks.last(), Some(Block::Paragraph(_))));
        let failed = document.blocks.iter().find_map(|block| match block {
            Block::Diagram(d) => Some(&d.output),
            _ => None,
        });
        assert!(
            matches!(
                failed,
                Some(DiagramOutput::Failed { message, unsupported: false })
                    if message == "renderer panicked: boom"
            ),
            "{failed:?}"
        );
    }

    #[test]
    fn a_graph_rasterises_to_png() {
        let png = MermanRenderer::default()
            .render_png("graph LR\n  A[開始] --> B[完了]\n")
            .expect("should rasterise");
        assert!(png.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn invalid_syntax_fails_to_rasterise_with_a_short_error() {
        let err = MermanRenderer::default()
            .render_png("graph LR\n  A -->\n  -->\n")
            .expect_err("should fail");
        assert!(!err.message.is_empty());
        assert!(!err.message.contains('\n'));
    }

    #[test]
    fn error_normalisation_collapses_whitespace_and_truncates() {
        assert_eq!(normalize("  boom \n details"), "boom");
        assert_eq!(normalize("a\n\nb"), "a");
        assert_eq!(normalize(""), "render failed");
        let long = "x".repeat(300);
        let normalized = normalize(&long);
        assert_eq!(normalized.chars().count(), 121);
        assert!(normalized.ends_with('…'));
    }
}
