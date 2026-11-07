//! Markdown text to a document ready for layout.
//!
//! Startup and every `--watch` reload go through `build`, so a reloaded
//! document is indistinguishable from one opened fresh.

use crate::diagram::{self, MermaidMode, MermaidWidth};
use crate::flavor::{self, Flavor};
use crate::markdown::model::Document;
use crate::source::SourceText;

pub fn build(
    text: String,
    flavor: Flavor,
    mermaid: MermaidMode,
    mermaid_width: MermaidWidth,
) -> Document {
    let mut document = flavor::parse(SourceText::new(text), flavor);
    diagram::resolve(&mut document, mermaid, mermaid_width, flavor);
    document
}
