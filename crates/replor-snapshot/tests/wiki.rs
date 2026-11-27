//! Wiki pages listed from a working tree and from the commit made from it
//! (`docs/design/p2-wiki.md`, "fixture と検証").
//!
//! The fixture is copied into a temporary repository, so it is committed
//! there and the repository holding it is never read or changed.

#![cfg(unix)]

mod common;

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{git, write};
use gix::ObjectId;
use replor_render::snapshot::{Snapshot, SnapshotError, SnapshotPath};
use replor_render::wiki::{self, OrderError, WikiLevel};
use replor_snapshot::{RevisionSnapshot, WorktreeSnapshot};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../replor/tests/fixtures");

struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Repo {
    fn new(build: impl FnOnce(&Path)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        build(&root);
        Self { _dir: dir, root }
    }

    /// A project wiki: the wiki root is the repository's root.
    fn fixture() -> Self {
        Self::copy("azure-wiki")
    }

    /// A code wiki in `docs/wiki/`, below the repository's root.
    fn code_wiki() -> Self {
        Self::copy("code-wiki")
    }

    fn copy(fixture: &str) -> Self {
        Self::new(|root| copy_dir(&Path::new(FIXTURES).join(fixture), root))
    }

    /// The working tree and the commit made from it.
    fn both(&self) -> [(&'static str, Arc<dyn Snapshot>); 2] {
        git(&self.root, &["init", "-q"]);
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-q", "-m", "fixture"]);
        let commit = git(&self.root, &["rev-parse", "HEAD"]);
        let commit = ObjectId::from_hex(commit.trim().as_bytes()).unwrap();
        [
            (
                "worktree",
                Arc::new(WorktreeSnapshot::open(&self.root).unwrap()),
            ),
            (
                "revision",
                Arc::new(RevisionSnapshot::open(&self.root, commit).unwrap()),
            ),
        ]
    }

    /// `folder` listed from both.
    fn levels(&self, folder: &str) -> Vec<(&'static str, WikiLevel)> {
        let folder = SnapshotPath::new(folder).unwrap();
        self.both()
            .into_iter()
            .map(|(label, snapshot)| (label, wiki::list_level(&*snapshot, &folder).unwrap()))
            .collect()
    }
}

fn copy_dir(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The wiki root found from each of `starts`, `-` for none.
fn roots(snapshot: &dyn Snapshot, starts: &[&str]) -> Vec<String> {
    starts
        .iter()
        .map(|start| {
            let start = SnapshotPath::new(start).unwrap();
            match wiki::find_root(snapshot, &start) {
                Some(root) => format!("{start} -> {root}"),
                None => format!("{start} -> -"),
            }
        })
        .collect()
}

/// One line a page, so a failure shows the whole level.
fn pages(level: &WikiLevel) -> Vec<String> {
    let shown = |path: &Option<SnapshotPath>| match path {
        Some(path) => path.to_string(),
        None => "-".to_string(),
    };
    level
        .pages
        .iter()
        .map(|page| {
            format!(
                "{} | {} | {} | {}{}",
                page.key,
                page.name,
                shown(&page.body),
                shown(&page.children),
                if page.missing() { " | missing" } else { "" },
            )
        })
        .collect()
}

#[test]
fn every_folder_of_a_project_wiki_finds_the_repository_root() {
    for (label, snapshot) in Repo::fixture().both() {
        assert_eq!(
            roots(&*snapshot, &["", "Design", "Folder-Only"]),
            [" -> ", "Design -> ", "Folder-Only -> "],
            "{label}"
        );
    }
}

#[test]
fn a_code_wiki_is_found_from_its_folders_only() {
    for (label, snapshot) in Repo::code_wiki().both() {
        assert_eq!(
            roots(&*snapshot, &["docs/wiki/Sub", "docs/wiki", "docs", ""]),
            [
                "docs/wiki/Sub -> docs/wiki",
                "docs/wiki -> docs/wiki",
                "docs -> -",
                " -> -",
            ],
            "{label}"
        );
    }
}

#[test]
fn only_an_order_file_or_an_attachments_folder_marks_a_wiki() {
    let repo = Repo::new(|root| {
        write(&root.join("a/.order/x"), "x");
        write(&root.join("b/.attachments"), "x");
        write(&root.join("c/.attachments/x.png"), "x");
    });
    for (label, snapshot) in repo.both() {
        assert_eq!(
            roots(&*snapshot, &["a", "b", "c"]),
            ["a -> -", "b -> -", "c -> c"],
            "{label}"
        );
    }
}

/// A revision lists a nested clone as a submodule it cannot look into, so
/// only the working tree has a wiki there.
#[test]
fn a_wiki_in_a_nested_clone_stops_at_the_clone() {
    let repo = Repo::new(|root| {
        write(&root.join(".order"), "Home\n");
        write(&root.join("vendor/wiki/.order"), "Page\n");
        write(&root.join("vendor/wiki/Page.md"), "p");
        let vendor = root.join("vendor");
        git(&vendor, &["init", "-q"]);
        git(&vendor, &["add", "-A"]);
        git(&vendor, &["commit", "-q", "-m", "vendor"]);
    });
    let [(_, worktree), _] = repo.both();
    assert_eq!(
        roots(&*worktree, &["vendor/wiki", "vendor", ""]),
        ["vendor/wiki -> vendor/wiki", "vendor -> -", " -> "]
    );
}

#[test]
fn the_root_is_in_order_then_by_name() {
    for (label, level) in Repo::fixture().levels("") {
        assert_eq!(
            pages(&level),
            [
                "Home | Home | Home.md | -",
                "Getting-Started | Getting Started | Getting-Started.md | -",
                "Design | Design | Design.md | Design",
                "Missing-Page | Missing Page | - | - | missing",
                "Folder-Only | Folder Only | - | Folder-Only",
                "Unlisted | Unlisted | Unlisted.md | -",
            ],
            "{label}"
        );
        assert_eq!(level.duplicates, 1, "{label}");
        assert_eq!(level.order_error, None, "{label}");
    }
}

#[test]
fn child_pages_are_named_from_their_file_names() {
    for (label, level) in Repo::fixture().levels("Design") {
        assert_eq!(
            pages(&level),
            [
                "Design/Architecture | Architecture | Design/Architecture.md | -",
                "Design/日本語ページ | 日本語ページ | Design/日本語ページ.md | -",
                "Design/API-%3A-v2 | API : v2 | Design/API-%3A-v2.md | -",
                "Design/FAQ%3A0525 | FAQ:0525 | Design/FAQ%3A0525.md | -",
                "Design/Q%3F-A | Q? A | Design/Q%3F-A.md | -",
                "Design/Set%2Dup-Guide | Set-up Guide | Design/Set%2Dup-Guide.md | -",
            ],
            "{label}"
        );
        assert_eq!(level.duplicates, 0, "{label}");
    }
}

#[test]
fn a_folder_without_order_is_by_name() {
    for (label, level) in Repo::fixture().levels("Folder-Only") {
        assert_eq!(
            pages(&level),
            ["Folder-Only/Child | Child | Folder-Only/Child.md | -"],
            "{label}"
        );
        assert_eq!(level.order_error, None, "{label}");
    }
}

#[test]
fn a_folder_that_cannot_be_listed_is_the_error() {
    let folder = SnapshotPath::new("Nowhere").unwrap();
    for (label, snapshot) in Repo::fixture().both() {
        assert!(wiki::list_level(&*snapshot, &folder).is_err(), "{label}");
    }
}

#[test]
fn only_markdown_files_and_folders_are_pages() {
    let repo = Repo::new(|root| {
        write(&root.join("a.md"), "a");
        write(&root.join("b.txt"), "b");
        write(&root.join("c.png"), "c");
        write(&root.join(".attachments/d.png"), "d");
        write(&root.join("e/f.md"), "f");
        write(&root.join("target.md"), "t");
        symlink("target.md", root.join("linked.md")).unwrap();
        symlink("nowhere.md", root.join("dangling.md")).unwrap();
        symlink("e", root.join("folder-link")).unwrap();
        symlink("e", root.join("folder-link.md")).unwrap();
    });
    for (label, level) in repo.levels("") {
        assert_eq!(
            pages(&level),
            [
                "a | a | a.md | -",
                "e | e | - | e",
                "linked | linked | linked.md | -",
                "target | target | target.md | -",
            ],
            "{label}"
        );
    }
}

#[test]
fn a_nested_repository_is_a_folder_of_pages_in_both() {
    let repo = Repo::new(|root| {
        write(&root.join(".order"), "Vendor\n");
        write(&root.join("Vendor/Page.md"), "p");
        let vendor = root.join("Vendor");
        git(&vendor, &["init", "-q"]);
        git(&vendor, &["add", "-A"]);
        git(&vendor, &["commit", "-q", "-m", "vendor"]);
    });
    for (label, level) in repo.levels("") {
        assert_eq!(pages(&level), ["Vendor | Vendor | - | Vendor"], "{label}");
    }
}

#[test]
fn a_line_that_cannot_name_a_page_is_passed_over() {
    let repo = Repo::new(|root| {
        write(&root.join(".order"), "a/b\n..\n.git\n# note\nPage\n");
        write(&root.join("Page.md"), "p");
    });
    for (label, level) in repo.levels("") {
        assert_eq!(
            pages(&level),
            [
                "# note | # note | - | - | missing",
                "Page | Page | Page.md | -"
            ],
            "{label}"
        );
    }
}

#[test]
fn an_order_that_cannot_be_read_leaves_the_pages_by_name() {
    let large = "b\n".repeat(wiki::MAX_ORDER_BYTES as usize / 2 + 1);
    let too_large = SnapshotError::TooLarge {
        limit: wiki::MAX_ORDER_BYTES,
    }
    .to_string();
    let cases: [(&[u8], OrderError); 2] = [
        (large.as_bytes(), OrderError::Read(too_large)),
        (b"b\n\xE9\n", OrderError::Decode { offset: 2 }),
    ];
    for (order, expected) in cases {
        let repo = Repo::new(|root| {
            write(&root.join(".order"), order);
            write(&root.join("a.md"), "a");
            write(&root.join("b.md"), "b");
        });
        for (label, level) in repo.levels("") {
            assert_eq!(
                pages(&level),
                ["a | a | a.md | -", "b | b | b.md | -"],
                "{label}"
            );
            assert_eq!(level.order_error.as_ref(), Some(&expected), "{label}");
            assert_eq!(level.duplicates, 0, "{label}");
        }
    }
}
