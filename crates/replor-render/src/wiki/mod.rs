//! Azure DevOps Wiki pages: which entries of a folder are pages, the order
//! `.order` gives them, and the names they are shown by. The rules are
//! `docs/design/p2-wiki.md`, "描画コアの `wiki` module".
//!
//! Everything is read through the Snapshot one folder at a time, so a working
//! tree and a revision list the same pages. Which rows are expanded or
//! selected belongs to the Wiki lens, not here.

mod name;
mod order;

pub use name::page_name;

use std::collections::BTreeMap;

use crate::snapshot::{EntryKind, Snapshot, SnapshotError, SnapshotPath};

/// Largest `.order` read.
pub const MAX_ORDER_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiPage {
    /// The name shown for the page (`page_name`).
    pub name: String,
    /// The page's path without `.md`, unique in its folder.
    pub key: SnapshotPath,
    /// `Page.md`. A page that is only a folder has none.
    pub body: Option<SnapshotPath>,
    /// `Page/`, the folder holding the page's child pages.
    pub children: Option<SnapshotPath>,
}

impl WikiPage {
    /// Named in `.order` with neither a body nor a folder.
    pub fn missing(&self) -> bool {
        self.body.is_none() && self.children.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLevel {
    pub pages: Vec<WikiPage>,
    /// Names `.order` gives again after their first line.
    pub duplicates: usize,
    /// Why `.order` could not be read. The pages are then in the order a
    /// folder without `.order` has.
    pub order_error: Option<OrderError>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OrderError {
    #[error(".order: {0}")]
    Read(String),
    #[error(".order is not UTF-8 (byte {offset})")]
    Decode { offset: usize },
}

/// The wiki `start` is in: the highest folder from `start` up to its content
/// root that holds a `.order` file or an `.attachments` folder, so every
/// folder of one wiki finds the same root. Nothing below `start` is looked
/// at, and nothing above the content root, which a nested clone owns.
pub fn find_root(snapshot: &dyn Snapshot, start: &SnapshotPath) -> Option<SnapshotPath> {
    // `content_root` is asked about a document; any name in `start` stands
    // for one there.
    let boundary = start
        .child(".order")
        .map(|document| snapshot.content_root(&document))
        .unwrap_or_default();
    let mut found = None;
    let mut dir = Some(start.clone());
    while let Some(at) = dir.filter(|at| at.starts_with(&boundary)) {
        if marks_a_wiki(snapshot, &at) {
            found = Some(at.clone());
        }
        dir = at.parent();
    }
    found
}

fn marks_a_wiki(snapshot: &dyn Snapshot, dir: &SnapshotPath) -> bool {
    snapshot.list(dir).is_ok_and(|entries| {
        entries.iter().any(|entry| match entry.name.as_str() {
            ".order" => entry.kind == EntryKind::File,
            ".attachments" => entry.kind == EntryKind::Directory,
            _ => false,
        })
    })
}

/// The pages of `folder`: those `.order` names first, in its order, then the
/// rest by the UTF-8 bytes of their names without `.md`, folders and files
/// together. Fails only when the folder itself cannot be listed.
pub fn list_level(
    snapshot: &dyn Snapshot,
    folder: &SnapshotPath,
) -> Result<WikiLevel, SnapshotError> {
    // Keyed by the file name without `.md`, so `Page.md` and `Page/` meet.
    let mut found: BTreeMap<String, (Option<SnapshotPath>, Option<SnapshotPath>)> = BTreeMap::new();
    let mut order_file = None;
    for entry in snapshot.list(folder)? {
        match entry.kind {
            // A working tree lists a nested repository as a directory and a
            // revision as a submodule; both are the same folder of pages.
            EntryKind::Directory | EntryKind::Submodule if entry.name != ".attachments" => {
                found.entry(entry.name).or_default().1 = Some(entry.path);
            }
            EntryKind::File | EntryKind::Symlink if entry.name == ".order" => {
                order_file = Some(entry.path);
            }
            EntryKind::File | EntryKind::Symlink => {
                let Some(stem) = entry.name.strip_suffix(".md") else {
                    continue;
                };
                // A symlink is a body only when it ends at a file, as the
                // Files lens opens it; a folder behind one is never listed.
                // Any other failure keeps the page, so opening it says why.
                if entry.kind == EntryKind::Symlink
                    && matches!(
                        snapshot.resolve(&entry.path),
                        Err(SnapshotError::NotFound
                            | SnapshotError::NotAFile
                            | SnapshotError::OutsideRoot
                            | SnapshotError::Unsupported)
                    )
                {
                    continue;
                }
                found.entry(stem.to_string()).or_default().0 = Some(entry.path);
            }
            _ => {}
        }
    }

    let order = order_file.map(|path| read_order(snapshot, &path));
    let (names, duplicates, order_error) = match &order {
        None => (Vec::new(), 0, None),
        Some(Ok(text)) => {
            let (names, duplicates) = order::parse(text);
            (names, duplicates, None)
        }
        Some(Err(err)) => (Vec::new(), 0, Some(err.clone())),
    };

    let mut pages = Vec::new();
    for stem in names {
        // A line that cannot be a file name names no page, not even a
        // missing one.
        let Some(key) = folder.child(stem) else {
            continue;
        };
        let (body, children) = found.remove(stem).unwrap_or_default();
        pages.push(page(stem, key, body, children));
    }
    for (stem, (body, children)) in found {
        if let Some(key) = folder.child(&stem) {
            pages.push(page(&stem, key, body, children));
        }
    }
    Ok(WikiLevel {
        pages,
        duplicates,
        order_error,
    })
}

fn page(
    stem: &str,
    key: SnapshotPath,
    body: Option<SnapshotPath>,
    children: Option<SnapshotPath>,
) -> WikiPage {
    WikiPage {
        name: page_name(stem),
        key,
        body,
        children,
    }
}

fn read_order(snapshot: &dyn Snapshot, path: &SnapshotPath) -> Result<String, OrderError> {
    let bytes = snapshot
        .resolve(path)
        .and_then(|resolved| snapshot.read(&resolved, MAX_ORDER_BYTES))
        .map_err(|err| OrderError::Read(err.to_string()))?;
    String::from_utf8(bytes).map_err(|err| OrderError::Decode {
        offset: err.utf8_error().valid_up_to(),
    })
}
