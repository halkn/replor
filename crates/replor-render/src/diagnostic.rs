use crate::source::SourceRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

/// A non-fatal problem found while parsing or rendering. Diagnostics are shown
/// next to the offending content and counted in the status bar; they never
/// abort the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub range: Option<SourceRange>,
}

impl Diagnostic {
    pub fn warning(message: impl Into<String>, range: Option<SourceRange>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            range,
        }
    }

    pub fn error(message: impl Into<String>, range: Option<SourceRange>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            range,
        }
    }
}
