//! The Files lens on replor's own repository: started without a path it opens
//! `docs/`, and `docs/design/concept.md` shows with its Mermaid diagram drawn.
//! The repository is only read.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::{ch, focus_preview, key, new_app, options, path, rendered_text, screen, write};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use replor::app::Outcome;
use replor::finder::Progress;
use replor::start;
use replor_render::flavor::Flavor;

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

#[test]
fn the_repository_opens_at_docs_and_shows_the_concept_with_its_diagram() {
    let start = start::resolve(None, repository()).unwrap();
    assert_eq!(start.top.as_str(), "docs");
    let (width, height) = (140, 40);
    let mut app = new_app(
        Arc::new(start.snapshot),
        start.top,
        options(Flavor::AzureDevops, true),
        width,
        height,
    );
    let tree = screen(&mut app, width, height);
    for name in ["▸ design", "▸ render"] {
        assert!(tree.contains(name), "{name} missing:\n{tree}");
    }

    app.open(&path("docs/design/concept.md"));
    let text = rendered_text(&app);
    assert!(text.contains("Working tree"), "{text}");
    assert!(
        !text.contains("flowchart LR"),
        "the diagram should be drawn, not shown as source"
    );
    let drawn = screen(&mut app, width, height);
    assert!(drawn.contains("docs/design/concept.md"), "{drawn}");
    assert!(drawn.contains("▾ design"), "{drawn}");

    assert_eq!(app.key(ch('q')), Outcome::Quit);
}

/// The S1 link in the P0 results table leads to a Japanese heading, and Back
/// returns to the table.
#[test]
fn the_p0_results_link_reaches_its_japanese_heading_and_back() {
    let start = start::resolve(None, repository()).unwrap();
    let mut app = new_app(
        Arc::new(start.snapshot),
        start.top,
        options(Flavor::AzureDevops, false),
        140,
        40,
    );
    app.open(&path("docs/spike/p0-results.md"));
    focus_preview(&mut app);
    let mut found = false;
    for _ in 0..20 {
        app.key(ch('f'));
        let bar = app
            .link_bar()
            .map(|bar| bar.text.clone())
            .unwrap_or_default();
        if bar == "#s1-普段の環境で画像を表示できるか" {
            found = true;
            break;
        }
    }
    assert!(found, "the S1 link should be selectable");
    let before = app.top_line();
    app.key(key(KeyCode::Enter));
    assert_eq!(app.top_line(), Some(21), "{:?}", app.state().status);
    app.key(ch('['));
    assert_eq!(app.top_line(), before);
}

/// `Ctrl-p` and a few letters of the name reach a file outside `docs/` as
/// well as inside it. The layout is replor's own, in a temporary directory,
/// so untracked files of the checkout cannot change the matches.
#[test]
fn the_finder_opens_the_p1_core_design_by_a_few_letters() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    for file in [
        "docs/design/p1-core.md",
        "docs/design/concept.md",
        "docs/spike/p0-results.md",
        "crates/replor/src/pager/mod.rs",
        "crates/replor-snapshot/src/lib.rs",
        "README.md",
    ] {
        write(&dir.path().join(file), "# Page\n");
    }
    let start = start::resolve(None, dir.path()).unwrap();
    assert_eq!(start.top.as_str(), "docs");
    let mut app = new_app(
        Arc::new(start.snapshot),
        start.top,
        options(Flavor::AzureDevops, false),
        140,
        40,
    );
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    while app.candidates().progress() == Progress::Listing {
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.tick();
    }
    app.tick();
    assert_eq!(app.candidates().progress(), Progress::Complete);
    for c in "p1co".chars() {
        app.key(ch(c));
    }
    assert_eq!(app.key(key(KeyCode::Enter)), Outcome::Replaced);
    assert_eq!(app.opened().unwrap().as_str(), "docs/design/p1-core.md");
}
