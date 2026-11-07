//! A working tree read with `std::fs`.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use ignore::gitignore::Gitignore;
use replor_render::snapshot::{
    Entry, EntryKind, Snapshot, SnapshotError, SnapshotPath, sort_entries,
};

#[derive(Debug, Clone)]
pub struct WorktreeSnapshot {
    /// Canonical, so every resolved path can be compared against it.
    root: PathBuf,
    start: SnapshotPath,
}

impl WorktreeSnapshot {
    /// The snapshot holding `start`: its nearest ancestor with a `.git`
    /// entry, or `start` itself outside a repository. `start` is where the
    /// tree opens, not a boundary.
    pub fn open(start: &Path) -> io::Result<Self> {
        let start = start.canonicalize()?;
        // A file opened outside a repository makes its directory the root, so
        // the images beside it can still be read.
        let dir = match start.is_dir() {
            true => start.as_path(),
            false => start.parent().unwrap_or(&start),
        };
        let root = dir
            .ancestors()
            .find(|dir| has_git_entry(dir))
            .unwrap_or(dir)
            .to_path_buf();
        let start = relative(&root, &start).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "start is not valid UTF-8")
        })?;
        Ok(Self { root, start })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the tree opens: the directory or file `open` was given.
    pub fn start(&self) -> &SnapshotPath {
        &self.start
    }

    fn absolute(&self, path: &SnapshotPath) -> PathBuf {
        path.components()
            .fold(self.root.clone(), |acc, part| acc.join(part))
    }

    /// The `.gitignore` files that apply to entries of `dir`, nearest last.
    /// They start at the repository holding `dir`, as Git's do: nothing above
    /// the root, and nothing from an outer repository inside a nested clone.
    /// A line Git would take literally but ignore cannot parse is skipped
    /// rather than failing the listing.
    fn gitignores(&self, dir: &SnapshotPath) -> Vec<Gitignore> {
        let mut dirs = vec![dir.clone()];
        let mut at = dir.clone();
        while !at.is_root() && !has_git_entry(&self.absolute(&at)) {
            at = at.parent().unwrap_or_default();
            dirs.push(at.clone());
        }
        dirs.iter()
            .rev()
            .map(|dir| self.absolute(dir).join(".gitignore"))
            .filter(|file| file.is_file())
            .map(|file| Gitignore::new(file).0)
            .collect()
    }
}

impl Snapshot for WorktreeSnapshot {
    fn list(&self, dir: &SnapshotPath) -> Result<Vec<Entry>, SnapshotError> {
        if in_git_dir(dir) {
            return Err(SnapshotError::NotFound);
        }
        let absolute = self.absolute(dir);
        // A listing never follows a symlink, including one on the way to `dir`.
        if absolute.canonicalize().map_err(io_error)? != absolute || !absolute.is_dir() {
            return Err(SnapshotError::NotFound);
        }
        // Hidden files are shown and only the repository's own `.gitignore`
        // files apply, as in docsail; the global excludes file and
        // `.git/info/exclude` do not.
        let gitignores = self.gitignores(dir);
        let mut entries = Vec::new();
        for item in std::fs::read_dir(&absolute).map_err(io_error)? {
            let item = item.map_err(io_error)?;
            let Some(path) = item.file_name().to_str().and_then(|name| dir.child(name)) else {
                continue;
            };
            // `DirEntry::file_type` does not follow a symlink.
            let kind = match item.file_type().map_err(io_error)? {
                t if t.is_dir() => EntryKind::Directory,
                t if t.is_file() => EntryKind::File,
                t if t.is_symlink() => EntryKind::Symlink,
                _ => EntryKind::Other,
            };
            let is_dir = kind == EntryKind::Directory;
            let ignored = gitignores
                .iter()
                .rev()
                .map(|gitignore| gitignore.matched(item.path(), is_dir))
                .find(|matched| !matched.is_none())
                .is_some_and(|matched| matched.is_ignore());
            if ignored {
                continue;
            }
            entries.push(Entry {
                name: path.name().unwrap_or_default().to_string(),
                path,
                kind,
            });
        }
        sort_entries(&mut entries);
        Ok(entries)
    }

    fn resolve(&self, path: &SnapshotPath) -> Result<SnapshotPath, SnapshotError> {
        // Canonicalised every time rather than joined lexically: a directory on
        // the way may be a symlink the OS would follow out of the root.
        let resolved = self.absolute(path).canonicalize().map_err(io_error)?;
        let inside = relative(&self.root, &resolved).ok_or(SnapshotError::OutsideRoot)?;
        // `.git` holds the repository's configuration, credentials in a remote
        // URL included. A revision has no such path, and a listing hides it.
        if in_git_dir(&inside) {
            return Err(SnapshotError::NotFound);
        }
        // Opening a FIFO would wait for a writer, so only a regular file passes.
        if !resolved.metadata().map_err(io_error)?.is_file() {
            return Err(SnapshotError::NotAFile);
        }
        Ok(inside)
    }

    fn read(&self, path: &SnapshotPath, limit: u64) -> Result<Vec<u8>, SnapshotError> {
        let resolved = self.absolute(&self.resolve(path)?);
        // The file can change between `resolve` and here; reading a local
        // working tree cannot rule that out.
        let file = File::open(resolved).map_err(io_error)?;
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() as u64 > limit {
            return Err(SnapshotError::TooLarge { limit });
        }
        Ok(bytes)
    }

    /// Walked lexically, which is exact for a path `resolve` returned: it
    /// passes through no symlink.
    fn content_root(&self, document: &SnapshotPath) -> SnapshotPath {
        // A nested clone or submodule owns the documents below it.
        let mut dir = document.parent().unwrap_or_default();
        loop {
            if dir.is_root() || has_git_entry(&self.absolute(&dir)) {
                return dir;
            }
            dir = dir.parent().unwrap_or_default();
        }
    }

    fn label(&self) -> String {
        self.root.display().to_string()
    }
}

fn in_git_dir(path: &SnapshotPath) -> bool {
    path.components()
        .any(|part| part.eq_ignore_ascii_case(".git"))
}

/// A `.git` directory in a clone, or a `.git` file in a linked worktree or a
/// submodule.
fn has_git_entry(dir: &Path) -> bool {
    dir.join(".git").symlink_metadata().is_ok()
}

/// `path` as a snapshot path under `root`, or `None` when it is outside or
/// not UTF-8.
fn relative(root: &Path, path: &Path) -> Option<SnapshotPath> {
    let rest = path.strip_prefix(root).ok()?;
    let parts = rest
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?;
    SnapshotPath::new(&parts.join("/")).ok()
}

fn io_error(err: io::Error) -> SnapshotError {
    match err.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => SnapshotError::NotFound,
        // A symlink loop is treated like a link out of the root, as the
        // revision's hop limit is.
        _ if err.raw_os_error() == Some(libc::ELOOP) => SnapshotError::OutsideRoot,
        _ => SnapshotError::Backend(Box::new(err)),
    }
}
