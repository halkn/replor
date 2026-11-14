//! Following links in the preview, and Back / Forward, on a fixture
//! repository built in a temporary directory.

mod common;

use std::path::Path;

use common::{ch, key, path, write};
use crossterm::event::KeyCode;
use replor::app::{App, Outcome};
use replor_render::flavor::Flavor;
use replor_render::snapshot::SnapshotPath;

/// Enough filler that a heading is below the first screen, and, after it,
/// that the heading can be scrolled to the top.
fn filler() -> String {
    (1..=40).map(|i| format!("Line {i}.\n\n")).collect()
}

/// `outside.md` sits beside the repository, where no link may lead.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    write(&dir.path().join("outside.md"), "# Outside\n");
    write(
        &repo.join("docs/index.md"),
        &format!(
            "# Index\n\n- [Guide setup](guide/setup.md#install-the-tools)\n- [Here](#later-on)\n- [Up and out](../../outside.md)\n- [Link out](out.md)\n- [Missing](none.md)\n- [No such heading](guide/setup.md#nowhere)\n- [Web](https://example.com/)\n\n{}## Later on\n\n{}",
            filler(),
            filler()
        ),
    );
    write(
        &repo.join("docs/guide/setup.md"),
        &format!(
            "# Setup\n\n{}## Install the tools\n\n{}",
            filler(),
            filler()
        ),
    );
    std::os::unix::fs::symlink(dir.path().join("outside.md"), repo.join("docs/out.md")).unwrap();
    write(
        &repo.join("wiki/page.md"),
        &format!(
            "[[_TOC_]]\n\n# Page\n\n{}## Deep section\n\n{}",
            filler(),
            filler()
        ),
    );
    dir
}

fn app(dir: &Path, flavor: Flavor) -> App {
    common::app(&dir.join("repo"), "docs", flavor, 100, 20)
}

/// Selects the `n`th link (1-based) from the top and follows it.
fn follow(app: &mut App, n: usize) -> Outcome {
    app.key(ch('g'));
    for _ in 0..n {
        app.key(ch('f'));
    }
    app.key(key(KeyCode::Enter))
}

/// The link bar as one line: its text, then its position.
fn status(app: &App) -> String {
    match app.link_bar() {
        Some(bar) => format!("{} [{}]", bar.text, bar.position),
        None => String::new(),
    }
}

fn selected_in_tree(app: &App) -> SnapshotPath {
    app.tree().rows()[app.tree().selected()].entry.path.clone()
}

#[test]
fn a_relative_link_opens_the_file_at_its_heading_and_back_returns() {
    let dir = fixture();
    let mut app = app(dir.path(), Flavor::Gfm);
    app.open(&path("docs/index.md"));
    assert_eq!(follow(&mut app, 1), Outcome::Replaced);
    assert_eq!(app.opened(), Some(&path("docs/guide/setup.md")));
    // `## Install the tools` is on line 83 of setup.md.
    assert_eq!(app.top_line(), Some(83), "{}", status(&app));
    // The tree follows the file the link opened.
    assert_eq!(selected_in_tree(&app), path("docs/guide/setup.md"));

    assert_eq!(app.key(ch('[')), Outcome::Replaced);
    assert_eq!(app.opened(), Some(&path("docs/index.md")));
    assert_eq!(app.top_line(), Some(1));
    assert_eq!(selected_in_tree(&app), path("docs/index.md"));

    assert_eq!(app.key(ch(']')), Outcome::Replaced);
    assert_eq!(app.opened(), Some(&path("docs/guide/setup.md")));
    assert_eq!(app.top_line(), Some(83));
}

#[test]
fn a_fragment_moves_within_the_document() {
    let dir = fixture();
    let mut app = app(dir.path(), Flavor::Gfm);
    app.open(&path("docs/index.md"));
    assert_eq!(follow(&mut app, 2), Outcome::Continue);
    assert_eq!(app.opened(), Some(&path("docs/index.md")));
    let later = 11 + 2 * 40;
    assert_eq!(app.top_line(), Some(later), "{}", status(&app));
    app.key(ch('['));
    assert_eq!(app.top_line(), Some(1));
    app.key(ch(']'));
    assert_eq!(app.top_line(), Some(later));
}

#[test]
fn links_out_of_the_snapshot_or_to_nothing_are_not_followed() {
    let dir = fixture();
    let mut app = app(dir.path(), Flavor::Gfm);
    app.open(&path("docs/index.md"));
    for (n, reason) in [
        (3, "leaves the repository"),
        (4, "leaves the repository"),
        (5, "does not exist"),
        (6, "has no heading #nowhere"),
        (7, "not opened: https://example.com/"),
    ] {
        assert_eq!(follow(&mut app, n), Outcome::Continue);
        assert_eq!(app.opened(), Some(&path("docs/index.md")));
        assert_eq!(app.top_line(), Some(1));
        assert!(status(&app).contains(reason), "link {n}: {}", status(&app));
    }
    // Nothing was followed, so there is nothing to go back to.
    app.key(ch('['));
    assert!(status(&app).contains("nothing to go back to"));
}

#[test]
fn a_toc_entry_moves_to_its_heading() {
    let dir = fixture();
    let mut app = app(dir.path(), Flavor::AzureDevops);
    app.open(&path("wiki/page.md"));
    // The TOC lists `Page` and then `Deep section`.
    assert_eq!(follow(&mut app, 2), Outcome::Continue);
    assert_eq!(app.top_line(), Some(5 + 2 * 40), "{}", status(&app));
}

#[test]
fn the_selection_cycles_and_names_the_target() {
    let dir = fixture();
    let mut app = app(dir.path(), Flavor::Gfm);
    app.open(&path("docs/index.md"));
    app.key(ch('f'));
    assert_eq!(status(&app), "guide/setup.md#install-the-tools [1/7]");
    assert!(!app.link_bar().unwrap().refused);
    app.key(ch('F'));
    assert_eq!(status(&app), "https://example.com/ [7/7]");
    // Any other move leaves the selection and the bar.
    app.key(ch('j'));
    assert!(app.link_bar().is_none());
    // Enter without a selection says how to make one.
    app.key(key(KeyCode::Enter));
    assert!(app.link_bar().unwrap().refused);
    assert!(status(&app).contains("select a link"), "{}", status(&app));
}

/// Only the selected link's label is reversed, and the bar above the status
/// line names its destination.
#[test]
fn the_selected_link_is_reversed_and_its_destination_shown() {
    use ratatui::style::Modifier;

    let dir = fixture();
    let mut app = app(dir.path(), Flavor::Gfm);
    app.open(&path("docs/index.md"));
    app.key(ch('f'));
    app.key(ch('f'));
    let buffer = common::draw(&mut app, 100, 20);
    let reversed: String = (0..20)
        .flat_map(|y| (0..100).map(move |x| (x, y)))
        .filter(|&(x, y)| buffer[(x, y)].modifier.contains(Modifier::REVERSED))
        // The tree's selection and the status bar are reversed too; only the
        // preview's body counts.
        .filter(|&(x, y)| x >= 30 && y < 19)
        .map(|(x, y)| buffer[(x, y)].symbol().to_string())
        .collect();
    assert_eq!(reversed, "Here");
    let bar = common::row(&buffer, 18);
    assert!(bar.contains("→ #later-on"), "{bar}");
    assert!(bar.contains("[2/7]"), "{bar}");
}
