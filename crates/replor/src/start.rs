//! Where the tree opens.

use std::io;
use std::path::Path;

use replor_render::snapshot::SnapshotPath;
use replor_snapshot::WorktreeSnapshot;

pub struct Start {
    pub snapshot: WorktreeSnapshot,
    /// The directory the tree shows first.
    pub top: SnapshotPath,
    /// A file named on the command line, opened in the preview.
    pub file: Option<SnapshotPath>,
}

/// With no path, `docs/` under the working directory when there is one, as in
/// docsail; otherwise the working directory. A directory opens the tree there;
/// a file opens the tree at its directory with the file in the preview.
pub fn resolve(path: Option<&Path>, cwd: &Path) -> io::Result<Start> {
    let path = match path {
        Some(path) => cwd.join(path),
        None => {
            let docs = cwd.join("docs");
            match docs.is_dir() {
                true => docs,
                false => cwd.to_path_buf(),
            }
        }
    };
    let snapshot = WorktreeSnapshot::open(&path)?;
    let start = snapshot.start().clone();
    let (top, file) = match path.is_dir() {
        true => (start, None),
        false => (start.parent().unwrap_or_default(), Some(start)),
    };
    Ok(Start {
        snapshot,
        top,
        file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("docs/design")).unwrap();
        std::fs::write(dir.path().join("docs/design/a.md"), "a").unwrap();
        dir
    }

    #[test]
    fn docs_is_preferred_without_a_path() {
        let dir = repo();
        let start = resolve(None, dir.path()).unwrap();
        assert_eq!(start.top.as_str(), "docs");
        assert_eq!(start.file, None);
        assert_eq!(start.snapshot.root(), dir.path().canonicalize().unwrap());
    }

    #[test]
    fn without_docs_the_working_directory_opens() {
        let dir = tempfile::tempdir().unwrap();
        let start = resolve(None, dir.path()).unwrap();
        assert!(start.top.is_root());
    }

    #[test]
    fn a_directory_or_a_file_can_be_named() {
        let dir = repo();
        let start = resolve(Some(Path::new("docs/design")), dir.path()).unwrap();
        assert_eq!(start.top.as_str(), "docs/design");
        let start = resolve(Some(Path::new("docs/design/a.md")), dir.path()).unwrap();
        assert_eq!(start.top.as_str(), "docs/design");
        assert_eq!(start.file.unwrap().as_str(), "docs/design/a.md");
    }
}
