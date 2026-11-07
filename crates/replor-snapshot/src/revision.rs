//! A commit's tree read with gix: objects only, with no filter or textconv,
//! and nothing written (`docs/adr/0001-p0-spike.md`, S4).

use std::collections::VecDeque;
use std::path::Path;

use gix::ObjectId;
use replor_render::snapshot::{
    Entry, EntryKind, Snapshot, SnapshotError, SnapshotPath, sort_entries,
};

use crate::MAX_SYMLINK_HOPS;

#[derive(Debug, Clone)]
pub struct RevisionSnapshot {
    /// Shared between threads; each call takes a thread-local handle.
    repo: gix::ThreadSafeRepository,
    commit: ObjectId,
    tree: ObjectId,
}

type Error = Box<dyn std::error::Error + Send + Sync>;

impl RevisionSnapshot {
    /// The tree of `commit` in the repository at `repository`.
    pub fn open(repository: &Path, commit: ObjectId) -> Result<Self, Error> {
        let repo = gix::open(repository)?;
        let tree = repo.find_commit(commit)?.tree_id()?.detach();
        Ok(Self {
            repo: repo.into_sync(),
            commit,
            tree,
        })
    }

    /// The tree entry at `path`, without following any symlink.
    fn entry(
        &self,
        repo: &gix::Repository,
        path: &SnapshotPath,
    ) -> Result<(gix::object::tree::EntryMode, ObjectId), SnapshotError> {
        let mut mode: gix::object::tree::EntryMode = gix::object::tree::EntryKind::Tree.into();
        let mut id = self.tree;
        for name in path.components() {
            if !mode.is_tree() {
                return Err(SnapshotError::NotFound);
            }
            let tree = repo.find_tree(id).map_err(backend)?;
            let entry = tree.find_entry(name).ok_or(SnapshotError::NotFound)?;
            (mode, id) = (entry.mode(), entry.object_id());
        }
        Ok((mode, id))
    }
}

impl Snapshot for RevisionSnapshot {
    fn list(&self, dir: &SnapshotPath) -> Result<Vec<Entry>, SnapshotError> {
        let repo = self.repo.to_thread_local();
        let (mode, id) = self.entry(&repo, dir)?;
        if mode.is_commit() {
            return Err(SnapshotError::Unsupported);
        }
        if !mode.is_tree() {
            return Err(SnapshotError::NotFound);
        }
        let tree = repo.find_tree(id).map_err(backend)?;
        let mut entries = Vec::new();
        for item in tree.iter() {
            let item = item.map_err(backend)?;
            // The tree may come from someone else's PR head, so a name that is
            // not UTF-8 or would alias another path is left out.
            let Some(path) = std::str::from_utf8(item.filename())
                .ok()
                .and_then(|name| dir.child(name))
            else {
                continue;
            };
            let mode = item.mode();
            let kind = if mode.is_tree() {
                EntryKind::Directory
            } else if mode.is_commit() {
                EntryKind::Submodule
            } else if mode.is_link() {
                EntryKind::Symlink
            } else {
                EntryKind::File
            };
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
        let repo = self.repo.to_thread_local();
        // Walked one component at a time from the root, so a symlink anywhere
        // on the way is followed the way the OS would follow it on disk.
        let mut pending: VecDeque<String> = path.components().map(str::to_string).collect();
        // `done` is the directory reached so far and `tree` its tree, so each
        // component is one lookup rather than a walk from the root.
        let mut done = SnapshotPath::root();
        let mut tree = self.tree;
        let mut hops = 0;
        while let Some(name) = pending.pop_front() {
            let here = done.child(&name).ok_or(SnapshotError::NotFound)?;
            let entry = repo.find_tree(tree).map_err(backend)?;
            let entry = entry
                .find_entry(name.as_str())
                .ok_or(SnapshotError::NotFound)?;
            let (mode, id) = (entry.mode(), entry.object_id());
            if mode.is_link() {
                hops += 1;
                if hops > MAX_SYMLINK_HOPS {
                    return Err(SnapshotError::OutsideRoot);
                }
                let blob = repo.find_blob(id).map_err(backend)?;
                let target =
                    std::str::from_utf8(&blob.data).map_err(|_| SnapshotError::NotFound)?;
                // An absolute target is outside the root, as it would be on
                // disk; a leading `/` only means the content root in Markdown.
                if target.starts_with('/') {
                    return Err(SnapshotError::OutsideRoot);
                }
                let target = done
                    .join_lexical(target)
                    .map_err(|_| SnapshotError::OutsideRoot)?;
                let parts: Vec<&str> = target.components().collect();
                for part in parts.into_iter().rev() {
                    pending.push_front(part.to_string());
                }
                done = SnapshotPath::root();
                tree = self.tree;
            } else if mode.is_commit() {
                return Err(SnapshotError::Unsupported);
            } else if mode.is_tree() {
                if pending.is_empty() {
                    return Err(SnapshotError::NotAFile);
                }
                done = here;
                tree = id;
            } else if pending.is_empty() {
                return Ok(here);
            } else {
                return Err(SnapshotError::NotFound);
            }
        }
        Err(SnapshotError::NotAFile)
    }

    fn read(&self, path: &SnapshotPath, limit: u64) -> Result<Vec<u8>, SnapshotError> {
        let resolved = self.resolve(path)?;
        let repo = self.repo.to_thread_local();
        let (_, id) = self.entry(&repo, &resolved)?;
        // The header gives the size without inflating the blob.
        if repo.find_header(id).map_err(backend)?.size() > limit {
            return Err(SnapshotError::TooLarge { limit });
        }
        Ok(repo.find_blob(id).map_err(backend)?.detach().data)
    }

    /// A nested repository is a submodule here and is not read, so every
    /// document shares the tree's root.
    fn content_root(&self, _document: &SnapshotPath) -> SnapshotPath {
        SnapshotPath::root()
    }

    fn label(&self) -> String {
        self.commit.to_hex_with_len(7).to_string()
    }
}

fn backend(err: impl std::error::Error + Send + Sync + 'static) -> SnapshotError {
    SnapshotError::Backend(Box::new(err))
}
