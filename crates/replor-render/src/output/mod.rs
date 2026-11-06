//! stdout backends. Both consume the same rendered surface; only the mapping
//! from semantic role to bytes differs.

pub mod ansi;
pub mod plain;

use std::io::Write;

use crate::error::{AppError, Result};
use crate::layout::theme::Theme;
use crate::layout::{RenderedDocument, RenderedLine, RenderedSpan};

/// Whether the stdout backend emits ANSI escapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    Ansi,
    Plain,
}

pub fn write_document(
    out: &mut impl Write,
    document: &RenderedDocument,
    color: ColorChoice,
    theme: &Theme,
    hyperlinks: bool,
) -> Result<()> {
    let result = match color {
        ColorChoice::Ansi => ansi::write_document(out, document, theme, hyperlinks),
        ColorChoice::Plain => plain::write_document(out, document),
    };
    result.map_err(|source| AppError::Output { source })
}

/// Drop trailing whitespace so piped output and `fzf --preview` panes stay clean.
pub fn trimmed(line: &RenderedLine) -> Vec<RenderedSpan> {
    let mut spans = line.spans.clone();
    while let Some(last) = spans.last_mut() {
        let trimmed = last.text.trim_end();
        if trimmed.is_empty() {
            spans.pop();
        } else {
            last.text.truncate(trimmed.len());
            break;
        }
    }
    spans
}
