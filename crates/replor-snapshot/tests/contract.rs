//! The same fixture read as a working tree and as the commit made from it must
//! give the same results (`docs/design/p1-core.md`, "symlink と境界").
//!
//! Each test builds its own repository in a temporary directory, so no other
//! repository is read or changed.

#![cfg(unix)]

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use gix::ObjectId;
use replor_render::diagram::{MermaidMode, MermaidWidth};
use replor_render::document;
use replor_render::flavor::Flavor;
use replor_render::image;
use replor_render::layout::icons::IconSet;
use replor_render::layout::theme::Variant;
use replor_render::output::ColorChoice;
use replor_render::snapshot::{
    DocumentSource, EntryKind, Snapshot, SnapshotError, SnapshotPath, WalkLimits, Walked, walk,
};
use replor_render::{RenderOptions, render_to_string};
use replor_snapshot::{RevisionSnapshot, WorktreeSnapshot};

const TEXT: &str = "# Title\n\nBody with ![a](.attachments/a.png).\n";

fn png() -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&20u32.to_be_bytes());
    out.extend_from_slice(&10u32.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0]);
    out
}

fn git(dir: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    // Keep the caller's Git environment out of the fixture: `GIT_DIR` and
    // friends would point the commands at another repository, and
    // `GIT_CONFIG_*` would bring in its configuration and hooks.
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(|key| key.starts_with("GIT_")) {
            command.env_remove(key);
        }
    }
    let out = command
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "replor")
        .env("GIT_AUTHOR_EMAIL", "replor@example.invalid")
        .env("GIT_COMMITTER_NAME", "replor")
        .env("GIT_COMMITTER_EMAIL", "replor@example.invalid")
        .output()
        .expect("git should run");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("git prints UTF-8")
}

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

struct Fixture {
    /// Holds the temporary directory for the life of the test.
    _dir: tempfile::TempDir,
    repo: PathBuf,
    commit: ObjectId,
}

impl Fixture {
    /// `outside.png` sits beside the repository, where no read may reach.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        write(&dir.path().join("outside.png"), png());

        write(&repo.join("README.md"), TEXT);
        write(
            &repo.join("bom.md"),
            [b"\xEF\xBB\xBF".as_slice(), TEXT.as_bytes()].concat(),
        );
        write(&repo.join(".attachments/a.png"), png());
        write(&repo.join("docs/page.md"), "# Page\n");
        write(&repo.join("Z/z.md"), "z");
        write(&repo.join("B.md"), "b");
        write(&repo.join("a.md"), "a");
        write(&repo.join(".hidden.md"), "hidden");
        write(&repo.join(".gitignore"), "ignored.md\n");
        write(&repo.join("ignored.md"), "ignored");
        let link = |target: &str, name: &str| symlink(target, repo.join(name)).unwrap();
        link(".attachments/a.png", "file-link.png");
        link(".attachments", "dir-link");
        link("../outside.png", "relative-out.png");
        link(
            dir.path().join("outside.png").to_str().unwrap(),
            "absolute-out.png",
        );
        link("loop-b.png", "loop-a.png");
        link("loop-a.png", "loop-b.png");
        link("../.attachments/a.png", "docs/up-link.png");
        link(".git/config", "git-config.md");

        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "fixture"]);
        let commit = git(&repo, &["rev-parse", "HEAD"]);
        let commit = ObjectId::from_hex(commit.trim().as_bytes()).unwrap();
        Self {
            _dir: dir,
            repo,
            commit,
        }
    }

    fn worktree(&self) -> Arc<dyn Snapshot> {
        Arc::new(WorktreeSnapshot::open(&self.repo).unwrap())
    }

    fn revision(&self) -> Arc<dyn Snapshot> {
        Arc::new(RevisionSnapshot::open(&self.repo, self.commit).unwrap())
    }

    fn both(&self) -> [(&'static str, Arc<dyn Snapshot>); 2] {
        [("worktree", self.worktree()), ("revision", self.revision())]
    }

    /// What a read could change: the working tree, the index, HEAD and refs.
    fn git_state(&self) -> (String, String, Vec<u8>, String) {
        (
            git(&self.repo, &["status", "--porcelain=v2"]),
            git(&self.repo, &["rev-parse", "HEAD"]),
            std::fs::read(self.repo.join(".git/index")).unwrap(),
            git(&self.repo, &["for-each-ref"]),
        )
    }
}

fn path(text: &str) -> SnapshotPath {
    SnapshotPath::new(text).unwrap()
}

/// A result reduced to something both implementations can be compared on.
fn outcome(result: Result<SnapshotPath, SnapshotError>) -> String {
    match result {
        Ok(path) => format!("ok {path}"),
        Err(SnapshotError::Backend(err)) => panic!("backend error: {err}"),
        Err(err) => format!("{err:?}"),
    }
}

fn names(snapshot: &dyn Snapshot, dir: &str) -> Vec<(String, EntryKind)> {
    snapshot
        .list(&path(dir))
        .unwrap()
        .into_iter()
        .map(|e| (e.name, e.kind))
        .collect()
}

#[test]
fn both_list_the_same_entries_in_the_same_order() {
    let fixture = Fixture::new();
    let [(_, worktree), (_, revision)] = fixture.both();
    let expected = [
        (".attachments", EntryKind::Directory),
        ("Z", EntryKind::Directory),
        ("docs", EntryKind::Directory),
        (".gitignore", EntryKind::File),
        (".hidden.md", EntryKind::File),
        ("B.md", EntryKind::File),
        ("README.md", EntryKind::File),
        ("a.md", EntryKind::File),
        ("absolute-out.png", EntryKind::Symlink),
        ("bom.md", EntryKind::File),
        ("dir-link", EntryKind::Symlink),
        ("file-link.png", EntryKind::Symlink),
        ("git-config.md", EntryKind::Symlink),
        ("loop-a.png", EntryKind::Symlink),
        ("loop-b.png", EntryKind::Symlink),
        ("relative-out.png", EntryKind::Symlink),
    ]
    .map(|(name, kind)| (name.to_string(), kind));
    assert_eq!(names(&*worktree, ""), expected);
    assert_eq!(names(&*revision, ""), expected);
    assert_eq!(names(&*worktree, "docs"), names(&*revision, "docs"));
}

#[test]
fn both_resolve_symlinks_and_boundaries_alike() {
    let fixture = Fixture::new();
    let cases = [
        ("README.md", "ok README.md"),
        ("file-link.png", "ok .attachments/a.png"),
        ("dir-link/a.png", "ok .attachments/a.png"),
        ("docs/up-link.png", "ok .attachments/a.png"),
        ("relative-out.png", "OutsideRoot"),
        ("absolute-out.png", "OutsideRoot"),
        ("loop-a.png", "OutsideRoot"),
        ("docs", "NotAFile"),
        ("dir-link", "NotAFile"),
        ("missing.md", "NotFound"),
        ("README.md/x", "NotFound"),
        // The repository's own files are never part of a snapshot.
        ("git-config.md", "NotFound"),
    ];
    for (name, snapshot) in fixture.both() {
        for (target, expected) in cases {
            assert_eq!(
                outcome(snapshot.resolve(&path(target))),
                expected,
                "{name}: {target}"
            );
        }
    }
}

#[test]
fn both_read_the_same_bytes_within_the_limit() {
    let fixture = Fixture::new();
    for (name, snapshot) in fixture.both() {
        assert_eq!(
            snapshot.read(&path("README.md"), 1024).unwrap(),
            TEXT.as_bytes(),
            "{name}"
        );
        assert_eq!(
            snapshot.read(&path("file-link.png"), 1024).unwrap(),
            png(),
            "{name}"
        );
        let limit = TEXT.len() as u64;
        assert!(snapshot.read(&path("README.md"), limit).is_ok(), "{name}");
        assert!(
            matches!(
                snapshot.read(&path("README.md"), limit - 1),
                Err(SnapshotError::TooLarge { .. })
            ),
            "{name}"
        );
    }
}

#[test]
fn both_show_the_same_images_to_a_document() {
    let fixture = Fixture::new();
    let cases = [
        (".attachments/a.png", true),
        ("/.attachments/a.png", true),
        ("file-link.png", true),
        ("dir-link/a.png", true),
        ("relative-out.png", false),
        ("absolute-out.png", false),
        ("loop-a.png", false),
        ("../outside.png", false),
        ("/../outside.png", false),
        ("/.git/config", false),
    ];
    for (name, snapshot) in fixture.both() {
        let source = DocumentSource::new(snapshot, path("README.md"));
        for (dest, shown) in cases {
            assert_eq!(
                image::load(dest, Some(&source)).is_some(),
                shown,
                "{name}: {dest}"
            );
        }
    }
}

fn rendered(snapshot: Arc<dyn Snapshot>, document: &str) -> String {
    let loaded = document::load(snapshot, path(document)).unwrap();
    let options = RenderOptions {
        base_dir: Some(loaded.base_dir),
        width: 80,
        flavor: Flavor::Gfm,
        mermaid: MermaidMode::Unicode,
        mermaid_width: MermaidWidth::Unicode,
        icons: IconSet::Unicode,
        color: ColorChoice::Plain,
        variant: Variant::Dark,
        highlight: false,
        hyperlinks: false,
    };
    render_to_string(loaded.text, &options).unwrap()
}

#[test]
fn a_bom_renders_like_no_bom_in_both() {
    let fixture = Fixture::new();
    for (name, snapshot) in fixture.both() {
        let plain = rendered(Arc::clone(&snapshot), "README.md");
        assert!(plain.starts_with("# Title"), "{name}: {plain}");
        assert_eq!(rendered(snapshot, "bom.md"), plain, "{name}");
    }
}

#[test]
fn a_document_that_is_not_utf8_is_an_error_in_both() {
    let fixture = Fixture::new();
    write(&fixture.repo.join("latin1.md"), b"caf\xE9\n");
    git(&fixture.repo, &["add", "latin1.md"]);
    git(&fixture.repo, &["commit", "-q", "-m", "latin1"]);
    let commit = git(&fixture.repo, &["rev-parse", "HEAD"]);
    let commit = ObjectId::from_hex(commit.trim().as_bytes()).unwrap();
    let revision: Arc<dyn Snapshot> =
        Arc::new(RevisionSnapshot::open(&fixture.repo, commit).unwrap());
    for snapshot in [fixture.worktree(), revision] {
        let err = document::load(snapshot, path("latin1.md")).unwrap_err();
        assert!(
            matches!(
                err,
                replor_render::error::AppError::Decode { offset: 3, .. }
            ),
            "{err:?}"
        );
    }
}

#[test]
fn reading_changes_nothing_in_the_repository() {
    let fixture = Fixture::new();
    let before = fixture.git_state();
    for (_, snapshot) in fixture.both() {
        snapshot.list(&SnapshotPath::root()).unwrap();
        snapshot.read(&path("README.md"), 1024).unwrap();
        let _ = snapshot.resolve(&path("loop-a.png"));
        rendered(snapshot, "bom.md");
    }
    assert_eq!(fixture.git_state(), before);
}

fn walked(snapshot: &dyn Snapshot, limits: WalkLimits) -> (Vec<String>, Walked) {
    let mut paths = Vec::new();
    let walked = walk(snapshot, limits, |entry| {
        paths.push(entry.path.to_string());
        true
    });
    (paths, walked)
}

const NO_LIMIT: WalkLimits = WalkLimits {
    entries: usize::MAX,
    depth: usize::MAX,
};

/// Symlinks to directories, inside, outside and looping, are handed over as
/// entries and never followed, so the walk ends inside the root.
#[test]
fn both_walk_the_same_entries_and_follow_no_symlink() {
    let fixture = Fixture::new();
    let outside = fixture.repo.parent().unwrap();
    write(&outside.join("away/far.md"), "far");
    symlink(outside.join("away"), fixture.repo.join("away-link")).unwrap();
    symlink(".", fixture.repo.join("self-link")).unwrap();
    symlink("..", fixture.repo.join("docs/up")).unwrap();
    git(&fixture.repo, &["add", "-A"]);
    git(&fixture.repo, &["commit", "-q", "-m", "directory links"]);
    let commit = git(&fixture.repo, &["rev-parse", "HEAD"]);
    let commit = ObjectId::from_hex(commit.trim().as_bytes()).unwrap();
    let revision = RevisionSnapshot::open(&fixture.repo, commit).unwrap();

    let (paths, done) = walked(&*fixture.worktree(), NO_LIMIT);
    assert_eq!(done, Walked::Complete);
    assert_eq!(walked(&revision, NO_LIMIT), (paths.clone(), done));
    for expected in ["away-link", "self-link", "docs/up", "loop-a.png", "Z/z.md"] {
        assert!(
            paths.contains(&expected.to_string()),
            "{expected}: {paths:?}"
        );
    }
    assert!(
        !paths.iter().any(|path| path.contains("far.md")),
        "{paths:?}"
    );
    assert!(
        !paths.iter().any(|path| path.contains("ignored")),
        "{paths:?}"
    );
    // Shallowest first, so a partial walk still has the top of the tree.
    let depth = |path: &String| path.matches('/').count();
    assert!(paths.is_sorted_by_key(depth), "{paths:?}");
}

#[test]
fn a_walk_past_its_limits_says_it_is_partial() {
    let fixture = Fixture::new();
    let snapshot = fixture.worktree();
    let (all, _) = walked(&*snapshot, NO_LIMIT);
    let few = WalkLimits {
        entries: 3,
        ..NO_LIMIT
    };
    assert_eq!(
        walked(&*snapshot, few),
        (all[..3].to_vec(), Walked::Partial)
    );
    let exact = WalkLimits {
        entries: all.len(),
        ..NO_LIMIT
    };
    assert_eq!(walked(&*snapshot, exact), (all.clone(), Walked::Complete));
    // An empty directory past the depth hides nothing.
    std::fs::create_dir(fixture.repo.join("Z/empty")).unwrap();
    let level = WalkLimits {
        depth: 1,
        ..NO_LIMIT
    };
    assert_eq!(walked(&*snapshot, level), (all.clone(), Walked::Complete));
    let shallow = WalkLimits {
        depth: 0,
        ..NO_LIMIT
    };
    let (top, done) = walked(&*snapshot, shallow);
    assert_eq!(done, Walked::Partial);
    assert!(top.iter().all(|path| !path.contains('/')), "{top:?}");
    assert!(top.contains(&"README.md".to_string()), "{top:?}");
}

#[test]
fn a_walk_stops_when_asked() {
    let fixture = Fixture::new();
    let mut seen = 0;
    let done = walk(&*fixture.worktree(), NO_LIMIT, |_| {
        seen += 1;
        seen < 2
    });
    assert_eq!((done, seen), (Walked::Stopped, 2));
}

/// A nested clone owns the documents below it: they may not reach the outer
/// repository's attachments.
#[test]
fn a_nested_git_directory_bounds_the_documents_below_it_in_a_worktree() {
    let fixture = Fixture::new();
    let nested = fixture.repo.join("docs/vendored");
    std::fs::create_dir_all(nested.join(".git")).unwrap();
    write(&nested.join("page.md"), "# Vendored\n");
    write(&nested.join("own.png"), png());
    let snapshot = fixture.worktree();
    assert_eq!(
        snapshot.content_root(&path("docs/vendored/page.md")),
        path("docs/vendored")
    );
    assert_eq!(
        snapshot.content_root(&path("docs/page.md")),
        SnapshotPath::root()
    );
    let source = DocumentSource::new(Arc::clone(&snapshot), path("docs/vendored/page.md"));
    assert!(image::load("own.png", Some(&source)).is_some());
    assert!(image::load("/own.png", Some(&source)).is_some());
    assert!(image::load("../../.attachments/a.png", Some(&source)).is_none());
    // Listed as a directory with its files readable; its `.git` is not listed.
    assert!(names(&*snapshot, "docs").contains(&("vendored".to_string(), EntryKind::Directory)));
    assert!(
        !names(&*snapshot, "docs/vendored")
            .iter()
            .any(|(n, _)| n == ".git")
    );
}

/// A linked worktree or a submodule records `.git` as a file.
#[test]
fn a_git_file_marks_the_root_and_the_content_root() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(".git"), "gitdir: /elsewhere\n");
    write(&dir.path().join("docs/sub/.git"), "gitdir: /elsewhere\n");
    let snapshot = WorktreeSnapshot::open(&dir.path().join("docs")).unwrap();
    assert_eq!(snapshot.root(), dir.path().canonicalize().unwrap());
    assert_eq!(snapshot.start(), &path("docs"));
    assert_eq!(
        snapshot.content_root(&path("docs/sub/a.md")),
        path("docs/sub")
    );
}

/// Outside a repository the start is the root, so an image elsewhere below it
/// is readable but nothing above it is.
#[test]
fn outside_a_repository_the_start_is_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let start = dir.path().join("notes");
    write(&dir.path().join("above.png"), png());
    write(&start.join("shared/a.png"), png());
    write(&start.join("pages/p.md"), "p");
    let snapshot: Arc<dyn Snapshot> = Arc::new(WorktreeSnapshot::open(&start).unwrap());
    let source = DocumentSource::new(snapshot, path("pages/p.md"));
    assert!(image::load("../shared/a.png", Some(&source)).is_some());
    assert!(image::load("/shared/a.png", Some(&source)).is_some());
    assert!(image::load("../../above.png", Some(&source)).is_none());
}

/// Opening a FIFO would wait for a writer, so it is refused before any read.
#[test]
fn a_fifo_is_not_a_file_in_a_worktree() {
    let fixture = Fixture::new();
    let status = Command::new("mkfifo")
        .arg(fixture.repo.join("pipe.png"))
        .status()
        .unwrap();
    assert!(status.success());
    let snapshot = fixture.worktree();
    assert_eq!(outcome(snapshot.resolve(&path("pipe.png"))), "NotAFile");
    assert!(names(&*snapshot, "").contains(&("pipe.png".to_string(), EntryKind::Other)));
    let source = DocumentSource::new(snapshot, path("README.md"));
    assert!(image::load("pipe.png", Some(&source)).is_none());
}

/// A gitlink is listed but never read: its commit is another repository's.
#[test]
fn a_submodule_is_listed_but_not_read_in_a_revision() {
    let fixture = Fixture::new();
    let head = fixture.commit.to_string();
    git(
        &fixture.repo,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},sub"),
        ],
    );
    git(&fixture.repo, &["commit", "-q", "-m", "submodule"]);
    let commit = git(&fixture.repo, &["rev-parse", "HEAD"]);
    let commit = ObjectId::from_hex(commit.trim().as_bytes()).unwrap();
    let snapshot = RevisionSnapshot::open(&fixture.repo, commit).unwrap();
    assert!(names(&snapshot, "").contains(&("sub".to_string(), EntryKind::Submodule)));
    assert_eq!(outcome(snapshot.resolve(&path("sub/a.md"))), "Unsupported");
    assert!(matches!(
        snapshot.list(&path("sub")),
        Err(SnapshotError::Unsupported)
    ));
    assert_eq!(
        snapshot.content_root(&path("sub/a.md")),
        SnapshotPath::root()
    );
    assert_eq!(snapshot.label(), commit.to_hex_with_len(7).to_string());
}

/// A `.gitignore` above the root is not the repository's, and one in an
/// outer repository does not reach into a nested clone.
#[test]
fn only_the_repository_own_gitignore_files_hide_entries() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(".gitignore"), "*.md\n");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    write(&repo.join("a.md"), "a");
    write(&repo.join(".gitignore"), "*.log\n");
    write(&repo.join("x.log"), "x");
    write(&repo.join("vendored/.git/HEAD"), "");
    write(&repo.join("vendored/y.log"), "y");
    let snapshot = WorktreeSnapshot::open(&repo).unwrap();
    let listed = |dir: &str| -> Vec<String> {
        names(&snapshot, dir)
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    };
    assert_eq!(listed(""), ["vendored", ".gitignore", "a.md"]);
    assert_eq!(listed("vendored"), ["y.log"]);
}

/// Git reads such a line as a literal pattern; the rest of the file still
/// applies and the listing does not fail.
#[test]
fn a_gitignore_line_that_cannot_be_parsed_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    write(&dir.path().join(".gitignore"), "{a\n*.log\n");
    write(&dir.path().join("docs/a.md"), "a");
    write(&dir.path().join("docs/x.log"), "x");
    let snapshot = WorktreeSnapshot::open(dir.path()).unwrap();
    assert_eq!(
        names(&snapshot, "docs"),
        [("a.md".to_string(), EntryKind::File)]
    );
}

/// Opened on a file outside any repository, the file's directory is the
/// root and the file is where the tree starts.
#[test]
fn a_file_outside_a_repository_opens_in_its_directory() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("note.md"), "# Note\n");
    write(&dir.path().join("a.png"), png());
    let snapshot = WorktreeSnapshot::open(&dir.path().join("note.md")).unwrap();
    assert_eq!(snapshot.root(), dir.path().canonicalize().unwrap());
    assert_eq!(snapshot.start(), &path("note.md"));
    let loaded = document::load(Arc::new(snapshot), path("note.md")).unwrap();
    assert!(image::load("a.png", Some(&loaded.source)).is_some());
}

/// A document opened through a symlink reads its images from where it
/// really is, under that directory's content root.
#[test]
fn a_document_behind_a_symlink_uses_the_content_root_it_lives_in() {
    let fixture = Fixture::new();
    let nested = fixture.repo.join("vendor/lib");
    std::fs::create_dir_all(nested.join(".git")).unwrap();
    write(&nested.join("page.md"), "# Lib\n");
    write(&nested.join("own.png"), png());
    symlink("vendor/lib", fixture.repo.join("lib-link")).unwrap();
    let loaded = document::load(fixture.worktree(), path("lib-link/page.md")).unwrap();
    assert_eq!(loaded.display_name, "lib-link/page.md");
    assert_eq!(loaded.source.content_root(), &path("vendor/lib"));
    assert!(image::load("own.png", Some(&loaded.source)).is_some());
    assert!(image::load("/own.png", Some(&loaded.source)).is_some());
    assert!(image::load("../../.attachments/a.png", Some(&loaded.source)).is_none());
}
