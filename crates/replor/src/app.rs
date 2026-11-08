//! The Files lens: the tree on the left, the preview on the right.
//!
//! A file is parsed and its diagrams rendered once, when it is opened. Layout
//! re-runs only when the pane changes size or the file is reloaded, never per
//! frame.

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use replor_render::error::AppError;
use replor_render::layout::theme::Theme;
use replor_render::layout::{LayoutOptions, RenderedDocument, StyleRole, layout_document};
use replor_render::markdown::model::{Document, headings};
use replor_render::snapshot::{EntryKind, Snapshot, SnapshotPath};

use crate::pager::TerminalGuard;
use crate::pager::event::{Input, map};
use crate::pager::help;
use crate::pager::images::{self, ImageSession, Placed};
use crate::pager::search::SearchIndex;
use crate::pager::state::{Action, Mode, OutlineItem, PagerState, rendered_line_for_source};
use crate::pager::view::{self, ViewContext, convert, elide_end, help_rows, widest_line};
use crate::preview::{self, Content, Options};
use crate::tree::{Opened, Tree};

/// How long a frame waits for input before looping again.
const POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Preview,
}

/// What a key does, decided before anything changes so the help table can be
/// checked against it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Quit,
    SwitchFocus,
    Reload,
    TreeMove(isize),
    TreeExpand,
    TreeCollapse,
    TreeOpen,
    Preview(Input),
    Ignored,
}

/// Overlays and the search prompt own every key while they are open, from
/// either pane; otherwise the focused pane gets the key.
pub fn route(focus: Focus, mode: Mode, key: KeyEvent) -> Command {
    if mode != Mode::Normal {
        return Command::Preview(map(key, mode));
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match (focus, key.code) {
            (_, KeyCode::Char('c')) => Command::Quit,
            (Focus::Preview, _) => Command::Preview(map(key, mode)),
            (Focus::Tree, _) => Command::Ignored,
        };
    }
    match key.code {
        KeyCode::Tab => return Command::SwitchFocus,
        KeyCode::Char('r') => return Command::Reload,
        KeyCode::Char('?') => return Command::Preview(Input::Navigate(Action::ToggleHelp)),
        KeyCode::Char('q') | KeyCode::Esc => return Command::Quit,
        _ => {}
    }
    match focus {
        Focus::Tree => match key.code {
            KeyCode::Char('j') | KeyCode::Down => Command::TreeMove(1),
            KeyCode::Char('k') | KeyCode::Up => Command::TreeMove(-1),
            KeyCode::Char('l') => Command::TreeExpand,
            KeyCode::Char('h') => Command::TreeCollapse,
            KeyCode::Enter => Command::TreeOpen,
            _ => Command::Ignored,
        },
        Focus::Preview => match map(key, mode) {
            Input::Ignored => Command::Ignored,
            input => Command::Preview(input),
        },
    }
}

/// What a key changed, for the frame loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    /// A different surface is in the preview, so the images it showed are
    /// freed.
    Replaced,
    Quit,
}

pub struct App {
    snapshot: Arc<dyn Snapshot>,
    tree: Tree,
    focus: Focus,
    options: Options,
    theme: Theme,
    opened: Option<SnapshotPath>,
    content: Content,
    rendered: RenderedDocument,
    texts: SearchIndex,
    state: PagerState,
    tree_area: Rect,
    preview_area: Rect,
}

impl App {
    pub fn new(
        snapshot: Arc<dyn Snapshot>,
        top: SnapshotPath,
        options: Options,
        theme: Theme,
        size: Rect,
    ) -> Self {
        let tree = Tree::new(Arc::clone(&snapshot), top);
        let message = match tree.error() {
            Some(error) => format!("Cannot list this directory: {error}"),
            None => "Select a file in the tree (Tab switches panes, ? shows the keys).".into(),
        };
        let content = preview::notice(&message, &options);
        let (tree_area, preview_area) = split(size);
        let mut app = Self {
            snapshot,
            tree,
            focus: Focus::Tree,
            options,
            theme,
            opened: None,
            content,
            rendered: RenderedDocument::default(),
            texts: SearchIndex::default(),
            state: PagerState::new(0, 0, 1, 1),
            tree_area,
            preview_area,
        };
        app.lay_out_fresh();
        app
    }

    /// Opens `path` in the preview and selects it in the tree.
    pub fn open(&mut self, path: &SnapshotPath) {
        self.tree.select(path);
        self.content = preview::open(&self.snapshot, path, &self.options);
        self.opened = Some(path.clone());
        self.lay_out_fresh();
    }

    /// A message in the status bar, such as a warning about the configuration.
    pub fn set_status(&mut self, status: String) {
        self.state.status = Some(replor_render::text::into_visible(status));
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    pub fn state(&self) -> &PagerState {
        &self.state
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    pub fn rendered(&self) -> &RenderedDocument {
        &self.rendered
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        match route(self.focus, self.state.mode(), key) {
            Command::Quit => return Outcome::Quit,
            Command::SwitchFocus => {
                self.focus = match self.focus {
                    Focus::Tree => Focus::Preview,
                    Focus::Preview => Focus::Tree,
                }
            }
            Command::Reload => {
                self.reload();
                return Outcome::Replaced;
            }
            Command::TreeMove(delta) => self.tree.move_by(delta),
            Command::TreeExpand => {
                let opened = self.tree.expand();
                return self.show(opened);
            }
            Command::TreeCollapse => self.tree.collapse(),
            Command::TreeOpen => {
                let opened = self.tree.open();
                return self.show(opened);
            }
            Command::Preview(input) => {
                if !self.apply(input) {
                    return Outcome::Quit;
                }
            }
            Command::Ignored => {}
        }
        Outcome::Continue
    }

    /// Puts a file the tree handed over into the preview.
    fn show(&mut self, opened: Opened) -> Outcome {
        match opened {
            Opened::File(entry) if entry.kind == EntryKind::Submodule => {
                self.content = preview::notice("A submodule is another repository.", &self.options);
                self.opened = None;
                self.lay_out_fresh();
                Outcome::Replaced
            }
            Opened::File(entry) => {
                self.open(&entry.path);
                Outcome::Replaced
            }
            Opened::Toggled | Opened::Nothing => Outcome::Continue,
        }
    }

    pub fn resize(&mut self, size: Rect) {
        (self.tree_area, self.preview_area) = split(size);
        self.lay_out_again();
    }

    /// The tree and the open file read again, the place in the file kept.
    fn reload(&mut self) {
        self.tree.reload();
        if let Some(path) = self.opened.clone() {
            self.content = preview::open(&self.snapshot, &path, &self.options);
            self.lay_out_again();
            self.state.status = Some("reloaded".to_string());
        }
    }

    /// Returns `false` when the pager should exit.
    fn apply(&mut self, input: Input) -> bool {
        let state = &mut self.state;
        let texts = &self.texts;
        match input {
            Input::Navigate(action) => return state.apply(action),
            Input::SearchChar(c) => state.insert_search_char(c, texts),
            Input::SearchEdit(edit) => state.edit_search(edit, texts),
            Input::SearchConfirm => state.confirm_search(texts),
            Input::SearchCancel => state.cancel_search(),
            Input::OutlineMove(delta) => state.move_outline(delta),
            Input::OutlineConfirm => state.confirm_outline(),
            Input::OutlineCancel => state.cancel_outline(),
            Input::HelpScroll(delta) => {
                state.scroll_help(delta, help::ENTRIES.len(), help_rows(state.viewport.height))
            }
            Input::HelpClose => state.close_help(),
            Input::Ignored => {}
        }
        true
    }

    fn body(&self) -> (usize, usize) {
        let area = self.preview_area;
        (
            area.height.saturating_sub(1).max(1) as usize,
            area.width.max(1) as usize,
        )
    }

    /// A new surface in the preview, read from the top.
    fn lay_out_fresh(&mut self) {
        let (height, width) = self.body();
        self.rendered = layout_document(&self.content.document, LayoutOptions::new(width));
        self.texts = self.rendered.lines.iter().map(|l| l.text()).collect();
        let status = self.state.status.take();
        self.state = PagerState::new(
            self.rendered.lines.len(),
            widest_line(&self.rendered.lines),
            height,
            width,
        );
        self.state.status = status;
        self.state
            .set_outline(outline(self.content.document.document(), &self.rendered));
    }

    /// The same surface laid out again, keeping the reader's place.
    fn lay_out_again(&mut self) {
        let anchor = self.state.anchor(&self.rendered.lines);
        let (height, width) = self.body();
        self.rendered = layout_document(&self.content.document, LayoutOptions::new(width));
        self.texts = self.rendered.lines.iter().map(|l| l.text()).collect();
        self.state.resize(
            self.rendered.lines.len(),
            widest_line(&self.rendered.lines),
            height,
            width,
        );
        self.state
            .set_outline(outline(self.content.document.document(), &self.rendered));
        self.state.restore_anchor(&self.rendered.lines, anchor);
        self.state.recompute_searches(&self.texts);
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        self.draw_tree(frame);
        let ctx = ViewContext {
            title: &self.content.title,
            flavor: &self.content.kind,
            source_lines: self.content.document.document().source.line_count(),
            diagnostics: self.rendered.diagnostics.len(),
            theme: self.theme,
        };
        view::draw(
            frame,
            self.preview_area,
            &self.rendered.lines,
            &self.state,
            &ctx,
        );
    }

    fn draw_tree(&mut self, frame: &mut Frame) {
        let area = self.tree_area;
        if area.width == 0 {
            return;
        }
        let normal = convert(self.theme.style(StyleRole::Normal));
        let muted = convert(self.theme.style(StyleRole::Muted));
        let status = convert(self.theme.style(StyleRole::Status));
        let rows_height = area.height.saturating_sub(1) as usize;
        self.tree.follow(rows_height);
        let width = area.width.saturating_sub(1) as usize;
        let selected = self.tree.selected();
        let lines: Vec<Line> = self
            .tree
            .rows()
            .iter()
            .enumerate()
            .skip(self.tree.scroll)
            .take(rows_height)
            .map(|(index, row)| {
                let marker = match (row.entry.kind, row.expanded) {
                    (EntryKind::Directory, true) => "▾ ",
                    (EntryKind::Directory, false) => "▸ ",
                    _ => "  ",
                };
                // A file name is the snapshot's, and may hold terminal controls.
                let name = replor_render::text::visible(&row.entry.name);
                let suffix = match row.entry.kind {
                    EntryKind::Symlink => "@",
                    EntryKind::Submodule => " (submodule)",
                    _ => "",
                };
                let text = format!("{}{marker}{name}{suffix}", "  ".repeat(row.depth));
                let mut style = match row.entry.kind {
                    EntryKind::Other | EntryKind::Submodule => muted,
                    _ => normal,
                };
                if index == selected {
                    style = match self.focus {
                        Focus::Tree => style.add_modifier(Modifier::REVERSED),
                        Focus::Preview => style.add_modifier(Modifier::BOLD),
                    };
                }
                Line::from(Span::styled(elide_end(&text, width), style))
            })
            .collect();
        let rows = Rect {
            height: rows_height as u16,
            width: width as u16,
            ..area
        };
        frame.render_widget(Paragraph::new(lines), rows);
        // The column between the panes.
        let border = Rect {
            x: area.x + area.width - 1,
            width: 1,
            height: rows_height as u16,
            ..area
        };
        let bar: Vec<Line> = (0..rows_height)
            .map(|_| Line::from(Span::styled("│", muted)))
            .collect();
        frame.render_widget(Paragraph::new(bar), border);
        let label = match self.tree.top().is_root() {
            true => self.snapshot.label(),
            false => format!("{}/", self.tree.top()),
        };
        let label = replor_render::text::into_visible(label);
        let footer = Rect {
            y: area.y + area.height.saturating_sub(1),
            height: 1,
            ..area
        };
        let text = view::elide_start(&label, area.width as usize);
        let pad =
            (area.width as usize).saturating_sub(replor_render::layout::wrap::display_width(&text));
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("{text}{}", " ".repeat(pad)),
                status,
            ))),
            footer,
        );
    }

    /// The images on screen this frame, placed inside the preview pane.
    pub fn images(&self) -> Vec<Placed> {
        if !self.state.shows_images() {
            return Vec::new();
        }
        let viewport = &self.state.viewport;
        let mut placed = images::visible(
            &self.rendered.lines,
            viewport.top,
            viewport.left,
            viewport.width,
            viewport.height,
        );
        for image in &mut placed {
            image.col += self.preview_area.x;
            image.row += self.preview_area.y;
        }
        placed
    }
}

/// A fixed share for the tree, which is enough for a path several directories
/// deep and leaves the preview most of the screen.
fn split(size: Rect) -> (Rect, Rect) {
    let tree = (size.width * 3 / 10).clamp(20, 40).min(size.width / 2);
    let left = Rect {
        width: tree,
        ..size
    };
    let right = Rect {
        x: size.x + tree,
        width: size.width - tree,
        ..size
    };
    (left, right)
}

/// Headings paired with the rendered line they start on. Rebuilt whenever the
/// surface is laid out again, since the line numbers change with the width.
fn outline(document: &Document, rendered: &RenderedDocument) -> Vec<OutlineItem> {
    headings(&document.blocks)
        .into_iter()
        .filter_map(|heading| {
            rendered_line_for_source(&rendered.lines, heading.range.line_start).map(|line| {
                OutlineItem {
                    level: heading.level,
                    text: replor_render::text::visible(&heading.plain).into_owned(),
                    line,
                }
            })
        })
        .collect()
}

pub fn run(mut app: App) -> Result<(), AppError> {
    let terminal_error = |source| AppError::Terminal { source };
    let mut guard = TerminalGuard::enter().map_err(terminal_error)?;
    let result = (|| {
        let backend = CrosstermBackend::new(std::io::stdout());
        let mut terminal = Terminal::new(backend).map_err(terminal_error)?;
        let size = terminal.size().map_err(terminal_error)?;
        app.resize(Rect::new(0, 0, size.width, size.height));
        // What the terminal is currently showing, so an unchanged frame does
        // not re-send image payloads that can be megabytes each.
        let mut images = ImageSession::new(app.options.images.map(|support| support.protocol));
        loop {
            let wanted = app.images();
            // Kitty removes its own placements. iTerm2 draws into the cells,
            // so the only way back is to make `ratatui` repaint everything.
            if images.needs_repaint(&wanted) {
                terminal.clear().map_err(terminal_error)?;
            }
            terminal
                .draw(|frame| app.draw(frame))
                .map_err(terminal_error)?;
            images.draw(wanted)?;

            if !event::poll(POLL).map_err(terminal_error)? {
                continue;
            }
            match event::read().map_err(terminal_error)? {
                Event::Key(key) if key.is_press() => match app.key(key) {
                    Outcome::Quit => return Ok(()),
                    Outcome::Replaced => images.release()?,
                    Outcome::Continue => {}
                },
                Event::Resize(columns, rows) => {
                    images.screen_cleared()?;
                    app.resize(Rect::new(0, 0, columns, rows));
                }
                _ => {}
            }
        }
    })();
    guard.leave();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pager::help::{ENTRIES, Pane};
    use replor_render::diagram::{MermaidMode, MermaidWidth};
    use replor_render::flavor::Flavor;
    use replor_render::layout::icons::IconSet;
    use replor_render::layout::theme::Variant;
    use replor_snapshot::WorktreeSnapshot;
    use std::path::Path;

    fn parse(name: &str) -> KeyEvent {
        if let Some(letter) = name.strip_prefix("Ctrl-") {
            let c = letter.chars().next().expect("a letter after Ctrl-");
            return KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        }
        let code = match name {
            "Space" => KeyCode::Char(' '),
            "Down" => KeyCode::Down,
            "Up" => KeyCode::Up,
            "Left" => KeyCode::Left,
            "Right" => KeyCode::Right,
            "PageDown" => KeyCode::PageDown,
            "PageUp" => KeyCode::PageUp,
            "Home" => KeyCode::Home,
            "End" => KeyCode::End,
            "Esc" => KeyCode::Esc,
            "Enter" => KeyCode::Enter,
            "Tab" => KeyCode::Tab,
            other => {
                let mut chars = other.chars();
                let c = chars.next().expect("a key name");
                assert!(chars.next().is_none(), "unknown key name: {other}");
                KeyCode::Char(c)
            }
        };
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// The overlay must not advertise a key that does nothing in the pane it
    /// is listed for.
    #[test]
    fn every_advertised_key_is_bound() {
        for entry in ENTRIES {
            let panes: &[Focus] = match entry.pane {
                Pane::Tree => &[Focus::Tree],
                Pane::Preview => &[Focus::Preview],
                Pane::Both => &[Focus::Tree, Focus::Preview],
            };
            for &focus in panes {
                for name in entry.keys {
                    assert_ne!(
                        route(focus, Mode::Normal, parse(name)),
                        Command::Ignored,
                        "{name} is listed in the help for {focus:?} but does nothing"
                    );
                }
            }
        }
    }

    #[test]
    fn the_same_letter_moves_the_focused_pane() {
        let j = parse("j");
        assert_eq!(route(Focus::Tree, Mode::Normal, j), Command::TreeMove(1));
        assert_eq!(
            route(Focus::Preview, Mode::Normal, j),
            Command::Preview(Input::Navigate(Action::LineDown))
        );
    }

    #[test]
    fn an_open_prompt_takes_letters_as_text() {
        for name in ["r", "q", "j"] {
            let c = name.chars().next().unwrap();
            assert_eq!(
                route(Focus::Preview, Mode::Search, parse(name)),
                Command::Preview(Input::SearchChar(c))
            );
        }
        assert_eq!(
            route(Focus::Tree, Mode::Help, parse("Esc")),
            Command::Preview(Input::HelpClose)
        );
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn app(dir: &Path, top: &str) -> App {
        let options = Options {
            flavor: Flavor::Gfm,
            mermaid: MermaidMode::Unicode,
            mermaid_width: MermaidWidth::Unicode,
            highlight: false,
            images: None,
            icons: IconSet::Unicode,
        };
        let snapshot = WorktreeSnapshot::open(dir).unwrap();
        App::new(
            Arc::new(snapshot),
            SnapshotPath::new(top).unwrap(),
            options,
            Theme::new(Variant::Dark),
            Rect::new(0, 0, 100, 30),
        )
    }

    fn text(app: &App) -> String {
        app.rendered()
            .lines
            .iter()
            .map(|line| line.text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn choosing_a_file_in_the_tree_shows_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        write(&dir.path().join("docs/a.md"), "# Alpha\n\nBody.\n");
        let mut app = app(dir.path(), "docs");
        assert!(text(&app).contains("Select a file"));
        assert_eq!(app.key(parse("Enter")), Outcome::Replaced);
        assert!(text(&app).contains("Alpha"), "{}", text(&app));
        // The preview scrolls only once it has the focus.
        assert_eq!(app.key(parse("Tab")), Outcome::Continue);
        assert_eq!(app.focus(), Focus::Preview);
        app.key(parse("G"));
        assert_eq!(app.key(parse("q")), Outcome::Quit);
    }

    #[test]
    fn l_opens_a_file_in_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        write(&dir.path().join("a.md"), "# Alpha\n");
        let mut app = app(dir.path(), "");
        assert_eq!(app.key(parse("l")), Outcome::Replaced);
        assert!(text(&app).contains("Alpha"), "{}", text(&app));
    }

    #[test]
    fn reload_shows_a_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        write(&dir.path().join("a.md"), "# Before\n");
        let mut app = app(dir.path(), "");
        app.open(&SnapshotPath::new("a.md").unwrap());
        write(&dir.path().join("a.md"), "# After\n");
        assert_eq!(app.key(parse("r")), Outcome::Replaced);
        assert!(text(&app).contains("After"), "{}", text(&app));
        assert_eq!(app.state().status.as_deref(), Some("reloaded"));
    }

    #[test]
    fn a_resize_lays_the_preview_out_at_the_pane_width() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        write(
            &dir.path().join("a.md"),
            &format!("{}\n", "word ".repeat(60)),
        );
        let mut app = app(dir.path(), "");
        app.open(&SnapshotPath::new("a.md").unwrap());
        let wide = app.rendered().lines.len();
        app.resize(Rect::new(0, 0, 50, 30));
        assert!(app.rendered().lines.len() > wide);
        assert!(app.state().viewport.width <= 30);
    }

    #[test]
    fn the_panes_share_the_screen() {
        let (tree, preview) = split(Rect::new(0, 0, 100, 30));
        assert_eq!((tree.width, preview.x, preview.width), (30, 30, 70));
        let (tree, preview) = split(Rect::new(0, 0, 30, 10));
        assert_eq!((tree.width, preview.width), (15, 15));
    }
}
