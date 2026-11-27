//! Where the wiki is found from, and what changes for its pages: the flavor
//! they are drawn with and where their `/` references start
//! (`docs/design/p2-wiki.md`, "Wiki root", "flavor").

mod common;

use std::path::Path;

use common::{ch, key, path};
use crossterm::event::KeyCode;
use replor::app::App;
use replor_render::flavor::Flavor;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 20;

fn code_wiki() -> tempfile::TempDir {
    common::fixture("code-wiki")
}

/// Opened at `top` with `gfm` configured, so a page drawn as `azure-devops`
/// got that from the wiki.
fn app(dir: &Path, top: &str) -> App {
    common::app(dir, top, Flavor::Gfm, WIDTH, HEIGHT)
}

fn status_bar(app: &mut App) -> String {
    common::row(&common::draw(app, WIDTH, HEIGHT), HEIGHT - 1)
}

/// Follows the first link of the open document.
fn follow_first(app: &mut App) {
    app.key(ch('g'));
    app.key(ch('f'));
    app.key(key(KeyCode::Enter));
}

#[test]
fn a_page_of_the_wiki_is_drawn_as_azure_devops_and_the_rest_as_configured() {
    let dir = code_wiki();
    let mut app = app(dir.path(), "docs/wiki/Sub");
    assert_eq!(app.wiki_root(), Some(&path("docs/wiki")));

    app.open(&path("docs/wiki/Sub/Page.md"));
    assert!(status_bar(&mut app).contains("azure-devops"));
    app.open(&path("README.md"));
    assert!(status_bar(&mut app).contains("gfm"));
}

#[test]
fn a_root_relative_link_in_the_wiki_starts_at_the_wiki_root() {
    let dir = code_wiki();
    let mut app = app(dir.path(), "docs/wiki/Sub");
    app.open(&path("docs/wiki/Sub/Page.md"));
    follow_first(&mut app);
    assert_eq!(app.opened(), Some(&path("docs/wiki/Home.md")));
}

#[test]
fn without_a_wiki_the_same_link_starts_at_the_repository_root() {
    let dir = code_wiki();
    let mut app = app(dir.path(), "docs");
    assert_eq!(app.wiki_root(), None);

    app.open(&path("docs/wiki/Sub/Page.md"));
    assert!(status_bar(&mut app).contains("gfm"));
    follow_first(&mut app);
    assert_eq!(app.opened(), Some(&path("docs/wiki/Sub/Page.md")));
    let refused = app.link_bar().map(|bar| bar.text.clone());
    assert_eq!(
        refused.as_deref(),
        Some("cannot follow /Home.md: Home.md does not exist")
    );
}
