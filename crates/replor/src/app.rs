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
use replor_render::navigation::{Anchor, Link, Target, anchors, links};
use replor_render::snapshot::{EntryKind, Snapshot, SnapshotPath};

use crate::navigate::{self, Destination, History, Place};
use crate::pager::TerminalGuard;
use crate::pager::event::{Input, map};
use crate::pager::help;
use crate::pager::images::{self, ImageSession, Placed};
use crate::pager::search::SearchIndex;
use crate::pager::state::{
    Action, Mode, OutlineItem, PagerState, rendered_line_for_source, source_line_at,
};
use crate::pager::view::{self, LinkBar, ViewContext, convert, elide_end, help_rows, widest_line};
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
    /// Selects the next (1) or previous (-1) link in the preview.
    SelectLink(isize),
    FollowLink,
    Back,
    Forward,
    /// Shows a Markdown file as its text, or its text as Markdown again.
    ToggleSource,
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
        KeyCode::Char('[') => return Command::Back,
        KeyCode::Char(']') => return Command::Forward,
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
        Focus::Preview => match key.code {
            KeyCode::Char('f') => Command::SelectLink(1),
            KeyCode::Char('F') => Command::SelectLink(-1),
            KeyCode::Enter => Command::FollowLink,
            KeyCode::Char('s') => Command::ToggleSource,
            _ => match map(key, mode) {
                Input::Ignored => Command::Ignored,
                input => Command::Preview(input),
            },
        },
    }
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Back,
    Forward,
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
    /// The open Markdown file is shown as its text. Opening any file, by any
    /// means, goes back to the rendering, where its links can be followed.
    source_view: bool,
    content: Content,
    rendered: RenderedDocument,
    texts: SearchIndex,
    state: PagerState,
    tree_area: Rect,
    preview_area: Rect,
    history: History,
    /// The open document's links and headings, from its IR.
    links: Vec<Link>,
    anchors: Vec<Anchor>,
    selected_link: Option<usize>,
    link_bar: Option<LinkBar>,
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
            source_view: false,
            content,
            rendered: RenderedDocument::default(),
            texts: SearchIndex::default(),
            state: PagerState::new(0, 0, 1, 1),
            tree_area,
            preview_area,
            history: History::default(),
            links: Vec::new(),
            anchors: Vec::new(),
            selected_link: None,
            link_bar: None,
        };
        app.lay_out_fresh();
        app
    }

    /// Opens `path` in the preview and selects it in the tree.
    pub fn open(&mut self, path: &SnapshotPath) {
        self.tree.select(path);
        self.content = preview::open(&self.snapshot, path, &self.options);
        self.opened = Some(path.clone());
        self.source_view = false;
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

    /// The file in the preview, if it shows one.
    pub fn opened(&self) -> Option<&SnapshotPath> {
        self.opened.as_ref()
    }

    /// The selected link's destination, or why a link was not followed.
    pub fn link_bar(&self) -> Option<&LinkBar> {
        self.link_bar.as_ref()
    }

    /// The line of the open file at the top of the preview.
    pub fn top_line(&self) -> Option<usize> {
        source_line_at(&self.rendered.lines, self.state.viewport.top)
            .map(|line| line.saturating_sub(self.content.line_offset).max(1))
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
            Command::SelectLink(step) => self.select_link(step),
            Command::FollowLink => return self.follow(),
            Command::Back => return self.go(Direction::Back),
            Command::Forward => return self.go(Direction::Forward),
            Command::ToggleSource => return self.toggle_source(),
            Command::Preview(input) => {
                // Any other move leaves the link that was selected.
                if matches!(input, Input::Navigate(_)) {
                    self.selected_link = None;
                    self.link_bar = None;
                }
                if !self.apply(input) {
                    return Outcome::Quit;
                }
            }
            Command::Ignored => {}
        }
        Outcome::Continue
    }

    /// The rendered line a link is drawn on, found by the source range the
    /// IR gave it rather than by searching the text. A TOC entry carries its
    /// heading's range, so it is looked for after the TOC's first line.
    fn link_line(&self, link: &Link) -> Option<usize> {
        let lines = &self.rendered.lines;
        let from = match link.toc {
            Some(toc) => lines.iter().position(|l| l.source_range == Some(toc))? + 1,
            None => 0,
        };
        (from..lines.len()).find(|&i| lines[i].source_range == Some(link.range))
    }

    fn select_link(&mut self, step: isize) {
        if self.links.is_empty() {
            self.refuse("no links in this file".to_string());
            return;
        }
        let total = self.links.len();
        let top = self.state.viewport.top;
        // The link bar covers the last row while a link is selected.
        let bottom = top + self.state.viewport.height.saturating_sub(1).max(1);
        let next = match self.selected_link {
            Some(index) => (index as isize + step).rem_euclid(total as isize) as usize,
            // The first selection starts from what is on screen.
            None => {
                let lines: Vec<Option<usize>> =
                    self.links.iter().map(|link| self.link_line(link)).collect();
                match step > 0 {
                    true => lines.iter().position(|l| l.is_some_and(|l| l >= top)),
                    false => lines.iter().rposition(|l| l.is_some_and(|l| l < bottom)),
                }
                .unwrap_or(if step > 0 { 0 } else { total - 1 })
            }
        };
        self.selected_link = Some(next);
        if let Some(line) = self.link_line(&self.links[next])
            && (line < top || line >= bottom)
        {
            self.state.viewport.top = line
                .saturating_sub(self.state.viewport.height / 3)
                .min(self.state.max_top());
        }
        let link = &self.links[next];
        let text = match &link.target {
            Target::Dest(dest) => dest.clone(),
            Target::Heading(_) => format!("heading: {}", link.label),
        };
        self.link_bar = Some(LinkBar {
            text: replor_render::text::into_visible(text),
            position: format!("{}/{total}", next + 1),
            refused: false,
        });
    }

    fn follow(&mut self) -> Outcome {
        let Some(link) = self.selected_link.map(|index| self.links[index].clone()) else {
            return self.refuse("select a link with f first".to_string());
        };
        let dest = match link.target {
            Target::Heading(range) => {
                self.remember();
                self.scroll_to_source(range.line_start);
                self.leave_link();
                return Outcome::Continue;
            }
            Target::Dest(dest) => dest,
        };
        let source = self.content.document.inline().source.clone();
        let destination = match navigate::resolve(source.as_ref(), &dest) {
            Ok(destination) => destination,
            Err(reason) => return self.refuse(format!("cannot follow {dest}: {reason}")),
        };
        match destination {
            Destination::External(url) => self.refuse(format!("not opened: {url}")),
            Destination::Here(fragment) => match navigate::find_anchor(&self.anchors, &fragment) {
                Some(anchor) => {
                    let line = anchor.range.line_start;
                    self.remember();
                    self.scroll_to_source(line);
                    self.leave_link();
                    Outcome::Continue
                }
                None => self.refuse(format!("no heading #{fragment}")),
            },
            Destination::File { path, fragment } => {
                let content = preview::open(&self.snapshot, &path, &self.options);
                // The heading is looked for before anything moves, so a
                // missing one leaves the reader where they are.
                let line = match &fragment {
                    Some(fragment) => {
                        let found = anchors(content.document.document());
                        match navigate::find_anchor(&found, fragment) {
                            Some(anchor) => Some(anchor.range.line_start),
                            None => {
                                return self.refuse(format!("{path} has no heading #{fragment}"));
                            }
                        }
                    }
                    None => None,
                };
                self.remember();
                self.replace(path, content);
                if let Some(line) = line {
                    self.scroll_to_source(line);
                }
                Outcome::Replaced
            }
        }
    }

    /// Says in the link bar why nothing moved. The selection stays, so the
    /// reader can pick another link from where they are.
    fn refuse(&mut self, message: String) -> Outcome {
        let position = match self.selected_link {
            Some(index) => format!("{}/{}", index + 1, self.links.len()),
            None => String::new(),
        };
        self.link_bar = Some(LinkBar {
            text: replor_render::text::into_visible(message),
            position,
            refused: true,
        });
        Outcome::Continue
    }

    fn leave_link(&mut self) {
        self.selected_link = None;
        self.link_bar = None;
    }

    fn go(&mut self, direction: Direction) -> Outcome {
        let place = self.current_place().and_then(|current| match direction {
            Direction::Back => self.history.back(current),
            Direction::Forward => self.history.forward(current),
        });
        let Some(place) = place else {
            let message = match direction {
                Direction::Back => "nothing to go back to",
                Direction::Forward => "nothing to go forward to",
            };
            return self.refuse(message.to_string());
        };
        let replaced = self.opened.as_ref() != Some(&place.path);
        if replaced {
            let content = preview::open(&self.snapshot, &place.path, &self.options);
            self.replace(place.path.clone(), content);
        }
        self.scroll_to_file_line(place.line);
        self.leave_link();
        match replaced {
            true => Outcome::Replaced,
            false => Outcome::Continue,
        }
    }

    /// Where the reader is: the open file and the source line at the top.
    fn current_place(&self) -> Option<Place> {
        Some(Place {
            path: self.opened.clone()?,
            line: self.top_line().unwrap_or(1),
        })
    }

    fn toggle_source(&mut self) -> Outcome {
        let Some(path) = self.opened.clone().filter(preview::is_markdown) else {
            self.state.status = Some("only Markdown has a source view".to_string());
            return Outcome::Continue;
        };
        // Rendered lines differ between the two, so the place is kept as a
        // line of the file.
        let line = self.top_line().unwrap_or(1);
        self.source_view = !self.source_view;
        self.content = self.load(&path);
        self.lay_out_fresh();
        self.scroll_to_file_line(line);
        Outcome::Replaced
    }

    /// The open file in the view the reader chose.
    fn load(&self, path: &SnapshotPath) -> Content {
        match self.source_view {
            true => preview::source(&self.snapshot, path, &self.options),
            false => preview::open(&self.snapshot, path, &self.options),
        }
    }

    fn remember(&mut self) {
        if let Some(place) = self.current_place() {
            self.history.visit(place);
        }
    }

    fn replace(&mut self, path: SnapshotPath, content: Content) {
        self.tree.select(&path);
        self.content = content;
        self.opened = Some(path);
        self.source_view = false;
        self.lay_out_fresh();
    }

    /// Scrolls to a line of the document, as its IR numbers them.
    fn scroll_to_source(&mut self, line: usize) {
        if let Some(index) = rendered_line_for_source(&self.rendered.lines, line) {
            self.state.viewport.top = index.min(self.state.max_top());
            self.state.initial_line = Some(index);
        }
    }

    /// Scrolls to a line of the open file.
    fn scroll_to_file_line(&mut self, line: usize) {
        self.scroll_to_source(line + self.content.line_offset);
    }

    /// Puts a file the tree handed over into the preview.
    fn show(&mut self, opened: Opened) -> Outcome {
        match opened {
            Opened::File(entry) if entry.kind == EntryKind::Submodule => {
                self.content = preview::notice("A submodule is another repository.", &self.options);
                self.opened = None;
                self.source_view = false;
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
            self.content = self.load(&path);
            self.index();
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

    fn index(&mut self) {
        let document = self.content.document.document();
        self.links = links(document);
        self.anchors = anchors(document);
        self.selected_link = None;
        self.link_bar = None;
    }

    /// A new surface in the preview, read from the top.
    fn lay_out_fresh(&mut self) {
        self.index();
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
            selected_link: self.selected_link.map(|index| self.links[index].key),
            link_bar: self.link_bar.as_ref(),
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
