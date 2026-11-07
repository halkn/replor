//! The only module that may reference `merman`.
//!
//! Keeping the dependency behind this boundary means a `merman` update can only
//! break this file. Output is terminal text, or a PNG for a terminal graphics
//! protocol; SVG never leaves this module.

use merman::ascii::{AsciiRenderOptions, HeadlessAsciiRenderer};
use merman::render::HeadlessRenderer;

use crate::diagram::MermaidMode;
use crate::markdown::model::DiagramPng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagramRenderOptions {
    pub mode: MermaidMode,
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

#[derive(Debug, Default, Clone, Copy)]
pub struct MermanRenderer;

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
        let renderer = HeadlessAsciiRenderer::new().with_ascii_options(charset);
        match renderer.render_ascii_sync(source) {
            Ok(Some(text)) => Ok(RenderedDiagram {
                lines: text.lines().map(str::to_string).collect(),
            }),
            // A diagram that is not detected at all yields no output.
            Ok(None) => Err(DiagramError {
                message: "no diagram detected".to_string(),
                unsupported: true,
            }),
            Err(err) => {
                let message = normalize(&err.to_string());
                Err(DiagramError {
                    unsupported: is_unsupported(&message),
                    message,
                })
            }
        }
    }

    fn render_png(&self, source: &str) -> Result<DiagramPng, DiagramError> {
        match HeadlessRenderer::new().render_svg_resvg_safe_sync(source) {
            Ok(Some(svg)) => super::raster::svg_to_png(&svg)
                .map(|raster| DiagramPng {
                    bytes: raster.png.into(),
                    css_width: raster.css_width,
                })
                .map_err(|message| DiagramError {
                    message: normalize(&message),
                    unsupported: false,
                }),
            Ok(None) => Err(DiagramError {
                message: "no diagram detected".to_string(),
                unsupported: true,
            }),
            Err(err) => {
                let message = normalize(&err.to_string());
                Err(DiagramError {
                    unsupported: is_unsupported(&message),
                    message,
                })
            }
        }
    }
}

/// Distinguish "this diagram family has no text renderer" from a genuine
/// syntax error, so the reader is not told their source is broken.
fn is_unsupported(message: &str) -> bool {
    let lowered = message.to_lowercase();
    lowered.contains("does not support diagram type") || lowered.contains("unsupported diagram")
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

    fn render(source: &str, mode: MermaidMode) -> Result<RenderedDiagram, DiagramError> {
        MermanRenderer.render(source, &DiagramRenderOptions { mode })
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
    fn invalid_syntax_reports_a_short_error() {
        let err =
            render("graph LR\n  A -->\n  -->\n", MermaidMode::Unicode).expect_err("should fail");
        assert!(!err.message.is_empty());
        assert!(!err.message.contains('\n'));
        assert!(err.message.chars().count() <= 121);
    }

    #[test]
    fn nonsense_input_does_not_panic() {
        let _ = render("this is not mermaid at all", MermaidMode::Unicode);
    }

    #[test]
    fn a_graph_rasterises_to_png() {
        let png = MermanRenderer
            .render_png("graph LR\n  A[開始] --> B[完了]\n")
            .expect("should rasterise");
        assert!(png.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn invalid_syntax_fails_to_rasterise_with_a_short_error() {
        let err = MermanRenderer
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
