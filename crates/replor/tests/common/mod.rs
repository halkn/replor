//! Helpers shared by replor's integration tests: building an `App`, making
//! keys and paths, and reading what it rendered or drew.

// Each test file is its own crate and uses only some of these.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use replor::app::App;
use replor::preview::Options;
use replor_render::diagram::{MermaidMode, MermaidWidth};
use replor_render::flavor::Flavor;
use replor_render::layout::icons::IconSet;
use replor_render::layout::theme::{Theme, Variant};
use replor_render::snapshot::{Snapshot, SnapshotPath};
use replor_snapshot::WorktreeSnapshot;

pub fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Options that draw diagrams and icons as text, so the output does not
/// depend on the terminal.
pub fn options(flavor: Flavor, highlight: bool) -> Options {
    Options {
        flavor,
        mermaid: MermaidMode::Unicode,
        mermaid_width: MermaidWidth::Unicode,
        highlight,
        images: None,
        icons: IconSet::Unicode,
    }
}

/// An app on the working tree at `root`, opened at `top`, with the preview
/// taking the keys.
pub fn app(root: &Path, top: &str, flavor: Flavor, width: u16, height: u16) -> App {
    let snapshot = WorktreeSnapshot::open(root).unwrap();
    let mut app = new_app(
        Arc::new(snapshot),
        path(top),
        options(flavor, false),
        width,
        height,
    );
    focus_preview(&mut app);
    app
}

/// An app with the tree taking the keys, as it starts.
pub fn new_app(
    snapshot: Arc<dyn Snapshot>,
    top: SnapshotPath,
    options: Options,
    width: u16,
    height: u16,
) -> App {
    App::new(
        snapshot,
        top,
        options,
        Theme::new(Variant::Dark),
        Rect::new(0, 0, width, height),
    )
}

pub fn focus_preview(app: &mut App) {
    app.key(key(KeyCode::Tab));
}

pub fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

pub fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

pub fn path(text: &str) -> SnapshotPath {
    SnapshotPath::new(text).unwrap()
}

/// The rendered lines of the preview, one per line.
pub fn rendered_text(app: &App) -> String {
    app.rendered()
        .lines
        .iter()
        .map(|line| line.text())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}

/// The drawn screen, one row per line.
pub fn screen(app: &mut App, width: u16, height: u16) -> String {
    let buffer = draw(app, width, height);
    (0..height)
        .map(|y| row(&buffer, y))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn row(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol().to_string())
        .collect()
}
