//! Switching a Markdown file between its rendering (Preview) and its text
//! (Source) with `s`, on a fixture repository built in a temporary directory.

use std::path::Path;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use replor::app::{App, Outcome};
use replor::preview::{MAX_TEXT_LINES, Options};
use replor_render::diagram::{MermaidMode, MermaidWidth};
use replor_render::flavor::Flavor;
use replor_render::layout::icons::IconSet;
use replor_render::layout::theme::{Theme, Variant};
use replor_render::snapshot::SnapshotPath;
use replor_snapshot::WorktreeSnapshot;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 20;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

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
    let options = Options {
        flavor: Flavor::Gfm,
        mermaid: MermaidMode::Unicode,
        mermaid_width: MermaidWidth::Unicode,
        highlight: false,
        images: None,
        icons: IconSet::Unicode,
    };
    let snapshot = WorktreeSnapshot::open(dir).unwrap();
    let mut app = App::new(
        Arc::new(snapshot),
        SnapshotPath::new("").unwrap(),
        options,
        Theme::new(Variant::Dark),
        Rect::new(0, 0, WIDTH, HEIGHT),
    );
    // The preview takes the keys from here on.
    app.key(key(KeyCode::Tab));
    app
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

fn path(text: &str) -> SnapshotPath {
    SnapshotPath::new(text).unwrap()
}

/// The text of the rendered line at the top of the preview.
fn top_text(app: &App) -> String {
    app.rendered().lines[app.state().viewport.top].text()
}

fn all_text(app: &App) -> String {
    app.rendered()
        .lines
        .iter()
        .map(|line| line.text())
        .collect::<Vec<_>>()
        .join("\n")
}

fn status_bar(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..WIDTH)
        .map(|x| buffer[(x, HEIGHT - 1)].symbol().to_string())
        .collect()
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
    assert!(!all_text(&app).contains("**"));
    assert!(status_bar(&mut app).contains("gfm"));
}

#[test]
fn s_is_refused_for_a_file_that_is_not_markdown() {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("c.rs"));
    let before = all_text(&app);
    assert_eq!(app.key(ch('s')), Outcome::Continue);
    assert_eq!(all_text(&app), before);
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
    let text = all_text(&app);
    assert!(
        text.contains("bold text") && !text.contains("bolder"),
        "{text}"
    );
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
    assert!(!all_text(&app).contains("**bold**"), "{}", all_text(&app));

    // From a link, then Back and Forward.
    app.open(&path("a.md"));
    app.key(ch('G'));
    app.key(ch('f'));
    assert_eq!(app.key(key(KeyCode::Enter)), Outcome::Replaced);
    assert_eq!(app.opened(), Some(&path("b.md")));
    app.key(ch('s'));
    assert!(all_text(&app).contains("**bold**"), "{}", all_text(&app));
    assert_eq!(app.key(ch('[')), Outcome::Replaced);
    assert_eq!(app.opened(), Some(&path("a.md")));
    assert!(!all_text(&app).contains("**Line"), "{}", all_text(&app));
    assert_eq!(app.key(ch(']')), Outcome::Replaced);
    assert!(!all_text(&app).contains("**bold**"), "{}", all_text(&app));
}

#[test]
fn reload_keeps_the_source() {
    let dir = fixture();
    let mut app = app(dir.path());
    app.open(&path("b.md"));
    app.key(ch('s'));
    write(&dir.path().join("b.md"), "# B\n\nNow **bolder** text.\n");
    assert_eq!(app.key(ch('r')), Outcome::Replaced);
    assert!(all_text(&app).contains("**bolder**"), "{}", all_text(&app));
    // The rendering kept aside was read again too.
    app.key(ch('s'));
    assert!(all_text(&app).contains("bolder"), "{}", all_text(&app));
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
    let before = all_text(&app);
    assert_eq!(app.key(ch('s')), Outcome::Continue);
    assert_eq!(all_text(&app), before);
    assert_eq!(app.top_line(), line);
    assert!(refusal(&app).contains("more than"), "{}", refusal(&app));
}
