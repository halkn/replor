//! Terminal layout: document IR to a rendered surface carrying semantic style
//! roles. No ratatui or ANSI types appear here; the output backends map roles.

mod diagram;
pub mod document;
pub mod highlight;
pub mod icons;
pub mod inline;
mod prepare;
pub mod table;
pub mod theme;
pub mod wrap;

pub use document::layout_document;
pub use prepare::PreparedDocument;

use std::rc::Rc;

use crate::diagnostic::Diagnostic;
use crate::image::Placement;
use crate::markdown::model::AlertKind;
use crate::source::SourceRange;

/// Semantic role of a run of text. Backends decide the concrete styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleRole {
    Normal,
    Muted,
    Heading(u8),
    Strong,
    Emphasis,
    Strike,
    InlineCode,
    Code,
    CodeBorder,
    Link,
    LinkTarget,
    Quote,
    /// Chrome and label of a GFM alert. The kind chooses the colour.
    Alert(AlertKind),
    ListMarker,
    TaskChecked,
    TaskUnchecked,
    TableBorder,
    Diagram,
    DiagramBorder,
    Warning,
    Error,
    SearchMatch,
    /// The match `n` and `N` are on, told apart from the others so the reader
    /// can see which one the viewport was moved to.
    CurrentMatch,
    InitialLine,
    Status,
    /// A token inside a code block. Highlighting classifies tokens; the theme
    /// still chooses the colours.
    Syntax(SyntaxKind),
}

/// Token classes a code block is split into. Deliberately coarse: these are the
/// distinctions that survive a 16-colour terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntaxKind {
    Keyword,
    String,
    Number,
    Comment,
    Type,
    Function,
    Punctuation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedSpan {
    pub text: String,
    pub role: StyleRole,
    /// Absolute URL this run points at, when a backend can make it clickable.
    /// Only the ANSI stdout backend uses it; the pager renders plain styled text.
    pub link: Option<String>,
}

impl RenderedSpan {
    pub fn new(text: impl Into<String>, role: StyleRole) -> Self {
        Self {
            text: crate::text::into_visible(text.into()),
            role,
            link: None,
        }
    }

    pub fn with_link(mut self, link: impl Into<String>) -> Self {
        self.link = Some(link.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderedLine {
    pub spans: Vec<RenderedSpan>,
    pub source_range: Option<SourceRange>,
    /// Set for content that must not be re-wrapped, such as code and diagrams.
    /// The pager scrolls horizontally instead.
    pub no_wrap: bool,
    /// An image to draw over this line and the blank lines reserved below it.
    /// Set on the first reserved line only, and only when the terminal can draw
    /// one; otherwise the image stays a text placeholder and this is `None`.
    pub image: Option<Rc<Placement>>,
}

impl RenderedLine {
    pub fn blank() -> Self {
        Self::default()
    }

    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderedDocument {
    pub lines: Vec<RenderedLine>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Inputs that change the rendered surface. Layout is re-run when these change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutOptions {
    pub width: usize,
}

impl LayoutOptions {
    pub const MIN_WIDTH: usize = 20;

    pub fn new(width: usize) -> Self {
        Self {
            width: width.max(Self::MIN_WIDTH),
        }
    }
}
