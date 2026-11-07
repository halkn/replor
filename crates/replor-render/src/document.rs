//! Markdown text to a document ready for layout.
//!
//! Startup and every `--watch` reload go through `build`, so a reloaded
//! document is indistinguishable from one opened fresh. A document read from a
//! snapshot goes through `load` first, which decodes it the same way whatever
//! the snapshot is.

use std::path::PathBuf;
use std::sync::Arc;

use crate::diagram::{self, MermaidMode, MermaidWidth};
use crate::error::{AppError, Result};
use crate::flavor::{self, Flavor};
use crate::markdown::model::Document;
use crate::snapshot::{DocumentSource, Snapshot, SnapshotPath};
use crate::source::SourceText;

/// A decoded document and where it came from.
#[derive(Debug, Clone)]
pub struct LoadedDocument {
    pub text: String,
    pub display_name: String,
    /// The document's directory, for displaying relative targets
    /// (`InlineContext::base_dir`).
    pub base_dir: PathBuf,
    pub source: DocumentSource,
}

/// Largest document read. A revision may be someone else's PR head, where a
/// blob of a few compressed megabytes can inflate to gigabytes.
pub const MAX_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;

/// Reads and decodes the document at `path`, following a symlink the way an
/// image is followed. The name and directory shown are the path as opened;
/// images are read from where the document really is.
pub fn load(snapshot: Arc<dyn Snapshot>, path: SnapshotPath) -> Result<LoadedDocument> {
    let failed = |source| AppError::Read {
        path: path.to_string(),
        source,
    };
    let resolved = snapshot.resolve(&path).map_err(failed)?;
    let bytes = snapshot
        .read(&resolved, MAX_DOCUMENT_BYTES)
        .map_err(failed)?;
    let display_name = path.to_string();
    let text = decode(bytes, &display_name)?;
    Ok(LoadedDocument {
        text,
        display_name,
        base_dir: PathBuf::from(path.parent().unwrap_or_default().as_str()),
        source: DocumentSource::new(snapshot, resolved),
    })
}

/// Document bytes to text. Every snapshot's documents, and stdin's, go
/// through here, so the same bytes always render the same.
pub fn decode(bytes: Vec<u8>, origin: &str) -> Result<String> {
    // A leading BOM is stripped so it never reaches the layout engine as a
    // zero-width character on the first line.
    let bytes = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => rest.to_vec(),
        None => bytes,
    };
    String::from_utf8(bytes).map_err(|err| AppError::Decode {
        origin: origin.to_string(),
        offset: err.utf8_error().valid_up_to(),
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::memory::MemorySnapshot;

    #[test]
    fn a_leading_bom_is_stripped() {
        assert_eq!(decode(b"\xEF\xBB\xBF# T\n".to_vec(), "a").unwrap(), "# T\n");
        assert_eq!(decode(b"# T\n".to_vec(), "a").unwrap(), "# T\n");
    }

    #[test]
    fn rejects_non_utf8_bytes() {
        match decode(vec![b'#', b' ', 0xFF], "stdin").unwrap_err() {
            AppError::Decode { offset, .. } => assert_eq!(offset, 2),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn a_document_carries_its_directory_and_source() {
        let snapshot = MemorySnapshot::new("");
        snapshot.write("docs/a.md", "# 見出し\n");
        let path = SnapshotPath::new("docs/a.md").unwrap();
        let loaded = load(snapshot, path.clone()).unwrap();
        assert_eq!(loaded.text, "# 見出し\n");
        assert_eq!(loaded.display_name, "docs/a.md");
        assert_eq!(loaded.base_dir, PathBuf::from("docs"));
        assert_eq!(loaded.source.path(), &path);
    }

    #[test]
    fn a_document_at_the_root_has_an_empty_directory() {
        let snapshot = MemorySnapshot::new("");
        snapshot.write("a.md", "a");
        let loaded = load(snapshot, SnapshotPath::new("a.md").unwrap()).unwrap();
        assert_eq!(loaded.base_dir, PathBuf::new());
    }

    #[test]
    fn a_missing_document_is_a_read_error() {
        let snapshot = MemorySnapshot::new("");
        let err = load(snapshot, SnapshotPath::new("a.md").unwrap()).unwrap_err();
        assert!(matches!(err, AppError::Read { .. }), "{err:?}");
    }
}
