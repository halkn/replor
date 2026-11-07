use std::io::Write;

use crate::layout::RenderedDocument;
use crate::output::trimmed;

/// Writes visible text only. No escape sequence of any kind is emitted.
pub fn write_document(out: &mut impl Write, document: &RenderedDocument) -> std::io::Result<()> {
    for line in &document.lines {
        for span in trimmed(line) {
            out.write_all(span.text.as_bytes())?;
        }
        out.write_all(b"\n")?;
    }
    out.flush()
}
