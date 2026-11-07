//! The Snapshot trait: how the render core reads files.
//!
//! The render core never touches the filesystem or Git itself. A working tree
//! and a revision both sit behind this trait, so the same rendering code runs
//! on either. The contract is `docs/design/p1-core.md`, "Snapshot の trait".

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

/// A path inside a snapshot: relative to its root, `/` separated, UTF-8, with
/// no empty, `.` or `..` component and no NUL. The root is the empty path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SnapshotPath(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("{0:?} is not a relative path of plain components")]
    Invalid(String),
    #[error("{0:?} leaves the snapshot root")]
    AboveRoot(String),
}

impl SnapshotPath {
    pub fn root() -> Self {
        Self::default()
    }

    /// A path that is already in canonical form. Nothing is normalised: a
    /// component that would need it makes the whole path invalid.
    pub fn new(path: &str) -> Result<Self, PathError> {
        if path.is_empty() {
            return Ok(Self::root());
        }
        if path.split('/').all(is_plain_component) {
            Ok(Self(path.to_string()))
        } else {
            Err(PathError::Invalid(path.to_string()))
        }
    }

    /// The entry `name` inside this directory, or `None` for a name a listing
    /// must not show. A name is never normalised: an untrusted tree could use
    /// one to alias another path or to make the tree loop.
    pub fn child(&self, name: &str) -> Option<Self> {
        if !is_plain_component(name) || name.eq_ignore_ascii_case(".git") {
            return None;
        }
        Some(self.joined(name))
    }

    /// Resolves a Markdown reference (a link or image path) from `document`.
    /// A leading `/` starts from `content_root`, as GitHub and Azure DevOps do;
    /// anything else starts from the document's directory. `.` and `..` are
    /// resolved lexically, so the result can still pass through a symlink: the
    /// snapshot checks where a read really ends up.
    pub fn from_reference(
        document: &SnapshotPath,
        content_root: &SnapshotPath,
        reference: &str,
    ) -> Result<Self, PathError> {
        match reference.strip_prefix('/') {
            Some(rest) => content_root.join_lexical(rest),
            None => document
                .parent()
                .unwrap_or_default()
                .join_lexical(reference),
        }
    }

    /// `relative` joined onto this directory with `.` and `..` resolved
    /// lexically. Empty components (`a//b`) are skipped.
    pub fn join_lexical(&self, relative: &str) -> Result<Self, PathError> {
        let mut parts: Vec<&str> = self.components().collect();
        for part in relative.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err(PathError::AboveRoot(relative.to_string()));
                    }
                }
                part if part.contains('\0') => {
                    return Err(PathError::Invalid(relative.to_string()));
                }
                part => parts.push(part),
            }
        }
        Ok(Self(parts.join("/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|part| !part.is_empty())
    }

    /// The last component, or `None` for the root.
    pub fn name(&self) -> Option<&str> {
        self.components().last()
    }

    /// The containing directory, or `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.rsplit_once('/') {
            Some((parent, _)) => Self(parent.to_string()),
            None => Self::root(),
        })
    }

    /// Whether `ancestor` is this path or one of its directories, compared by
    /// component so `docs` does not contain `docs2`.
    pub fn starts_with(&self, ancestor: &SnapshotPath) -> bool {
        ancestor.is_root()
            || self.0 == ancestor.0
            || self
                .0
                .strip_prefix(&ancestor.0)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    fn joined(&self, name: &str) -> Self {
        if self.is_root() {
            Self(name.to_string())
        } else {
            Self(format!("{}/{name}", self.0))
        }
    }
}

impl fmt::Display for SnapshotPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn is_plain_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0'])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    /// A gitlink in a revision's tree. A working tree lists a nested
    /// repository as a `Directory` instead, since its files are on disk.
    Submodule,
    /// A FIFO, socket or device.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: SnapshotPath,
    pub kind: EntryKind,
}

/// Puts entries in the order every `Snapshot::list` returns: directories
/// first, then everything else, each by the UTF-8 bytes of the name. Neither
/// Git's tree order nor the OS's directory order would match across
/// implementations.
pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        let group = |entry: &Entry| entry.kind != EntryKind::Directory;
        match group(a).cmp(&group(b)) {
            Ordering::Equal => a.name.as_bytes().cmp(b.name.as_bytes()),
            other => other,
        }
    });
}

/// Errors carry no implementation type, so the render core does not depend on
/// `gix` or on how a working tree is read.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("not found")]
    NotFound,
    #[error("not a regular file")]
    NotAFile,
    #[error("outside the snapshot root")]
    OutsideRoot,
    #[error("larger than {limit} bytes")]
    TooLarge { limit: u64 },
    #[error("not supported by this snapshot")]
    Unsupported,
    #[error(transparent)]
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

pub trait Snapshot: Send + Sync {
    /// One directory level, in the order `sort_entries` gives.
    fn list(&self, dir: &SnapshotPath) -> Result<Vec<Entry>, SnapshotError>;
    /// Follows symlinks to the regular file `path` names, which must stay
    /// inside the root.
    fn resolve(&self, path: &SnapshotPath) -> Result<SnapshotPath, SnapshotError>;
    /// The contents of a regular file. Fails with `TooLarge` once the file
    /// passes `limit`, without reading the rest.
    fn read(&self, path: &SnapshotPath, limit: u64) -> Result<Vec<u8>, SnapshotError>;
    /// The boundary for the images a document may show, and the start of its
    /// `/` references.
    fn content_root(&self, document: &SnapshotPath) -> SnapshotPath;
    /// A name for the status bar: a path for a working tree, a short OID for a
    /// revision.
    fn label(&self) -> String;
}

/// A document together with the snapshot it was read from, which is where its
/// images are read from too.
#[derive(Clone)]
pub struct DocumentSource {
    snapshot: Arc<dyn Snapshot>,
    path: SnapshotPath,
    content_root: SnapshotPath,
}

impl DocumentSource {
    /// `path` should be the document as `Snapshot::resolve` returned it, so
    /// its references start from the directory it really is in, as they do on
    /// disk. The content root depends only on the document, so it is found
    /// once here rather than for every image.
    pub fn new(snapshot: Arc<dyn Snapshot>, path: SnapshotPath) -> Self {
        let content_root = snapshot.content_root(&path);
        Self {
            snapshot,
            path,
            content_root,
        }
    }

    pub fn snapshot(&self) -> &dyn Snapshot {
        &*self.snapshot
    }

    pub fn path(&self) -> &SnapshotPath {
        &self.path
    }

    pub fn content_root(&self) -> &SnapshotPath {
        &self.content_root
    }
}

impl fmt::Debug for DocumentSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentSource")
            .field("snapshot", &self.snapshot.label())
            .field("path", &self.path)
            .field("content_root", &self.content_root)
            .finish()
    }
}

/// A snapshot held in memory, for the render core's own tests.
#[cfg(test)]
pub(crate) mod memory {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct MemorySnapshot {
        files: Mutex<BTreeMap<String, Vec<u8>>>,
        content_root: SnapshotPath,
    }

    impl MemorySnapshot {
        pub fn new(content_root: &str) -> Arc<Self> {
            Arc::new(Self {
                files: Mutex::default(),
                content_root: SnapshotPath::new(content_root).expect("a valid content root"),
            })
        }

        pub fn write(&self, path: &str, bytes: impl Into<Vec<u8>>) {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_string(), bytes.into());
        }

        pub fn source(self: &Arc<Self>, document: &str) -> DocumentSource {
            DocumentSource::new(
                Arc::clone(self) as Arc<dyn Snapshot>,
                SnapshotPath::new(document).expect("a valid document path"),
            )
        }
    }

    impl Snapshot for MemorySnapshot {
        fn list(&self, _dir: &SnapshotPath) -> Result<Vec<Entry>, SnapshotError> {
            Err(SnapshotError::Unsupported)
        }

        fn resolve(&self, path: &SnapshotPath) -> Result<SnapshotPath, SnapshotError> {
            match self.files.lock().unwrap().contains_key(path.as_str()) {
                true => Ok(path.clone()),
                false => Err(SnapshotError::NotFound),
            }
        }

        fn read(&self, path: &SnapshotPath, limit: u64) -> Result<Vec<u8>, SnapshotError> {
            let files = self.files.lock().unwrap();
            let bytes = files.get(path.as_str()).ok_or(SnapshotError::NotFound)?;
            match bytes.len() as u64 > limit {
                true => Err(SnapshotError::TooLarge { limit }),
                false => Ok(bytes.clone()),
            }
        }

        fn content_root(&self, _document: &SnapshotPath) -> SnapshotPath {
            self.content_root.clone()
        }

        fn label(&self) -> String {
            "memory".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> SnapshotPath {
        SnapshotPath::new(text).expect("a valid path")
    }

    #[test]
    fn a_path_must_already_be_canonical() {
        assert_eq!(path("a/b.md").as_str(), "a/b.md");
        assert!(SnapshotPath::new("").unwrap().is_root());
        for bad in ["/a", "a/", "a//b", "./a", "a/../b", "a\0b"] {
            assert!(SnapshotPath::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn entry_names_that_would_alias_or_hide_a_path_are_not_listed() {
        let dir = path("docs");
        assert_eq!(dir.child("a.md"), Some(path("docs/a.md")));
        assert_eq!(SnapshotPath::root().child("a.md"), Some(path("a.md")));
        for bad in ["", ".", "..", "a/b", "a\0", ".git", ".GIT"] {
            assert_eq!(dir.child(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_reference_is_resolved_from_the_document_directory() {
        let doc = path("wiki/design/page.md");
        let root = path("wiki");
        let resolve = |r| SnapshotPath::from_reference(&doc, &root, r);
        assert_eq!(resolve("a.png"), Ok(path("wiki/design/a.png")));
        assert_eq!(resolve("./x/../a.png"), Ok(path("wiki/design/a.png")));
        assert_eq!(
            resolve("../.attachments/a.png"),
            Ok(path("wiki/.attachments/a.png"))
        );
        assert_eq!(
            resolve("/.attachments/a.png"),
            Ok(path("wiki/.attachments/a.png"))
        );
        assert_eq!(resolve("../../../a.png").ok(), None);
        assert_eq!(resolve("a\0.png").ok(), None);
    }

    #[test]
    fn a_reference_from_a_document_at_the_root() {
        let doc = path("README.md");
        let root = SnapshotPath::root();
        assert_eq!(
            SnapshotPath::from_reference(&doc, &root, "a.png"),
            Ok(path("a.png"))
        );
        assert!(SnapshotPath::from_reference(&doc, &root, "../a.png").is_err());
    }

    #[test]
    fn ancestry_is_compared_by_component() {
        assert!(path("docs/a.md").starts_with(&path("docs")));
        assert!(path("docs").starts_with(&path("docs")));
        assert!(path("docs").starts_with(&SnapshotPath::root()));
        assert!(!path("docs2/a.md").starts_with(&path("docs")));
        assert!(!path("docs").starts_with(&path("docs/a.md")));
    }

    #[test]
    fn parent_and_name() {
        assert_eq!(path("a/b/c.md").parent(), Some(path("a/b")));
        assert_eq!(path("c.md").parent(), Some(SnapshotPath::root()));
        assert_eq!(SnapshotPath::root().parent(), None);
        assert_eq!(path("a/b/c.md").name(), Some("c.md"));
        assert_eq!(SnapshotPath::root().name(), None);
    }

    #[test]
    fn directories_come_first_then_names_by_bytes() {
        let entry = |name: &str, kind| Entry {
            name: name.to_string(),
            path: path(name),
            kind,
        };
        let mut entries = vec![
            entry("b.md", EntryKind::File),
            entry("Z", EntryKind::Directory),
            entry("a", EntryKind::Symlink),
            entry("a", EntryKind::Directory),
            entry("B.md", EntryKind::File),
            entry("é.md", EntryKind::File),
        ];
        sort_entries(&mut entries);
        let order: Vec<_> = entries.iter().map(|e| (e.name.as_str(), e.kind)).collect();
        assert_eq!(
            order,
            [
                ("Z", EntryKind::Directory),
                ("a", EntryKind::Directory),
                ("B.md", EntryKind::File),
                ("a", EntryKind::Symlink),
                ("b.md", EntryKind::File),
                ("é.md", EntryKind::File),
            ]
        );
    }
}
