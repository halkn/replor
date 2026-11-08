//! The file tree in the left pane.
//!
//! Directories are listed one level at a time, when they are expanded, so a
//! large repository opens without walking all of it. Everything is read
//! through the Snapshot, so the tree does not know whether it shows a working
//! tree or a revision.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use replor_render::snapshot::{Entry, EntryKind, Snapshot, SnapshotPath};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub entry: Entry,
    pub depth: usize,
    pub expanded: bool,
}

/// What opening the selected row did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    File(Entry),
    Toggled,
    Nothing,
}

pub struct Tree {
    snapshot: Arc<dyn Snapshot>,
    /// The directory the tree opens at. Not a boundary: the snapshot's root is.
    top: SnapshotPath,
    expanded: HashSet<SnapshotPath>,
    listings: HashMap<SnapshotPath, Result<Vec<Entry>, String>>,
    rows: Vec<Row>,
    selected: usize,
    /// First row on screen.
    pub scroll: usize,
}

impl Tree {
    pub fn new(snapshot: Arc<dyn Snapshot>, top: SnapshotPath) -> Self {
        let mut tree = Self {
            snapshot,
            top,
            expanded: HashSet::new(),
            listings: HashMap::new(),
            rows: Vec::new(),
            selected: 0,
            scroll: 0,
        };
        tree.rebuild();
        tree
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn top(&self) -> &SnapshotPath {
        &self.top
    }

    /// Why the top directory could not be listed, if it could not.
    pub fn error(&self) -> Option<&str> {
        self.listings
            .get(&self.top)
            .and_then(|listing| listing.as_ref().err())
            .map(String::as_str)
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    /// Selects the row showing `path`, expanding the directories above it.
    pub fn select(&mut self, path: &SnapshotPath) {
        let mut dir = path.parent();
        while let Some(at) = dir {
            if !at.starts_with(&self.top) || at == self.top {
                break;
            }
            self.expanded.insert(at.clone());
            dir = at.parent();
        }
        self.rebuild();
        if let Some(index) = self.rows.iter().position(|row| &row.entry.path == path) {
            self.selected = index;
        }
    }

    /// Expands the selected directory, or hands over the selected file as
    /// `open` does. An expanded directory stays as it is.
    pub fn expand(&mut self) -> Opened {
        let Some(row) = self.rows.get(self.selected) else {
            return Opened::Nothing;
        };
        match (row.entry.kind, row.expanded) {
            (EntryKind::Directory, true) => Opened::Nothing,
            (EntryKind::Directory, false) => {
                self.expanded.insert(row.entry.path.clone());
                self.rebuild();
                Opened::Toggled
            }
            _ => Opened::File(row.entry.clone()),
        }
    }

    /// Collapses the selected directory, or, from anything else, moves to
    /// the directory holding it, so repeated presses walk up the tree.
    pub fn collapse(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        if row.entry.kind == EntryKind::Directory && row.expanded {
            let path = row.entry.path.clone();
            self.expanded.remove(&path);
            self.rebuild();
            return;
        }
        let depth = row.depth;
        if let Some(parent) = self.rows[..self.selected]
            .iter()
            .rposition(|above| above.depth + 1 == depth)
        {
            self.selected = parent;
        }
    }

    pub fn open(&mut self) -> Opened {
        let Some(row) = self.rows.get(self.selected) else {
            return Opened::Nothing;
        };
        match row.entry.kind {
            EntryKind::Directory => {
                let path = row.entry.path.clone();
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
                self.rebuild();
                Opened::Toggled
            }
            _ => Opened::File(row.entry.clone()),
        }
    }

    /// Lists every directory again, keeping what is expanded and selected.
    pub fn reload(&mut self) {
        let selected = self
            .rows
            .get(self.selected)
            .map(|row| row.entry.path.clone());
        self.listings.clear();
        self.rebuild();
        if let Some(path) = selected {
            match self.rows.iter().position(|row| row.entry.path == path) {
                Some(index) => self.selected = index,
                None => self.move_by(0),
            }
        }
    }

    /// Keeps the selection on screen in a pane `height` rows tall.
    pub fn follow(&mut self, height: usize) {
        let height = height.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(height));
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let top = self.top.clone();
        self.push_children(&top, 0, &mut rows);
        self.rows = rows;
        self.move_by(0);
    }

    fn push_children(&mut self, dir: &SnapshotPath, depth: usize, rows: &mut Vec<Row>) {
        let snapshot = Arc::clone(&self.snapshot);
        let listing = self
            .listings
            .entry(dir.clone())
            .or_insert_with(|| snapshot.list(dir).map_err(|err| err.to_string()))
            .clone();
        let Ok(entries) = listing else {
            return;
        };
        for entry in entries {
            let expanded =
                entry.kind == EntryKind::Directory && self.expanded.contains(&entry.path);
            let path = entry.path.clone();
            rows.push(Row {
                entry,
                depth,
                expanded,
            });
            if expanded {
                self.push_children(&path, depth + 1, rows);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replor_snapshot::WorktreeSnapshot;
    use std::path::Path;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        write(&dir.path().join(".gitignore"), "target/\n*.log\n");
        write(&dir.path().join("target/debug/out.md"), "x");
        write(&dir.path().join("run.log"), "x");
        write(&dir.path().join("docs/guide/intro.md"), "# Intro\n");
        write(&dir.path().join("docs/index.md"), "# Index\n");
        write(&dir.path().join("README.md"), "# R\n");
        write(&dir.path().join("Cargo.toml"), "[package]\n");
        dir
    }

    fn tree(dir: &Path, top: &str) -> Tree {
        let snapshot = WorktreeSnapshot::open(dir).unwrap();
        Tree::new(Arc::new(snapshot), SnapshotPath::new(top).unwrap())
    }

    fn names(tree: &Tree) -> Vec<String> {
        tree.rows()
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), row.entry.name))
            .collect()
    }

    #[test]
    fn gitignored_paths_are_not_in_the_tree() {
        let dir = fixture();
        let tree = tree(dir.path(), "");
        assert_eq!(
            names(&tree),
            ["docs", ".gitignore", "Cargo.toml", "README.md"]
        );
    }

    #[test]
    fn a_directory_expands_and_collapses_in_place() {
        let dir = fixture();
        let mut tree = tree(dir.path(), "");
        assert_eq!(tree.open(), Opened::Toggled);
        assert_eq!(
            names(&tree)[..3],
            ["docs", "  guide", "  index.md"].map(String::from)
        );
        tree.move_by(1);
        tree.open();
        assert!(names(&tree).contains(&"    intro.md".to_string()));
        tree.move_by(-1);
        tree.open();
        assert_eq!(
            names(&tree),
            ["docs", ".gitignore", "Cargo.toml", "README.md"]
        );
    }

    #[test]
    fn h_and_l_collapse_and_expand_and_h_walks_up() {
        let dir = fixture();
        let mut tree = tree(dir.path(), "");
        tree.expand();
        tree.expand();
        assert_eq!(
            names(&tree)[..3],
            ["docs", "  guide", "  index.md"].map(String::from)
        );
        tree.move_by(1);
        tree.expand();
        tree.move_by(1);
        assert_eq!(tree.rows()[tree.selected()].entry.name, "intro.md");
        // A file: up to its directory, then that directory closes, then up again.
        tree.collapse();
        assert_eq!(tree.rows()[tree.selected()].entry.name, "guide");
        tree.collapse();
        assert!(!names(&tree).contains(&"    intro.md".to_string()));
        tree.collapse();
        assert_eq!(tree.rows()[tree.selected()].entry.name, "docs");
        tree.collapse();
        assert_eq!(
            names(&tree),
            ["docs", ".gitignore", "Cargo.toml", "README.md"]
        );
        // At the top level there is nowhere further up.
        tree.collapse();
        assert_eq!(tree.selected(), 0);
        // `l` on an open directory leaves it open; on a file it opens the file.
        tree.expand();
        assert_eq!(tree.expand(), Opened::Nothing);
        assert!(names(&tree).contains(&"  guide".to_string()));
        tree.move_by(2);
        let Opened::File(entry) = tree.expand() else {
            panic!("expected a file");
        };
        assert_eq!(entry.path.as_str(), "docs/index.md");
    }

    #[test]
    fn opening_a_file_hands_it_over() {
        let dir = fixture();
        let mut tree = tree(dir.path(), "docs");
        assert_eq!(names(&tree), ["guide", "index.md"]);
        tree.move_by(5);
        let Opened::File(entry) = tree.open() else {
            panic!("expected a file");
        };
        assert_eq!(entry.path.as_str(), "docs/index.md");
    }

    #[test]
    fn select_expands_the_way_to_a_file() {
        let dir = fixture();
        let mut tree = tree(dir.path(), "docs");
        tree.select(&SnapshotPath::new("docs/guide/intro.md").unwrap());
        assert_eq!(tree.rows()[tree.selected()].entry.name, "intro.md");
    }

    #[test]
    fn reload_keeps_the_selection_and_shows_new_files() {
        let dir = fixture();
        let mut tree = tree(dir.path(), "docs");
        tree.move_by(1);
        write(&dir.path().join("docs/added.md"), "new");
        tree.reload();
        assert_eq!(names(&tree), ["guide", "added.md", "index.md"]);
        assert_eq!(tree.rows()[tree.selected()].entry.name, "index.md");
    }

    #[test]
    fn the_selection_stays_on_screen() {
        let dir = fixture();
        let mut tree = tree(dir.path(), "");
        tree.move_by(3);
        tree.follow(2);
        assert_eq!(tree.scroll, 2);
        tree.move_by(-3);
        tree.follow(2);
        assert_eq!(tree.scroll, 0);
    }
}
