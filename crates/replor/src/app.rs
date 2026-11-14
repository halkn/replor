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
use ratatui::widgets::{Block, Clear, Paragraph};
use replor_render::error::AppError;
use replor_render::layout::theme::Theme;
use replor_render::layout::{LayoutOptions, RenderedDocument, StyleRole, layout_document};
use replor_render::markdown::model::{Document, headings};
use replor_render::navigation::{Anchor, Link, Target, anchors, links};
use replor_render::snapshot::{EntryKind, Snapshot, SnapshotPath, WalkLimits};

use crate::finder::{self, Candidates, Finder, Progress};
use crate::navigate::{self, Destination, History, Place};
use crate::pager::TerminalGuard;
use crate::pager::event::{Input, map};
use crate::pager::help;
use crate::pager::images::{self, ImageSession, Placed};
use crate::pager::search::SearchIndex;
use crate::pager::state::{
    Action, Mode, OutlineItem, PagerState, ViewportAnchor, rendered_line_for_position,
    rendered_line_for_source, source_line_at, source_position_at,
};
use crate::pager::view::{
    self, LinkBar, ViewContext, convert, elide_end, elide_start, help_rows, outline_window,
    overlay_area, widest_line,
};
use crate::preview::{self, Content, Options};
use crate::tree::{Opened, Tree};

/// How long a frame waits for input before looping again.
const POLL: Duration = Duration::from_millis(250);

/// Matches the finder shows at once, when the screen is tall enough.
const FINDER_ROWS: usize = 20;

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
    /// Opens the prompt that finds a file by its path.
    FindFile,
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
            (_, KeyCode::Char('p')) => Command::FindFile,
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
    /// The rendering of the open Markdown file while its source is shown,
    /// kept so going back does not read the file and render its diagrams
    /// again. Opening any file, by any means, goes back to the rendering,
    /// where its links can be followed.
    rendering: Option<Content>,
    /// After a switch between rendering and source: the top it put this face
    /// at, and where the other face was left. Switching back before scrolling
    /// returns there exactly, which a place in the file cannot always say: a
    /// block shown on fewer lines than it has cannot tell them all apart, a
    /// heading shares its range with its `[[_TOC_]]` entry, and the end of a
    /// document holds the top above where it was asked to be. Any other
    /// layout forgets it.
    switched: Option<(usize, ViewportAnchor)>,
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
    /// Every file, for the finder, listed again by `r`.
    candidates: Candidates,
    limits: WalkLimits,
    /// The finder while it is open. It owns every key then.
    finder: Option<Finder>,
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
        let candidates = Candidates::start(Arc::clone(&snapshot), finder::LIMITS);
        let mut app = Self {
            snapshot,
            tree,
            focus: Focus::Tree,
            options,
            theme,
            opened: None,
            rendering: None,
            switched: None,
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
            candidates,
            limits: finder::LIMITS,
            finder: None,
        };
        app.lay_out_fresh();
        app
    }

    /// Opens `path` in the preview and selects it in the tree.
    pub fn open(&mut self, path: &SnapshotPath) {
        let content = preview::open(&self.snapshot, path, &self.options);
        self.replace(Some(path.clone()), content);
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

    /// The line of the open file at the top of the preview. The fences around
    /// a file shown as code are no lines of it, and count as the nearest one.
    pub fn top_line(&self) -> Option<usize> {
        source_line_at(&self.rendered.lines, self.state.viewport.top).map(|line| {
            line.saturating_sub(self.content.line_offset)
                .clamp(1, self.content.lines.max(1))
        })
    }

    /// Lists the files for the finder again, within other limits.
    pub fn list_files_within(&mut self, limits: WalkLimits) {
        self.limits = limits;
        self.candidates = Candidates::start(Arc::clone(&self.snapshot), limits);
        self.refresh_finder(true);
    }

    /// Takes the files listed since the last frame. The frame loop calls it
    /// between key presses, so the listing never holds up a key.
    pub fn tick(&mut self) {
        if self.candidates.receive() {
            self.refresh_finder(false);
        }
    }

    /// The finder, if it is open.
    pub fn finder(&self) -> Option<&Finder> {
        self.finder.as_ref()
    }

    pub fn candidates(&self) -> &Candidates {
        &self.candidates
    }

    fn refresh_finder(&mut self, query_changed: bool) {
        if let Some(finder) = &mut self.finder {
            finder.refresh(self.candidates.entries(), query_changed);
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if self.finder.is_some() {
            return self.finder_key(key);
        }
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
            Command::FindFile => {
                self.finder = Some(Finder::default());
                self.refresh_finder(true);
            }
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

    /// A key while the finder is open. A file chosen there opens as one
    /// chosen in the tree does: the tree follows, and nothing is added to
    /// Back / Forward.
    fn finder_key(&mut self, key: KeyEvent) -> Outcome {
        let Some(finder) = &mut self.finder else {
            return Outcome::Continue;
        };
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if control => return Outcome::Quit,
            KeyCode::Esc => self.finder = None,
            KeyCode::Enter => {
                let chosen = finder.chosen(self.candidates.entries()).cloned();
                if let Some(entry) = chosen {
                    self.finder = None;
                    return self.show(Opened::File(entry));
                }
            }
            KeyCode::Up => finder.move_by(-1),
            KeyCode::Down => finder.move_by(1),
            KeyCode::Char('p') if control => finder.move_by(-1),
            KeyCode::Char('n') if control => finder.move_by(1),
            KeyCode::Backspace => {
                if finder.query.pop().is_some() {
                    self.refresh_finder(true);
                }
            }
            KeyCode::Char(c) if !control && !key.modifiers.contains(KeyModifiers::ALT) => {
                finder.query.push(c);
                self.refresh_finder(true);
            }
            _ => {}
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
                self.replace(Some(path), content);
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
            self.replace(Some(place.path.clone()), content);
        }
        self.scroll_to_file_line(place.line);
        self.leave_link();
        match replaced {
            true => Outcome::Replaced,
            false => Outcome::Continue,
        }
    }

    /// Where the reader is: the open file and its line at the top.
    fn current_place(&self) -> Option<Place> {
        Some(Place {
            path: self.opened.clone()?,
            line: self.top_line().unwrap_or(1),
        })
    }

    fn toggle_source(&mut self) -> Outcome {
        let Some(path) = self.opened.clone().filter(preview::is_markdown) else {
            return self.refuse("only Markdown has a source view".to_string());
        };
        let top = self.state.viewport.top;
        let back = self
            .switched
            .filter(|&(at, _)| at == top)
            .map(|(_, anchor)| anchor);
        let leaving = self.state.anchor(&self.rendered.lines);
        // Rendered lines differ between the two, so the place is kept in the
        // file, finer than a line within a block.
        let place = source_position_at(&self.rendered.lines, top)
            .map_or(1.0, |position| position - self.content.line_offset as f64);
        match self.rendering.take() {
            Some(rendering) => self.content = rendering,
            None => match preview::source(&self.content, &path, &self.options) {
                Ok(source) => self.rendering = Some(std::mem::replace(&mut self.content, source)),
                // The rendering stays, so the reader keeps their place.
                Err(reason) => return self.refuse(format!("cannot show the source: {reason}")),
            },
        }
        // Both faces are the same file, so a search on one goes on in the
        // other, as it does across `r` and a resize.
        let search = std::mem::take(&mut self.state.search);
        self.lay_out_fresh();
        let covered = match back {
            // The face is laid out as it was left, so its place is exactly
            // there, whatever the other face could tell apart.
            Some(anchor) => {
                self.state.restore_anchor(&self.rendered.lines, anchor);
                true
            }
            None => {
                let position = place + self.content.line_offset as f64;
                match rendered_line_for_position(&self.rendered.lines, position) {
                    Some((index, covered)) => {
                        self.scroll_to_index(index);
                        covered
                    }
                    None => false,
                }
            }
        };
        // A line no block covers moved to the next block, which is where the
        // reader now is, there and on the way back.
        self.switched = covered.then_some((self.state.viewport.top, leaving));
        self.state.carry_search(search, &self.texts);
        Outcome::Replaced
    }

    fn remember(&mut self) {
        if let Some(place) = self.current_place() {
            self.history.visit(place);
        }
    }

    /// A new surface in the preview: a file, or a notice when `path` is
    /// `None`. It always starts as the rendering.
    fn replace(&mut self, path: Option<SnapshotPath>, content: Content) {
        if let Some(path) = &path {
            self.tree.select(path);
        }
        self.content = content;
        self.opened = path;
        self.rendering = None;
        self.lay_out_fresh();
    }

    /// Scrolls to a line of the document, as its IR numbers them.
    fn scroll_to_source(&mut self, line: usize) {
        if let Some(index) = rendered_line_for_source(&self.rendered.lines, line) {
            self.scroll_to_index(index);
        }
    }

    fn scroll_to_index(&mut self, index: usize) {
        self.state.viewport.top = index.min(self.state.max_top());
        self.state.initial_line = Some(index);
    }

    /// Scrolls to a line of the open file.
    fn scroll_to_file_line(&mut self, line: usize) {
        self.scroll_to_source(line + self.content.line_offset);
    }

    /// Puts a file the tree handed over into the preview.
    fn show(&mut self, opened: Opened) -> Outcome {
        match opened {
            Opened::File(entry) if entry.kind == EntryKind::Submodule => {
                let notice = preview::notice("A submodule is another repository.", &self.options);
                self.replace(None, notice);
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
        self.list_files_within(self.limits);
        if let Some(path) = self.opened.clone() {
            let rendering = preview::open(&self.snapshot, &path, &self.options);
            let mut refused = None;
            // The source stays shown, unless the file has grown past what it
            // can show.
            self.content = match self.rendering.take() {
                None => rendering,
                Some(_) => match preview::source(&rendering, &path, &self.options) {
                    Ok(source) => {
                        self.rendering = Some(rendering);
                        source
                    }
                    Err(reason) => {
                        refused = Some(format!("cannot show the source: {reason}"));
                        rendering
                    }
                },
            };
            self.index();
            self.lay_out_again();
            self.state.status = Some("reloaded".to_string());
            if let Some(message) = refused {
                self.refuse(message);
            }
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
        self.switched = None;
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
        self.switched = None;
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
            top_line: self.top_line().unwrap_or(0),
            lines: self.content.lines,
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
        self.draw_finder(frame);
    }

    /// Drawn over both panes, like the heading list over the preview.
    fn draw_finder(&self, frame: &mut Frame) {
        let Some(finder) = &self.finder else {
            return;
        };
        let screen = frame.area();
        let body = Rect {
            height: screen.height.saturating_sub(1),
            ..screen
        };
        // A fixed size, so the box does not jump as the matches change.
        let area = overlay_area(body, 1 + FINDER_ROWS);
        let inner_width = area.width.saturating_sub(2) as usize;
        let inner_height = area.height.saturating_sub(2) as usize;
        let style = |role| convert(self.theme.style(role));
        let query = replor_render::text::visible(&finder.query);
        let prompt = elide_start(&format!("> {query}"), inner_width.saturating_sub(1));
        let mut rows = vec![Line::from(vec![
            Span::styled(prompt, style(StyleRole::Normal)),
            // The caret: the terminal cursor stays hidden.
            Span::styled(
                " ",
                style(StyleRole::Normal).add_modifier(Modifier::REVERSED),
            ),
        ])];
        let entries = self.candidates.entries();
        let window = outline_window(
            finder.selected,
            finder.matches.len(),
            inner_height.saturating_sub(1),
        );
        rows.extend(window.map(|index| {
            let path = entries[finder.matches[index]].entry.path.as_str();
            // A path is the snapshot's, and may hold terminal controls. The
            // start is cut, so the file name stays.
            let label = elide_start(&replor_render::text::visible(path), inner_width);
            let role = match index == finder.selected {
                true => StyleRole::SearchMatch,
                false => StyleRole::Normal,
            };
            Line::from(Span::styled(label, style(role)))
        }));
        if finder.matches.is_empty() && inner_height > 1 {
            rows.push(Line::from(Span::styled(
                "no match",
                style(StyleRole::Muted),
            )));
        }
        let progress = match self.candidates.progress() {
            Progress::Listing => " · listing…",
            Progress::Complete => "",
            Progress::Partial => " · partial: limit reached",
        };
        let title = format!("Files {}/{}{progress}", finder.matches.len(), entries.len());
        let block = Block::bordered()
            .title(title)
            .border_style(style(StyleRole::CodeBorder));
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(rows).block(block), area);
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
        // An image is drawn over the cells, so it would cover an overlay.
        if !self.state.shows_images() || self.finder.is_some() {
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
            app.tick();
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
            "Backspace" => KeyCode::Backspace,
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

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn listed(app: &mut App) {
        app.candidates.wait();
        app.refresh_finder(false);
    }

    fn screen(app: &mut App) -> String {
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn selected_in_tree(app: &App) -> &str {
        app.tree().rows()[app.tree().selected()].entry.path.as_str()
    }

    fn linked_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        write(&dir.path().join("a.md"), "# A\n\n[to b](b.md)\n");
        write(&dir.path().join("b.md"), "# Bravo\n");
        write(&dir.path().join("docs/guide/charlie.md"), "# Charlie\n");
        dir
    }

    #[test]
    fn a_file_found_by_its_path_opens_as_from_the_tree() {
        let dir = linked_fixture();
        let mut app = app(dir.path(), "");
        app.open(&SnapshotPath::new("a.md").unwrap());
        app.key(parse("Tab"));
        app.key(parse("f"));
        assert_eq!(app.key(parse("Enter")), Outcome::Replaced);
        assert_eq!(app.opened().unwrap().as_str(), "b.md");

        // From either pane.
        app.key(ctrl('p'));
        listed(&mut app);
        type_text(&mut app, "chr");
        let finder = app.finder().unwrap();
        assert_eq!(finder.query, "chr");
        assert_eq!(finder.matches.len(), 1);
        assert_eq!(app.key(parse("Enter")), Outcome::Replaced);
        assert!(app.finder().is_none());
        assert_eq!(app.opened().unwrap().as_str(), "docs/guide/charlie.md");
        assert!(text(&app).contains("Charlie"), "{}", text(&app));
        assert_eq!(selected_in_tree(&app), "docs/guide/charlie.md");

        // Only the link was recorded, so Back leaves `b.md` out.
        app.key(parse("["));
        assert_eq!(app.opened().unwrap().as_str(), "a.md");
        app.key(parse("]"));
        assert_eq!(app.opened().unwrap().as_str(), "docs/guide/charlie.md");
        app.key(parse("]"));
        assert_eq!(app.opened().unwrap().as_str(), "docs/guide/charlie.md");
    }

    #[test]
    fn the_finder_takes_every_key_and_esc_leaves_things_as_they_were() {
        let dir = linked_fixture();
        let mut app = app(dir.path(), "");
        assert_eq!(app.key(ctrl('p')), Outcome::Continue);
        listed(&mut app);
        // Keys that quit or reload elsewhere are text here.
        type_text(&mut app, "qr");
        assert_eq!(app.finder().unwrap().query, "qr");
        app.key(parse("Backspace"));
        app.key(parse("Backspace"));
        assert_eq!(app.finder().unwrap().matches.len(), 3);
        app.key(parse("Down"));
        app.key(ctrl('n'));
        app.key(ctrl('p'));
        assert_eq!(app.finder().unwrap().selected, 1);
        assert_eq!(app.key(parse("Esc")), Outcome::Continue);
        assert!(app.finder().is_none());
        assert!(app.opened().is_none());
        assert_eq!(app.focus(), Focus::Tree);
    }

    #[test]
    fn the_finder_shows_its_matches_and_a_partial_listing() {
        let dir = linked_fixture();
        let mut app = app(dir.path(), "");
        app.key(ctrl('p'));
        listed(&mut app);
        let drawn = screen(&mut app);
        assert!(drawn.contains("Files 3/3"), "{drawn}");
        assert!(drawn.contains("docs/guide/charlie.md"), "{drawn}");
        assert!(!drawn.contains("partial"), "{drawn}");
        type_text(&mut app, "zzz");
        assert!(screen(&mut app).contains("no match"));

        app.key(parse("Esc"));
        app.list_files_within(WalkLimits {
            entries: 2,
            depth: 64,
        });
        app.key(ctrl('p'));
        listed(&mut app);
        let drawn = screen(&mut app);
        assert!(
            drawn.contains("Files 2/2 · partial: limit reached"),
            "{drawn}"
        );
    }

    #[test]
    fn reload_lists_the_files_again() {
        let dir = linked_fixture();
        let mut app = app(dir.path(), "");
        listed(&mut app);
        write(&dir.path().join("delta.md"), "# Delta\n");
        app.key(parse("r"));
        app.key(ctrl('p'));
        listed(&mut app);
        type_text(&mut app, "delta");
        assert_eq!(app.finder().unwrap().matches.len(), 1);
    }

    #[test]
    fn the_panes_share_the_screen() {
        let (tree, preview) = split(Rect::new(0, 0, 100, 30));
        assert_eq!((tree.width, preview.x, preview.width), (30, 30, 70));
        let (tree, preview) = split(Rect::new(0, 0, 30, 10));
        assert_eq!((tree.width, preview.width), (15, 15));
    }
}
