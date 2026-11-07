//! Diagram resolution.
//!
//! Diagrams are rendered once, after parsing and before any layout, so the
//! frame loop never calls the renderer. Neither `merman`'s text output nor the
//! PNG depends on a target width, so a resize reuses the same rendered diagram;
//! the terminal scales a picture to the cells it is given.

pub mod azure_compat;
pub mod mermaid;
mod raster;

#[cfg(test)]
mod tests;

use clap::ValueEnum;

use crate::diagnostic::Diagnostic;
use crate::flavor::Flavor;
use crate::markdown::model::{Block, DiagramBlock, DiagramOutput, Document};
use mermaid::{DiagramRenderOptions, DiagramRenderer, MermanRenderer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum MermaidMode {
    #[default]
    Unicode,
    Ascii,
    Image,
    Source,
    Off,
}

pub fn resolve(document: &mut Document, mode: MermaidMode, flavor: Flavor) {
    let renderer = MermanRenderer;
    let mut diagnostics = Vec::new();
    resolve_blocks(
        &mut document.blocks,
        &renderer,
        mode,
        flavor,
        &mut diagnostics,
    );
    document.diagnostics.append(&mut diagnostics);
}

fn resolve_blocks(
    blocks: &mut [Block],
    renderer: &impl DiagramRenderer,
    mode: MermaidMode,
    flavor: Flavor,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for block in blocks.iter_mut() {
        if let Block::Diagram(diagram) = block {
            diagram.output = DiagramOutput::Source;
            if flavor == Flavor::AzureDevops {
                diagram.warnings = azure_compat::warnings(&diagram.source);
                for warning in &diagram.warnings {
                    diagnostics.push(Diagnostic::warning(warning.clone(), Some(diagram.range)));
                }
            }
            match mode {
                // Source and off never invoke the renderer.
                MermaidMode::Source | MermaidMode::Off => {}
                MermaidMode::Image => match renderer.render_png(&diagram.source) {
                    Ok(png) => diagram.output = DiagramOutput::Image(png),
                    // Text may still draw it, and reports the error if not.
                    Err(_) => render_text(diagram, renderer, MermaidMode::Unicode, diagnostics),
                },
                MermaidMode::Unicode | MermaidMode::Ascii => {
                    render_text(diagram, renderer, mode, diagnostics)
                }
            }
        }
        for children in block.children_mut() {
            resolve_blocks(children, renderer, mode, flavor, diagnostics);
        }
    }
}

fn render_text(
    diagram: &mut DiagramBlock,
    renderer: &impl DiagramRenderer,
    mode: MermaidMode,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match renderer.render(&diagram.source, &DiagramRenderOptions { mode }) {
        Ok(rendered) => diagram.output = DiagramOutput::Text(rendered.lines),
        Err(error) => {
            // A failed diagram falls back to its source, never failing the document.
            let severity = if error.unsupported {
                Diagnostic::warning
            } else {
                Diagnostic::error
            };
            diagnostics.push(severity(
                format!("mermaid: {}", error.message),
                Some(diagram.range),
            ));
            diagram.output = DiagramOutput::Failed {
                message: error.message,
                unsupported: error.unsupported,
            };
        }
    }
}
