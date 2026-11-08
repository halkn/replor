//! The Files lens on replor's own repository: started without a path it opens
//! `docs/`, and `docs/design/concept.md` shows with its Mermaid diagram drawn.
//! The repository is only read.

use std::path::Path;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use replor::app::{App, Outcome};
use replor::preview::Options;
use replor::start;
use replor_render::diagram::{MermaidMode, MermaidWidth};
use replor_render::flavor::Flavor;
use replor_render::layout::icons::IconSet;
use replor_render::layout::theme::{Theme, Variant};
use replor_render::snapshot::SnapshotPath;

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_repository_opens_at_docs_and_shows_the_concept_with_its_diagram() {
    let start = start::resolve(None, repository()).unwrap();
    assert_eq!(start.top.as_str(), "docs");
    let options = Options {
        flavor: Flavor::AzureDevops,
        mermaid: MermaidMode::Unicode,
        mermaid_width: MermaidWidth::Unicode,
        highlight: true,
        images: None,
        icons: IconSet::Unicode,
    };
    let (width, height) = (140, 40);
    let mut app = App::new(
        Arc::new(start.snapshot),
        start.top,
        options,
        Theme::new(Variant::Dark),
        Rect::new(0, 0, width, height),
    );
    let tree = screen(&mut app, width, height);
    for name in ["▸ design", "▸ render"] {
        assert!(tree.contains(name), "{name} missing:\n{tree}");
    }

    app.open(&SnapshotPath::new("docs/design/concept.md").unwrap());
    let text: String = app
        .rendered()
        .lines
        .iter()
        .map(|line| line.text() + "\n")
        .collect();
    assert!(text.contains("Working tree"), "{text}");
    assert!(
        !text.contains("flowchart LR"),
        "the diagram should be drawn, not shown as source"
    );
    let drawn = screen(&mut app, width, height);
    assert!(drawn.contains("docs/design/concept.md"), "{drawn}");
    assert!(drawn.contains("▾ design"), "{drawn}");

    let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(app.key(key), Outcome::Quit);
}

/// The S1 link in the P0 results table leads to a Japanese heading, and Back
/// returns to the table.
#[test]
fn the_p0_results_link_reaches_its_japanese_heading_and_back() {
    let start = start::resolve(None, repository()).unwrap();
    let options = Options {
        flavor: Flavor::AzureDevops,
        mermaid: MermaidMode::Unicode,
        mermaid_width: MermaidWidth::Unicode,
        highlight: false,
        images: None,
        icons: IconSet::Unicode,
    };
    let mut app = App::new(
        Arc::new(start.snapshot),
        start.top,
        options,
        Theme::new(Variant::Dark),
        Rect::new(0, 0, 140, 40),
    );
    app.open(&SnapshotPath::new("docs/spike/p0-results.md").unwrap());
    let press = |app: &mut App, code| app.key(KeyEvent::new(code, KeyModifiers::NONE));
    press(&mut app, KeyCode::Tab);
    let mut found = false;
    for _ in 0..20 {
        press(&mut app, KeyCode::Char('f'));
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
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.top_line(), Some(21), "{:?}", app.state().status);
    press(&mut app, KeyCode::Char('['));
    assert_eq!(app.top_line(), before);
}
