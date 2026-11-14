//! Switching a Markdown file between its rendering (Preview) and its text
//! (Source) with `s`, on a fixture repository built in a temporary directory.

mod common;

use std::path::Path;

use common::{ch, key, path, rendered_text, write};
use crossterm::event::KeyCode;
use replor::app::{App, Outcome};
use replor::preview::MAX_TEXT_LINES;
use replor_render::flavor::Flavor;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 20;

/// `a.md` has `**Line i.**` on line `2i + 1`, so a line's text says where it
/// is in the file, and the emphasis tells Preview from Source.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    let body: String = (1..=60).map(|i| format!("**Line {i}.**\n\n")).collect();
    write(
        &dir.path().join("a.md"),
        &format!("# Doc\n\n{body}[To b](b.md)\n"),
    );
    write(&dir.path().join("b.md"), "# B\n\nSome **bold** text.\n");
    write(&dir.path().join("c.rs"), "fn main() {}\n");
    dir
}

fn app(dir: &Path) -> App {
    common::app(dir, "", Flavor::Gfm, WIDTH, HEIGHT)
}

/// The text of the rendered line at the top of the preview.
fn top_text(app: &App) -> String {
    app.rendered().lines[app.state().viewport.top].text()
}

fn status_bar(app: &mut App) -> String {
    common::row(&common::draw(app, WIDTH, HEIGHT), HEIGHT - 1)
}

#[test]
fn s_shows_the_source_and_back_at_the_same_line() {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("a.md"));
    for _ in 0..15 {
        app.key(ch('j'));
    }
    let line = app.top_line().expect("a source line at the top");
    assert!(line > 1, "{line}");
    assert!(!top_text(&app).contains("**"), "{}", top_text(&app));

    assert_eq!(app.key(ch('s')), Outcome::Replaced);
    assert_eq!(app.top_line(), Some(line));
    // The opening fence is not counted: the top row is that line of the file.
    let expected = format!("**Line {}.**", (line - 1) / 2);
    let top = top_text(&app);
    assert!(top.contains(&expected), "line {line}: {top}");
    assert!(status_bar(&mut app).contains("source"));

    // Moved in the source, the preview follows.
    for _ in 0..6 {
        app.key(ch('j'));
    }
    let moved = app.top_line().expect("a source line at the top");
    assert!(moved > line, "{moved}");
    // A paragraph's line, which the preview starts a block on. From a blank
    // line between paragraphs the preview goes to the next one instead.
    assert_eq!(moved % 2, 1, "{moved}");
    let moved_text = top_text(&app);

    assert_eq!(app.key(ch('s')), Outcome::Replaced);
    assert_eq!(app.top_line(), Some(moved), "source top was: {moved_text}");
    assert!(!rendered_text(&app).contains("**"));
    assert!(status_bar(&mut app).contains("gfm"));
}

#[test]
fn s_is_refused_for_a_file_that_is_not_markdown() {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("c.rs"));
    let before = rendered_text(&app);
    assert_eq!(app.key(ch('s')), Outcome::Continue);
    assert_eq!(rendered_text(&app), before);
    assert!(refusal(&app).contains("Markdown"), "{}", refusal(&app));

    // The reason belongs to that file, and goes when another opens.
    app.open(&path("a.md"));
    assert_eq!(refusal(&app), "");
    assert_eq!(app.state().status, None);
}

/// The returning rendering is the one kept from before: only `r` reads the
/// file again.
#[test]
fn back_from_the_source_is_the_rendering_kept_from_before() {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("b.md"));
    app.key(ch('s'));
    write(&dir.path().join("b.md"), "# B\n\nNow **bolder** text.\n");
    assert_eq!(app.key(ch('s')), Outcome::Replaced);
    let text = rendered_text(&app);
    assert!(
        text.contains("bold text") && !text.contains("bolder"),
        "{text}"
    );
}

/// The source is the text the rendering was made from, so the two show the
/// same version of the file: only `r` reads it again.
#[test]
fn the_source_is_the_text_the_rendering_was_made_from() {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("b.md"));
    write(&dir.path().join("b.md"), "# B\n\nNow **bolder** text.\n");
    assert_eq!(app.key(ch('s')), Outcome::Replaced);
    let text = rendered_text(&app);
    assert!(
        text.contains("**bold**") && !text.contains("bolder"),
        "{text}"
    );
    assert_eq!(app.key(ch('r')), Outcome::Replaced);
    assert!(
        rendered_text(&app).contains("**bolder**"),
        "{}",
        rendered_text(&app)
    );
}

/// A symlink's source is the text the rendering followed it to.
#[test]
fn the_source_of_a_symlink_is_the_file_it_leads_to() {
    let dir = fixture();
    std::os::unix::fs::symlink("b.md", dir.path().join("link.md")).unwrap();
    let mut app = app(dir.path());
    app.open(&path("link.md"));
    assert_eq!(app.key(ch('s')), Outcome::Replaced, "{}", refusal(&app));
    assert!(
        rendered_text(&app).contains("**bold**"),
        "{}",
        rendered_text(&app)
    );
}

/// A file the rendering could not read has no text to show as source, only
/// the notice in its place.
#[test]
fn a_markdown_file_that_was_not_read_has_no_source() {
    let dir = fixture();
    std::fs::write(dir.path().join("bad.md"), b"# Bad\n\n\xff\n").unwrap();
    let mut app = app(dir.path());
    app.open(&path("bad.md"));
    let before = rendered_text(&app);
    assert_eq!(app.key(ch('s')), Outcome::Continue);
    assert_eq!(rendered_text(&app), before);
    assert!(refusal(&app).contains("UTF-8"), "{}", refusal(&app));
}

fn refusal(app: &App) -> String {
    app.link_bar()
        .filter(|bar| bar.refused)
        .map(|bar| bar.text.clone())
        .unwrap_or_default()
}

#[test]
fn another_file_opens_in_the_preview() {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("a.md"));
    app.key(ch('s'));
    app.open(&path("b.md"));
    assert!(
        !rendered_text(&app).contains("**bold**"),
        "{}",
        rendered_text(&app)
    );

    // From a link, then Back and Forward.
    app.open(&path("a.md"));
    app.key(ch('G'));
    app.key(ch('f'));
    assert_eq!(app.key(key(KeyCode::Enter)), Outcome::Replaced);
    assert_eq!(app.opened(), Some(&path("b.md")));
    app.key(ch('s'));
    assert!(
        rendered_text(&app).contains("**bold**"),
        "{}",
        rendered_text(&app)
    );
    assert_eq!(app.key(ch('[')), Outcome::Replaced);
    assert_eq!(app.opened(), Some(&path("a.md")));
    assert!(
        !rendered_text(&app).contains("**Line"),
        "{}",
        rendered_text(&app)
    );
    assert_eq!(app.key(ch(']')), Outcome::Replaced);
    assert!(
        !rendered_text(&app).contains("**bold**"),
        "{}",
        rendered_text(&app)
    );
}

#[test]
fn reload_keeps_the_source() {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("b.md"));
    app.key(ch('s'));
    write(&dir.path().join("b.md"), "# B\n\nNow **bolder** text.\n");
    assert_eq!(app.key(ch('r')), Outcome::Replaced);
    assert!(
        rendered_text(&app).contains("**bolder**"),
        "{}",
        rendered_text(&app)
    );
    // The rendering kept aside was read again too.
    app.key(ch('s'));
    assert!(
        rendered_text(&app).contains("bolder"),
        "{}",
        rendered_text(&app)
    );
}

/// The rendering stays, at the same place, with the reason in the link bar.
#[test]
fn the_source_of_a_long_markdown_file_is_not_shown() {
    let dir = fixture();
    let body: String = (1..=MAX_TEXT_LINES / 2 + 1)
        .map(|i| format!("Line {i}.\n\n"))
        .collect();
    write(&dir.path().join("long.md"), &body);
    let mut app = app(dir.path());
    app.open(&path("long.md"));
    for _ in 0..15 {
        app.key(ch('j'));
    }
    let line = app.top_line();
    let before = rendered_text(&app);
    assert_eq!(app.key(ch('s')), Outcome::Continue);
    assert_eq!(rendered_text(&app), before);
    assert_eq!(app.top_line(), line);
    assert!(refusal(&app).contains("more than"), "{}", refusal(&app));
}

/// The status bar's `line/lines`: the only word with a `/` between numbers.
fn position(status: &str) -> String {
    status
        .split_whitespace()
        .find(|word| {
            word.split_once('/')
                .is_some_and(|(a, b)| a.parse::<usize>().is_ok() && b.parse::<usize>().is_ok())
        })
        .unwrap_or_default()
        .to_string()
}

/// A text file counts its own lines, not the fences around it.
#[test]
fn the_status_bar_of_a_text_file_counts_the_file_lines() {
    let dir = fixture();
    let body: String = (1..=121).map(|i| format!("line {i}\n")).collect();
    write(&dir.path().join("long.txt"), &body);
    let mut app = app(dir.path());
    app.open(&path("long.txt"));
    assert_eq!(position(&status_bar(&mut app)), "1/121");
    for _ in 0..20 {
        app.key(ch('j'));
    }
    let line = app.top_line().expect("a source line at the top");
    let top = top_text(&app);
    assert!(
        top.trim_end().ends_with(&format!(" line {line}")),
        "line {line}: {top}"
    );
    let status = status_bar(&mut app);
    assert_eq!(position(&status), format!("{line}/121"), "{status}");
}

/// With room for one row, the bottom of a text file is its closing fence,
/// which is no line of the file: the status bar stops at the last one.
#[test]
fn the_closing_fence_counts_as_the_last_line() {
    let dir = fixture();
    let body: String = (1..=121).map(|i| format!("line {i}\n")).collect();
    write(&dir.path().join("long.txt"), &body);
    let height = 2;
    let mut app = common::app(dir.path(), "", Flavor::Gfm, WIDTH, height);
    app.open(&path("long.txt"));
    app.key(ch('G'));
    assert_eq!(app.top_line(), Some(121), "{}", top_text(&app));
    let status = common::row(&common::draw(&mut app, WIDTH, height), height - 1);
    assert_eq!(position(&status), "121/121", "{status}");
}

#[test]
fn preview_and_source_show_the_same_line_and_lines() {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("a.md"));
    for _ in 0..15 {
        app.key(ch('j'));
    }
    let line = app.top_line().expect("a source line at the top");
    let preview = status_bar(&mut app);
    // `a.md` has 123 lines.
    assert_eq!(position(&preview), format!("{line}/123"), "{preview}");
    app.key(ch('s'));
    let source = status_bar(&mut app);
    assert_eq!(position(&source), position(&preview), "{source}");
}

fn search(app: &mut App, query: &str) {
    app.key(ch('/'));
    for c in query.chars() {
        app.key(ch(c));
    }
    app.key(key(KeyCode::Enter));
}

/// The text of the rendered line the current match is on.
fn current_match(app: &App) -> String {
    let found = app.state().search.current().expect("a current match");
    app.rendered().lines[found.line].text()
}

/// A confirmed search outlives the switch, counted again on the other face,
/// and `n` goes on through its matches there. The place is kept as before.
#[test]
fn a_confirmed_search_is_kept_across_the_switch() {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("a.md"));
    // `Line 5.` and `Line 50.` to `Line 59.`.
    search(&mut app, "line 5");
    assert_eq!(app.state().search.count(), 11);
    let line = app.top_line();

    assert_eq!(app.key(ch('s')), Outcome::Replaced);
    assert_eq!(app.top_line(), line);
    assert_eq!(app.state().search.query(), "line 5");
    assert_eq!(app.state().search.count(), 11);
    let aimed = app.state().search.current();
    app.key(ch('n'));
    assert_eq!(app.state().search.current(), aimed);
    let shown = app.state().search.current().unwrap().line;
    let top = app.state().viewport.top;
    assert!((top..top + usize::from(HEIGHT)).contains(&shown), "{shown}");
    let found = current_match(&app);
    assert!(found.contains("**Line 5"), "{found}");

    let line = app.top_line().expect("a source line at the top");
    assert_eq!(app.key(ch('s')), Outcome::Replaced);
    // From a blank line between paragraphs the preview goes to the next one.
    let back = app.top_line().expect("a source line at the top");
    assert!((line..=line + 1).contains(&back), "{line} -> {back}");
    assert_eq!(app.state().search.count(), 11);
    app.key(ch('N'));
    let found = current_match(&app);
    assert!(found.contains("Line 5") && !found.contains("**"), "{found}");
}

/// A query only one face has keeps on the other, which says it has none.
#[test]
fn a_search_with_no_match_on_the_other_face_says_so() {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("a.md"));
    app.key(ch('s'));
    search(&mut app, "**line");
    assert_eq!(app.state().search.count(), 60);
    app.key(ch('s'));
    assert_eq!(app.state().search.query(), "**line");
    assert_eq!(app.state().search.count(), 0);
    let status = status_bar(&mut app);
    assert!(status.contains("no match: **line"), "{status}");
    app.key(ch('s'));
    assert_eq!(app.state().search.count(), 60);
    let status = status_bar(&mut app);
    assert!(!status.contains("no match"), "{status}");
}

/// `line 3` matches `Line 3.` above the place reached, and `Line 30.` to
/// `Line 39.` below the screen.
fn past_line_3() -> (tempfile::TempDir, App) {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("a.md"));
    search(&mut app, "line 3");
    assert_eq!(app.state().search.count(), 11);
    for _ in 0..10 {
        app.key(ch('j'));
    }
    let line = app.top_line().expect("a source line at the top");
    assert!(7 < line && line + 20 < 61, "{line}");
    (dir, app)
}

/// After the switch `n` goes to the first match below the top, though it is
/// off the screen, and `N` to the last above it: none is skipped.
#[test]
fn after_the_switch_n_and_n_go_to_the_nearest_matches() {
    let (_dir, mut app) = past_line_3();
    app.key(ch('s'));
    app.key(ch('n'));
    let found = current_match(&app);
    assert!(found.contains("**Line 30.**"), "{found}");

    let (_dir, mut app) = past_line_3();
    app.key(ch('s'));
    app.key(ch('N'));
    let found = current_match(&app);
    assert!(found.contains("**Line 3.**"), "{found}");
    let status = status_bar(&mut app);
    assert!(!status.contains("wrapped"), "{status}");
}

/// A status said of the face left behind is not said of the new one.
#[test]
fn the_switch_leaves_the_status_of_the_other_face() {
    let (_dir, mut app) = past_line_3();
    // From `Line 3.`, the first match, back past the top to the last.
    app.key(ch('N'));
    let status = status_bar(&mut app);
    assert!(status.contains("wrapped"), "{status}");
    app.key(ch('s'));
    let status = status_bar(&mut app);
    assert!(!status.contains("wrapped"), "{status}");
}

/// `p.md` has a paragraph of 21 lines, `pNN …` on lines 9 to 29, long enough
/// to wrap in the preview, between short ones and blank lines.
fn paragraph() -> (tempfile::TempDir, App) {
    let dir = fixture();
    let long: String = (1..=21)
        .map(|i| format!("p{i:02} alpha beta gamma delta epsilon zeta eta theta\n"))
        .collect();
    let after: String = (1..=40).map(|i| format!("\nFiller {i}.\n")).collect();
    write(
        &dir.path().join("p.md"),
        &format!("# P\n\nOne.\n\nTwo.\n\nThree.\n\n{long}{after}"),
    );
    let mut app = app(dir.path());
    app.open(&path("p.md"));
    (dir, app)
}

/// Scrolls the source until `line` of the file is at the top.
fn source_at(app: &mut App, line: usize) {
    app.key(ch('s'));
    for _ in 0..line + 1 {
        if app.top_line() == Some(line) {
            return;
        }
        app.key(ch('j'));
    }
    assert_eq!(app.top_line(), Some(line));
}

/// The middle of a paragraph stays the middle in the preview, and comes back
/// to where it was.
#[test]
fn the_middle_of_a_paragraph_is_kept_across_the_switch_and_back() {
    let (_dir, mut app) = paragraph();
    source_at(&mut app, 19);
    app.key(ch('s'));
    let top = top_text(&app);
    assert!(top.contains("alpha") && !top.contains("p01"), "{top}");
    app.key(ch('s'));
    assert_eq!(app.top_line(), Some(19));
}

#[test]
fn the_middle_of_a_paragraph_in_the_preview_is_not_its_first_line() {
    let (_dir, mut app) = paragraph();
    for _ in 0..40 {
        if top_text(&app).contains("p10") {
            break;
        }
        app.key(ch('j'));
    }
    let top = top_text(&app);
    assert!(top.contains("p10") && !top.contains("p01"), "{top}");
    app.key(ch('s'));
    let line = app.top_line().expect("a source line at the top");
    assert!((10..=29).contains(&line), "{line}");
}

/// Short lines join into a paragraph of two rendered lines, too few to tell
/// 21 lines apart: coming back before scrolling returns to the very line.
#[test]
fn a_line_of_a_short_paragraph_comes_back_as_it_was() {
    let dir = fixture();
    let short: String = (1..=21).map(|i| format!("q{i:02}\n")).collect();
    let after: String = (1..=40).map(|i| format!("\nFiller {i}.\n")).collect();
    write(
        &dir.path().join("q.md"),
        &format!("# Q\n\nOne.\n\nTwo.\n\nThree.\n\n{short}{after}"),
    );
    let mut app = app(dir.path());
    app.open(&path("q.md"));
    source_at(&mut app, 19);
    app.key(ch('s'));
    app.key(ch('s'));
    assert_eq!(app.top_line(), Some(19));
}

/// A blank line between paragraphs is in no block: the switch goes to the
/// next one, and coming back stays there.
#[test]
fn a_blank_line_goes_to_the_next_paragraph_and_stays() {
    let (_dir, mut app) = paragraph();
    source_at(&mut app, 8);
    app.key(ch('s'));
    app.key(ch('s'));
    assert_eq!(app.top_line(), Some(9));
}

/// The source ends in 30 short lines the rendering joins into two, so the
/// end of the source is further down than the rendering can scroll, which
/// holds its top above where it was asked to be. Coming back is still to
/// the end of the source.
#[test]
fn the_end_of_the_source_comes_back_to_itself() {
    let dir = fixture();
    let filler: String = (1..=20).map(|i| format!("Filler {i}.\n\n")).collect();
    let short: String = (1..=30).map(|i| format!("e{i:02}\n")).collect();
    write(&dir.path().join("e.md"), &format!("# E\n\n{filler}{short}"));
    let mut app = app(dir.path());
    app.open(&path("e.md"));
    app.key(ch('s'));
    app.key(ch('G'));
    let line = app.top_line();
    assert!(line.is_some_and(|line| line > 42), "{line:?}");
    app.key(ch('s'));
    app.key(ch('s'));
    assert_eq!(app.top_line(), line);
}

/// A heading shares its range with its `[[_TOC_]]` entry: coming back is to
/// the heading the reader was at, not to the entry.
#[test]
fn a_heading_listed_in_the_toc_comes_back_to_itself() {
    let dir = fixture();
    let filler: String = (1..=30).map(|i| format!("Filler {i}.\n\n")).collect();
    write(
        &dir.path().join("t.md"),
        &format!("[[_TOC_]]\n\n# T\n\n{filler}## Foo\n\n{filler}"),
    );
    let mut app = common::app(dir.path(), "", Flavor::AzureDevops, WIDTH, HEIGHT);
    app.open(&path("t.md"));
    for _ in 0..80 {
        if app.state().viewport.top > 5 && top_text(&app).contains("Foo") {
            break;
        }
        app.key(ch('j'));
    }
    let top = app.state().viewport.top;
    assert!(top > 5 && top_text(&app).contains("Foo"), "{top}");
    app.key(ch('s'));
    app.key(ch('s'));
    assert_eq!(app.state().viewport.top, top, "{}", top_text(&app));
}
