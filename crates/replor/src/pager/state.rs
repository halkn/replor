//! Pager view state and the pure transitions the key map drives.
//!
//! The rendered document is owned by the app, not by this state, so scrolling
//! and searching never clone the surface.

use super::search_session::SearchSession;
use crate::pager::search::{Search, SearchIndex};
use replor_render::layout::RenderedLine;
use replor_render::source::SourceRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    LineDown,
    LineUp,
    HalfPageDown,
    HalfPageUp,
    PageDown,
    PageUp,
    Top,
    Bottom,
    ScrollLeft,
    ScrollRight,
    ResetHorizontal,
    StartSearch,
    NextMatch,
    PreviousMatch,
    ToggleOutline,
    ToggleHelp,
    Quit,
}

/// An edit to the search prompt. The bindings match the readline keys a shell
/// already gives the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEdit {
    Backspace,
    Delete,
    DeleteWordBefore,
    KillToStart,
    KillToEnd,
    CaretLeft,
    CaretRight,
    CaretStart,
    CaretEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    Outline,
    Help,
}

#[derive(Debug, Clone)]
enum Interaction {
    Normal,
    Search(SearchSession),
    Outline,
    Help,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewportAnchor {
    top: Option<Anchor>,
    search: Option<Anchor>,
}

/// A rendered line identified by the source it shows, so it can be found again
/// in a layout of another width or of a reloaded file.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    range: SourceRange,
    /// Which run showing `range` this is, counted from the top. A heading and
    /// its `[[_TOC_]]` entry share a range, as do a list item's separators.
    occurrence: usize,
    /// Position within the run of lines that share `range`. A wrapped
    /// paragraph is one source range however many lines it takes, so the
    /// position is kept in proportion to the run's length.
    offset: usize,
    len: usize,
}

/// One heading in the outline overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineItem {
    pub level: u8,
    pub text: String,
    /// Rendered line the heading starts on. Resolved again after a re-layout.
    pub line: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    pub items: Vec<OutlineItem>,
    pub selected: usize,
}

impl Outline {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn move_by(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() - 1;
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    /// Select the last heading at or above `line`, so opening the overlay lands
    /// on the section currently being read.
    fn select_for_line(&mut self, line: usize) {
        self.selected = self
            .items
            .iter()
            .rposition(|item| item.line <= line)
            .unwrap_or(0);
    }
}

/// How far a single horizontal scroll step moves.
const HORIZONTAL_STEP: usize = 8;

#[derive(Debug, Clone)]
pub struct Viewport {
    pub top: usize,
    pub left: usize,
    /// Rows available for the document, excluding the status bar.
    pub height: usize,
    pub width: usize,
    pub total: usize,
    pub widest: usize,
}

#[derive(Debug, Clone)]
pub struct PagerState {
    pub viewport: Viewport,
    interaction: Interaction,
    pub search: Search,
    pub status: Option<String>,
    /// Rendered line opened by `--line`, highlighted until the reader moves.
    pub initial_line: Option<usize>,
    pub outline: Outline,
    /// First help entry on screen. A terminal too short for the whole list must
    /// still be able to reach the last line of it.
    pub help_top: usize,
}

impl PagerState {
    pub fn new(total: usize, widest: usize, height: usize, width: usize) -> Self {
        Self {
            viewport: Viewport {
                top: 0,
                left: 0,
                height: height.max(1),
                width: width.max(1),
                total,
                widest,
            },
            interaction: Interaction::Normal,
            search: Search::default(),
            status: None,
            initial_line: None,
            outline: Outline::default(),
            help_top: 0,
        }
    }

    pub fn mode(&self) -> Mode {
        match self.interaction {
            Interaction::Normal => Mode::Normal,
            Interaction::Search(_) => Mode::Search,
            Interaction::Outline => Mode::Outline,
            Interaction::Help => Mode::Help,
        }
    }

    /// Whether images are drawn this frame. They are written after `ratatui`
    /// has drawn the frame and kitty layers them above the text, so an overlay
    /// would be hidden under any image it covers.
    pub fn shows_images(&self) -> bool {
        matches!(self.mode(), Mode::Normal | Mode::Search)
    }

    pub fn search_session(&self) -> Option<&SearchSession> {
        match &self.interaction {
            Interaction::Search(session) => Some(session),
            _ => None,
        }
    }

    fn search_session_mut(&mut self) -> Option<&mut SearchSession> {
        match &mut self.interaction {
            Interaction::Search(session) => Some(session),
            _ => None,
        }
    }

    pub fn anchor(&self, lines: &[RenderedLine]) -> ViewportAnchor {
        ViewportAnchor {
            top: anchor_at(lines, self.viewport.top),
            search: self
                .search_session()
                .and_then(|session| anchor_at(lines, session.origin.0)),
        }
    }

    pub fn restore_anchor(&mut self, lines: &[RenderedLine], anchor: ViewportAnchor) {
        self.initial_line = None;
        if let Some(index) = anchor.top.and_then(|anchor| line_for_anchor(lines, anchor)) {
            self.viewport.top = index.min(self.max_top());
        }
        let max_top = self.max_top();
        let max_left = self.max_left();
        if let Some(session) = self.search_session_mut() {
            if let Some(index) = anchor
                .search
                .and_then(|anchor| line_for_anchor(lines, anchor))
            {
                session.origin.0 = index.min(max_top);
            }
            session.origin.0 = session.origin.0.min(max_top);
            session.origin.1 = session.origin.1.min(max_left);
        }
    }

    pub fn max_top(&self) -> usize {
        self.viewport.total.saturating_sub(self.viewport.height)
    }

    pub fn max_left(&self) -> usize {
        self.viewport.widest.saturating_sub(self.viewport.width)
    }

    pub fn clamp(&mut self) {
        self.viewport.top = self.viewport.top.min(self.max_top());
        self.viewport.left = self.viewport.left.min(self.max_left());
    }

    pub fn percentage(&self) -> usize {
        if self.viewport.total <= self.viewport.height {
            return 100;
        }
        let last = (self.viewport.top + self.viewport.height).min(self.viewport.total);
        last * 100 / self.viewport.total
    }

    /// Apply a navigation action. Returns `false` when the pager should exit.
    pub fn apply(&mut self, action: Action) -> bool {
        // Any deliberate movement retires the `--line` highlight.
        if !matches!(
            action,
            Action::Quit | Action::StartSearch | Action::ToggleOutline | Action::ToggleHelp
        ) {
            self.initial_line = None;
        }
        self.status = None;
        match action {
            Action::Quit => return false,
            Action::ToggleOutline => self.toggle_outline(),
            Action::ToggleHelp => self.toggle_help(),
            Action::LineDown => self.scroll_down(1),
            Action::LineUp => self.scroll_up(1),
            Action::HalfPageDown => self.scroll_down(self.viewport.height.div_ceil(2)),
            Action::HalfPageUp => self.scroll_up(self.viewport.height.div_ceil(2)),
            Action::PageDown => self.scroll_down(self.viewport.height),
            Action::PageUp => self.scroll_up(self.viewport.height),
            Action::Top => self.viewport.top = 0,
            Action::Bottom => self.viewport.top = self.max_top(),
            Action::ScrollLeft => {
                self.viewport.left = self.viewport.left.saturating_sub(HORIZONTAL_STEP)
            }
            Action::ScrollRight => {
                self.viewport.left = (self.viewport.left + HORIZONTAL_STEP).min(self.max_left());
            }
            Action::ResetHorizontal => self.viewport.left = 0,
            Action::StartSearch => {
                self.interaction =
                    Interaction::Search(SearchSession::new(self.viewport.top, self.viewport.left));
            }
            Action::NextMatch => self.jump(true),
            Action::PreviousMatch => self.jump(false),
        }
        true
    }

    fn scroll_down(&mut self, amount: usize) {
        self.viewport.top = (self.viewport.top + amount).min(self.max_top());
    }

    fn scroll_up(&mut self, amount: usize) {
        self.viewport.top = self.viewport.top.saturating_sub(amount);
    }

    fn jump(&mut self, forward: bool) {
        if !self.search.is_active() {
            return;
        }
        let before = self.search.position();
        let aimed = self.search.is_aimed();
        let top = self.viewport.top;
        let found = if forward {
            self.search.next()
        } else {
            self.search.previous()
        };
        match found {
            Some(m) => {
                self.reveal(m.line);
                // Cycling past either end is easy to mistake for "no more
                // matches", so say that it happened. From an aimed match the
                // reader stands at the top of the screen, not on a match.
                let wrapped = if aimed {
                    forward && m.line < top || !forward && m.line >= top
                } else {
                    matches!((before, self.search.position()), (Some(before), Some(after))
                        if forward && after <= before || !forward && after >= before)
                };
                if wrapped && self.search.count() > 1 {
                    self.status = Some("wrapped".to_string());
                }
            }
            None => self.status = Some(format!("no match: {}", self.search.query())),
        }
    }

    pub fn visible_search(&self) -> &Search {
        self.search_session()
            .map_or(&self.search, |session| &session.preview)
    }

    #[cfg(test)]
    fn preview_count(&self) -> usize {
        self.search_session()
            .map_or(0, |session| session.preview.count())
    }

    pub fn insert_search_char(&mut self, c: char, lines: &SearchIndex) {
        if c.is_control() {
            return;
        }
        if let Some(session) = self.search_session_mut() {
            session.insert(c);
            self.update_preview(lines);
        }
    }

    pub fn edit_search(&mut self, edit: SearchEdit, lines: &SearchIndex) {
        if let Some(session) = self.search_session_mut()
            && session.edit(edit)
        {
            self.update_preview(lines);
        }
    }

    fn update_preview(&mut self, lines: &SearchIndex) {
        let top = self.restore_origin();
        self.status = None;
        if let Some(session) = self.search_session_mut() {
            session.preview.replace_query(&session.input, lines);
            session.preview.select_from(top);
            if let Some(found) = session.preview.current() {
                self.initial_line = None;
                self.reveal(found.line);
            }
        }
    }

    pub fn recompute_searches(&mut self, lines: &SearchIndex) {
        self.search.recompute(lines);
        self.search.select_from(self.viewport.top);
        let top = self.viewport.top;
        if let Some(session) = self.search_session_mut() {
            session.preview.recompute(lines);
            session.preview.select_from(top);
        }
    }

    /// A confirmed search carried over to another surface, its matches
    /// counted again there without moving to one: the place is kept by other
    /// means. `n` and `N` then go to the nearest matches below and above the
    /// top. A query with no match there says so, as confirming it would.
    pub fn carry_search(&mut self, search: Search, lines: &SearchIndex) {
        self.search = search;
        self.recompute_searches(lines);
        self.search.aim_from(self.viewport.top);
        // Any status was said of the surface left behind.
        self.status = (self.search.is_active() && self.search.count() == 0)
            .then(|| format!("no match: {}", self.search.query()));
    }

    pub fn confirm_search(&mut self, lines: &SearchIndex) {
        let Interaction::Search(session) =
            std::mem::replace(&mut self.interaction, Interaction::Normal)
        else {
            return;
        };
        self.search.set_query(&session.input, lines);
        if !self.search.is_active() {
            return;
        }
        self.search.select_from(self.viewport.top);
        match self.search.current() {
            Some(found) => {
                self.initial_line = None;
                self.reveal(found.line);
            }
            None => self.status = Some(format!("no match: {}", self.search.query())),
        }
    }

    pub fn cancel_search(&mut self) {
        self.restore_origin();
        self.interaction = Interaction::Normal;
    }

    fn restore_origin(&mut self) -> usize {
        if let Some(session) = self.search_session() {
            let (top, left) = session.origin;
            self.viewport.top = top.min(self.max_top());
            self.viewport.left = left.min(self.max_left());
        }
        self.viewport.top
    }

    fn toggle_outline(&mut self) {
        if self.mode() == Mode::Outline {
            self.interaction = Interaction::Normal;
            return;
        }
        if self.outline.is_empty() {
            self.status = Some("no headings".to_string());
            return;
        }
        self.outline.select_for_line(self.viewport.top);
        self.interaction = Interaction::Outline;
    }

    fn toggle_help(&mut self) {
        self.interaction = match self.mode() {
            Mode::Help => Interaction::Normal,
            _ => {
                self.help_top = 0;
                Interaction::Help
            }
        };
    }

    /// Scroll the key list. `visible` is how many entries fit in the overlay.
    pub fn scroll_help(&mut self, delta: isize, total: usize, visible: usize) {
        let last = total.saturating_sub(visible);
        self.help_top = self.help_top.saturating_add_signed(delta).min(last);
    }

    pub fn close_help(&mut self) {
        self.interaction = Interaction::Normal;
    }

    pub fn move_outline(&mut self, delta: isize) {
        self.outline.move_by(delta);
    }

    /// Jump to the selected heading and close the overlay.
    pub fn confirm_outline(&mut self) {
        self.interaction = Interaction::Normal;
        let Some(item) = self.outline.items.get(self.outline.selected) else {
            return;
        };
        let line = item.line;
        self.initial_line = None;
        // A heading is easier to read from the top of the screen than centred.
        self.viewport.top = line.min(self.max_top());
    }

    pub fn cancel_outline(&mut self) {
        self.interaction = Interaction::Normal;
    }

    /// Rebuild the outline after a re-layout, keeping the current selection.
    pub fn set_outline(&mut self, items: Vec<OutlineItem>) {
        let selected = self.outline.selected;
        self.outline = Outline {
            selected: selected.min(items.len().saturating_sub(1)),
            items,
        };
        if self.outline.is_empty() && self.mode() == Mode::Outline {
            self.interaction = Interaction::Normal;
        }
    }

    /// Bring `line` into view, centring it when it is off screen.
    pub fn reveal(&mut self, line: usize) {
        if line < self.viewport.top || line >= self.viewport.top + self.viewport.height {
            self.viewport.top = line
                .saturating_sub(self.viewport.height / 2)
                .min(self.max_top());
        }
    }

    /// Re-fit the viewport after a resize or re-layout, keeping `anchor` at the
    /// top of the screen.
    pub fn resize(&mut self, total: usize, widest: usize, height: usize, width: usize) {
        self.viewport.total = total;
        self.viewport.widest = widest;
        self.viewport.height = height.max(1);
        self.viewport.width = width.max(1);
        self.clamp();
    }
}

/// The source line shown at the top of the viewport, for the status bar.
pub fn source_line_at(lines: &[RenderedLine], index: usize) -> Option<usize> {
    anchor_at(lines, index).map(|anchor| anchor.range.line_start)
}

/// The anchor for the rendered line at `index`. A line with no source, such as
/// a blank separator, is anchored to the nearest mapped line after it, or
/// before it at the end of the document.
fn anchor_at(lines: &[RenderedLine], index: usize) -> Option<Anchor> {
    let mapped = |i: &usize| lines[*i].source_range.is_some();
    let index = (index..lines.len())
        .find(mapped)
        .or_else(|| (0..index.min(lines.len())).rev().find(mapped))?;
    let range = lines[index].source_range?;
    let (start, len) = run(lines, index, range);
    let occurrence = run_starts(lines, range)
        .take_while(|&other| other < start)
        .count();
    Some(Anchor {
        range,
        occurrence,
        offset: index - start,
        len,
    })
}

/// Where `anchor` is in `lines`: the same run when it is still there, else the
/// line covering where its range started, which is what a reload that moved
/// the bytes still finds.
fn line_for_anchor(lines: &[RenderedLine], anchor: Anchor) -> Option<usize> {
    let first = run_starts(lines, anchor.range)
        .nth(anchor.occurrence)
        .or_else(|| rendered_line_for_source(lines, anchor.range.line_start))?;
    let range = lines[first].source_range?;
    if range.line_start != anchor.range.line_start {
        return Some(first);
    }
    let (start, len) = run(lines, first, range);
    Some(start + anchor.offset * len / anchor.len)
}

/// First index and length of the run of lines around `index` that show `range`.
fn run(lines: &[RenderedLine], index: usize, range: SourceRange) -> (usize, usize) {
    let same = |i: usize| lines[i].source_range == Some(range);
    let start = (0..index)
        .rev()
        .take_while(|&i| same(i))
        .last()
        .unwrap_or(index);
    let end = (index..lines.len())
        .take_while(|&i| same(i))
        .last()
        .unwrap_or(index);
    (start, end - start + 1)
}

/// Where each run of lines showing `range` begins, in order.
fn run_starts(lines: &[RenderedLine], range: SourceRange) -> impl Iterator<Item = usize> + '_ {
    let same = move |i: usize| lines[i].source_range == Some(range);
    (0..lines.len()).filter(move |&i| same(i) && (i == 0 || !same(i - 1)))
}

/// Where the rendered line at `index` is in the source, finer than a line:
/// within a block shown on several lines, the line's place in its run laid
/// over the block's source lines. Both ways the middle of a share is taken,
/// so where there are more rendered lines than source lines, going and
/// coming back lands on the same line. A run not laid over its lines is at
/// the middle of its first line (`proportional`).
pub fn source_position_at(lines: &[RenderedLine], index: usize) -> Option<f64> {
    let anchor = anchor_at(lines, index)?;
    let range = anchor.range;
    let start = range.line_start as f64;
    if !proportional(lines, range, anchor.len) {
        return Some(start + 0.5);
    }
    let count = (range.line_end - range.line_start + 1) as f64;
    Some(start + (anchor.offset as f64 + 0.5) * count / anchor.len as f64)
}

/// The rendered line for a source position from `source_position_at`, and
/// whether a block covers it. In the block `rendered_line_for_source` picks,
/// the position is laid over its run of lines the same way back, so a place
/// in the middle of a paragraph stays there. A line no block covers goes
/// where `rendered_line_for_source` sends it.
pub fn rendered_line_for_position(lines: &[RenderedLine], position: f64) -> Option<(usize, bool)> {
    let line = position.max(1.0) as usize;
    let first = rendered_line_for_source(lines, line)?;
    let Some(range) = lines[first].source_range else {
        return Some((first, false));
    };
    if !(range.line_start..=range.line_end).contains(&line) {
        return Some((first, false));
    }
    let (start, len) = run(lines, first, range);
    if !proportional(lines, range, len) {
        return Some((start, true));
    }
    let count = (range.line_end - range.line_start + 1) as f64;
    let offset = ((position - range.line_start as f64) * len as f64 / count) as usize;
    Some((start + offset.min(len - 1), true))
}

/// Whether a run of `len` lines showing `range` is the block's own text laid
/// over its lines. A container's lines (a `<details>` summary, a list item's
/// separators) carry its whole range though its content is elsewhere, so a
/// range with another inside it is kept at its start.
fn proportional(lines: &[RenderedLine], range: SourceRange, len: usize) -> bool {
    len > 1
        && !lines
            .iter()
            .filter_map(|line| line.source_range)
            .any(|other| {
                other != range
                    && range.line_start <= other.line_start
                    && other.line_end <= range.line_end
            })
}

/// Rendered line to open for a 1-based source line: the narrowest line covering
/// it, else the nearest following mapped line, else the nearest preceding one.
///
/// Narrowest rather than first, because a container's separator and title
/// lines carry the whole container's range and come before its content.
pub fn rendered_line_for_source(lines: &[RenderedLine], source_line: usize) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    if let Some((index, _)) = lines
        .iter()
        .enumerate()
        .filter_map(|(index, l)| l.source_range.map(|r| (index, r)))
        .filter(|(_, r)| r.line_start <= source_line && source_line <= r.line_end)
        .min_by_key(|(_, r)| r.line_end - r.line_start)
    {
        return Some(index);
    }
    if let Some(index) = lines
        .iter()
        .position(|l| l.source_range.is_some_and(|r| r.line_start > source_line))
    {
        return Some(index);
    }
    lines
        .iter()
        .rposition(|l| l.source_range.is_some_and(|r| r.line_end < source_line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replor_render::layout::{RenderedLine, RenderedSpan, StyleRole};
    use replor_render::source::SourceRange;

    #[test]
    fn cancelling_after_relayout_restores_the_source_position() {
        let old = vec![mapped(1, 1), mapped(2, 2), mapped(3, 3), mapped(4, 4)];
        let mut state = PagerState::new(old.len(), 10, 1, 40);
        state.viewport.top = 2;
        state.apply(Action::StartSearch);
        state.viewport.top = 0;
        let anchor = state.anchor(&old);
        let new = vec![
            mapped(1, 1),
            mapped(1, 1),
            mapped(2, 2),
            mapped(2, 2),
            mapped(3, 3),
            mapped(4, 4),
        ];
        state.resize(new.len(), 10, 1, 40);
        state.restore_anchor(&new, anchor);
        state.cancel_search();
        assert_eq!(state.viewport.top, 4);
        assert_eq!(source_line_at(&new, state.viewport.top), Some(3));
    }

    fn lines_of(line_start: usize, line_end: usize, count: usize) -> Vec<RenderedLine> {
        vec![mapped(line_start, line_end); count]
    }

    fn restored(old: &[RenderedLine], top: usize, new: &[RenderedLine]) -> usize {
        let mut state = PagerState::new(old.len(), 10, 1, 40);
        state.viewport.top = top;
        let anchor = state.anchor(old);
        state.resize(new.len(), 10, 1, 40);
        state.restore_anchor(new, anchor);
        state.viewport.top
    }

    #[test]
    fn a_relayout_keeps_the_position_inside_a_block_in_proportion() {
        let old = [lines_of(1, 1, 4), lines_of(3, 6, 10)].concat();
        let new = [lines_of(1, 1, 2), lines_of(3, 6, 20)].concat();
        // Six lines into a ten-line paragraph is six tenths of the way through it.
        assert_eq!(restored(&old, 4 + 6, &new), 2 + 12);
        // The last line stays the last line rather than spilling into the next block.
        let after = [lines_of(1, 1, 2), lines_of(3, 6, 3), lines_of(8, 8, 5)].concat();
        assert_eq!(restored(&old, 4 + 9, &after), 2 + 2);
    }

    #[test]
    fn a_reload_that_shifts_bytes_still_finds_the_block_by_its_line() {
        let block = |byte_start| SourceRange {
            byte_start,
            byte_end: byte_start + 10,
            line_start: 5,
            line_end: 8,
        };
        let line = |range| RenderedLine {
            source_range: Some(range),
            ..mapped(0, 0)
        };
        let old = [lines_of(1, 1, 2), vec![line(block(10)); 4]].concat();
        let new = [lines_of(1, 1, 2), vec![line(block(14)); 4]].concat();
        assert_eq!(restored(&old, 2 + 3, &new), 2 + 3);
    }

    fn layout(markdown: &str, flavor: replor_render::flavor::Flavor) -> Vec<RenderedLine> {
        let document = replor_render::document::build(
            markdown.into(),
            flavor,
            replor_render::diagram::MermaidMode::Off,
            replor_render::diagram::MermaidWidth::Unicode,
        );
        let prepared = replor_render::layout::PreparedDocument::new(document, Default::default());
        replor_render::layout::layout_document(
            &prepared,
            replor_render::layout::LayoutOptions::new(40),
        )
        .lines
    }

    fn texts(lines: &[RenderedLine]) -> Vec<String> {
        lines.iter().map(RenderedLine::text).collect()
    }

    fn index_of(lines: &[RenderedLine], needle: &str) -> usize {
        texts(lines)
            .iter()
            .position(|text| text.contains(needle))
            .unwrap_or_else(|| panic!("no line containing {needle:?} in {:?}", texts(lines)))
    }

    const IN_A_LIST: &str = "1. Step one\n\n   ```sh\n   echo 0\n   echo 1\n   echo 2\n   ```\n";

    /// A container's separator and title lines carry the container's whole
    /// range, which also covers the code inside it.
    #[test]
    fn a_source_line_inside_a_container_opens_on_its_own_line() {
        use replor_render::flavor::Flavor;
        let lines = layout(IN_A_LIST, Flavor::Gfm);
        assert_eq!(
            rendered_line_for_source(&lines, 5),
            Some(index_of(&lines, "echo 1"))
        );
        let details = "<details><summary>S</summary>\n\n```\nc0\nc1\n```\n\n</details>\n";
        let lines = layout(details, Flavor::AzureDevops);
        assert_eq!(
            rendered_line_for_source(&lines, 5),
            Some(index_of(&lines, "c1"))
        );
        // The delimiter row is a line of the table too.
        let lines = layout("| h |\n|---|\n| a |\n", Flavor::Gfm);
        assert_eq!(
            rendered_line_for_source(&lines, 2),
            Some(index_of(&lines, "├"))
        );
    }

    #[test]
    fn a_reload_inside_a_container_keeps_the_code_line() {
        use replor_render::flavor::Flavor;
        let old = layout(IN_A_LIST, Flavor::Gfm);
        let edited = IN_A_LIST.replace("Step one", "Step number one");
        let new = layout(&edited, Flavor::Gfm);
        assert_eq!(
            restored(&old, index_of(&old, "echo 2"), &new),
            index_of(&new, "echo 2")
        );
    }

    /// The same range can appear more than once, such as a heading and its
    /// `[[_TOC_]]` entry, or the separators of one list item.
    #[test]
    fn a_repeated_range_restores_the_occurrence_that_was_on_top() {
        use replor_render::flavor::Flavor;
        let toc = "[[_TOC_]]\n\n# One\n\npara\n\n# Two\n\nlong paragraph here\n";
        let lines = layout(toc, Flavor::AzureDevops);
        let heading = index_of(&lines, "# Two");
        assert_eq!(restored(&lines, heading, &lines), heading);

        let loose = "- intro\n\n  ```\n  code\n  ```\n\n  outro\n";
        let lines = layout(loose, Flavor::Gfm);
        let separator = index_of(&lines, "outro") - 1;
        assert_eq!(restored(&lines, separator, &lines), separator);
    }

    fn state() -> PagerState {
        PagerState::new(100, 200, 10, 40)
    }

    /// A container's title on lines 1 to 8, then a paragraph of lines 3 to 5
    /// wrapped onto six lines.
    #[test]
    fn a_position_within_a_block_is_kept_in_proportion() {
        let mut lines = vec![mapped(1, 8)];
        lines.extend((0..6).map(|_| mapped(3, 5)));
        assert_eq!(source_position_at(&lines, 0), Some(1.5));
        assert_eq!(source_position_at(&lines, 4), Some(3.0 + 3.5 * 3.0 / 6.0));
        assert_eq!(rendered_line_for_position(&lines, 4.5), Some((4, true)));
        // Line 2 is the container's own.
        assert_eq!(rendered_line_for_position(&lines, 2.5), Some((0, true)));
    }

    /// With more rendered lines than source lines, by however much, every
    /// source line comes back to itself.
    #[test]
    fn a_line_of_a_block_comes_back_to_itself() {
        for len in [3, 4, 5, 7] {
            let lines: Vec<_> = (0..len).map(|_| mapped(3, 5)).collect();
            for line in 3..=5 {
                let (index, covered) =
                    rendered_line_for_position(&lines, line as f64 + 0.5).unwrap();
                assert!(covered);
                let back = source_position_at(&lines, index).unwrap();
                assert_eq!(back as usize, line, "{len}: {line} -> {index} -> {back}");
            }
        }
    }

    /// A `<details>` summary wrapped onto two lines carries the whole range,
    /// and stays at its start rather than being laid over the content.
    #[test]
    fn a_container_wrapped_onto_several_lines_stays_at_its_start() {
        let mut lines = vec![mapped(1, 20), mapped(1, 20)];
        lines.extend((0..4).map(|_| mapped(3, 18)));
        assert_eq!(source_position_at(&lines, 0), Some(1.5));
        assert_eq!(source_position_at(&lines, 1), Some(1.5));
        assert_eq!(rendered_line_for_position(&lines, 1.5), Some((0, true)));
    }

    fn mapped(line_start: usize, line_end: usize) -> RenderedLine {
        RenderedLine {
            spans: vec![RenderedSpan::new("x", StyleRole::Normal)],
            source_range: Some(SourceRange {
                byte_start: 0,
                byte_end: 1,
                line_start,
                line_end,
            }),
            no_wrap: false,
            image: None,
        }
    }

    #[test]
    fn vertical_scrolling_clamps_at_both_ends() {
        let mut s = state();
        s.apply(Action::LineUp);
        assert_eq!(s.viewport.top, 0);
        s.apply(Action::Bottom);
        assert_eq!(s.viewport.top, 90);
        s.apply(Action::LineDown);
        assert_eq!(s.viewport.top, 90);
        s.apply(Action::Top);
        assert_eq!(s.viewport.top, 0);
    }

    #[test]
    fn page_and_half_page_steps_differ() {
        let mut s = state();
        s.apply(Action::HalfPageDown);
        assert_eq!(s.viewport.top, 5);
        s.apply(Action::PageDown);
        assert_eq!(s.viewport.top, 15);
        s.apply(Action::HalfPageUp);
        assert_eq!(s.viewport.top, 10);
        s.apply(Action::PageUp);
        assert_eq!(s.viewport.top, 0);
    }

    #[test]
    fn horizontal_scrolling_clamps_and_resets() {
        let mut s = state();
        s.apply(Action::ScrollLeft);
        assert_eq!(s.viewport.left, 0);
        s.apply(Action::ScrollRight);
        assert_eq!(s.viewport.left, 8);
        for _ in 0..100 {
            s.apply(Action::ScrollRight);
        }
        assert_eq!(s.viewport.left, s.max_left());
        s.apply(Action::ResetHorizontal);
        assert_eq!(s.viewport.left, 0);
    }

    #[test]
    fn a_document_shorter_than_the_screen_does_not_scroll() {
        let mut s = PagerState::new(3, 10, 20, 40);
        s.apply(Action::Bottom);
        assert_eq!(s.viewport.top, 0);
        assert_eq!(s.percentage(), 100);
    }

    #[test]
    fn images_are_hidden_only_while_an_overlay_is_open() {
        let mut s = state();
        s.outline = Outline {
            items: vec![OutlineItem {
                level: 1,
                text: "a".into(),
                line: 0,
            }],
            selected: 0,
        };
        assert!(s.shows_images());
        s.apply(Action::ToggleOutline);
        assert!(!s.shows_images());
        s.cancel_outline();
        s.apply(Action::ToggleHelp);
        assert!(!s.shows_images());
        s.close_help();
        s.apply(Action::StartSearch);
        assert!(s.shows_images());
    }

    #[test]
    fn quit_stops_the_loop() {
        assert!(!state().apply(Action::Quit));
        assert!(state().apply(Action::LineDown));
    }

    #[test]
    fn resize_clamps_offsets_into_the_new_bounds() {
        let mut s = state();
        s.apply(Action::Bottom);
        s.apply(Action::ScrollRight);
        s.resize(20, 10, 10, 40);
        assert_eq!(s.viewport.top, 10);
        assert_eq!(s.viewport.left, 0);
    }

    #[test]
    fn searching_moves_to_the_match_and_reports_misses() {
        let lines: SearchIndex = (0..100)
            .map(|i| if i == 42 { "needle".into() } else { "x".into() })
            .collect();
        let mut s = state();
        s.apply(Action::StartSearch);
        s.search_session_mut().unwrap().input = "needle".into();
        s.confirm_search(&lines);
        assert_eq!(s.mode(), Mode::Normal);
        assert!(s.viewport.top <= 42 && 42 < s.viewport.top + s.viewport.height);

        let before = s.viewport.top;
        s.apply(Action::StartSearch);
        s.search_session_mut().unwrap().input = "absent".into();
        s.confirm_search(&lines);
        assert_eq!(s.viewport.top, before);
        assert!(s.status.as_deref().unwrap().contains("no match"));
    }

    fn haystack() -> SearchIndex {
        (0..100)
            .map(|i| match i {
                20 | 42 | 80 => "needle".to_string(),
                _ => "x".to_string(),
            })
            .collect()
    }

    /// Typing moves the viewport as the query grows, without waiting for Enter.
    #[test]
    fn typing_previews_the_first_match_from_where_the_search_started() {
        let lines = haystack();
        let mut s = state();
        s.viewport.top = 30;
        s.apply(Action::StartSearch);
        for c in "needle".chars() {
            s.insert_search_char(c, &lines);
        }
        assert_eq!(s.mode(), Mode::Search);
        assert!(
            s.viewport.top <= 42 && 42 < s.viewport.top + s.viewport.height,
            "top={}",
            s.viewport.top
        );
        assert_eq!(s.preview_count(), 3);

        // Deleting back to nothing puts the reader where they started.
        for _ in 0.."needle".len() {
            s.edit_search(SearchEdit::Backspace, &lines);
        }
        assert_eq!(s.viewport.top, 30);
        assert_eq!(s.preview_count(), 0);
    }

    #[test]
    fn a_query_with_no_match_leaves_the_viewport_alone() {
        let lines = haystack();
        let mut s = state();
        s.viewport.top = 30;
        s.apply(Action::StartSearch);
        for c in "absent".chars() {
            s.insert_search_char(c, &lines);
        }
        assert_eq!(s.viewport.top, 30);
        assert_eq!(s.preview_count(), 0);
    }

    #[test]
    fn cancelling_a_search_returns_to_where_it_started() {
        let lines = haystack();
        let mut s = state();
        s.viewport.top = 30;
        s.viewport.left = 8;
        s.apply(Action::StartSearch);
        for c in "needle".chars() {
            s.insert_search_char(c, &lines);
        }
        s.cancel_search();
        assert_eq!(s.mode(), Mode::Normal);
        assert_eq!((s.viewport.top, s.viewport.left), (30, 8));
        assert!(!s.search.is_active());
    }

    #[test]
    fn confirming_keeps_the_previewed_match() {
        let lines = haystack();
        let mut s = state();
        s.viewport.top = 30;
        s.apply(Action::StartSearch);
        for c in "needle".chars() {
            s.insert_search_char(c, &lines);
        }
        let previewed = s.viewport.top;
        s.confirm_search(&lines);
        assert_eq!(s.mode(), Mode::Normal);
        assert_eq!(s.viewport.top, previewed);
        assert_eq!(s.search.current().unwrap().line, 42);
        assert_eq!(s.preview_count(), 0);
    }

    fn typed(text: &str, lines: &SearchIndex) -> PagerState {
        let mut s = state();
        s.apply(Action::StartSearch);
        for c in text.chars() {
            s.insert_search_char(c, lines);
        }
        s
    }

    #[test]
    fn the_caret_edits_the_prompt_the_way_a_shell_does() {
        let lines = haystack();
        let mut s = typed("needle here", &lines);

        s.edit_search(SearchEdit::DeleteWordBefore, &lines);
        assert_eq!(s.search_session().unwrap().input, "needle ");
        s.edit_search(SearchEdit::CaretStart, &lines);
        assert_eq!(s.search_session().unwrap().caret, 0);
        s.insert_search_char('a', &lines);
        assert_eq!(
            (
                s.search_session().unwrap().input.as_str(),
                s.search_session().unwrap().caret
            ),
            ("aneedle ", 1)
        );
        s.edit_search(SearchEdit::Delete, &lines);
        assert_eq!(s.search_session().unwrap().input, "aeedle ");
        s.edit_search(SearchEdit::CaretEnd, &lines);
        s.edit_search(SearchEdit::KillToStart, &lines);
        assert_eq!(
            (
                s.search_session().unwrap().input.as_str(),
                s.search_session().unwrap().caret
            ),
            ("", 0)
        );
    }

    #[test]
    fn caret_movement_respects_grapheme_boundaries() {
        let lines = haystack();
        let mut s = typed("日本語", &lines);
        assert_eq!(s.search_session().unwrap().caret, 9);
        s.edit_search(SearchEdit::CaretLeft, &lines);
        assert_eq!(s.search_session().unwrap().caret, 6);
        s.edit_search(SearchEdit::Backspace, &lines);
        assert_eq!(
            (
                s.search_session().unwrap().input.as_str(),
                s.search_session().unwrap().caret
            ),
            ("日語", 3)
        );
        s.edit_search(SearchEdit::CaretRight, &lines);
        s.edit_search(SearchEdit::CaretRight, &lines);
        assert_eq!(
            s.search_session().unwrap().caret,
            s.search_session().unwrap().input.len()
        );
        // Moving past either end stays on a boundary.
        s.edit_search(SearchEdit::CaretRight, &lines);
        assert_eq!(
            s.search_session().unwrap().caret,
            s.search_session().unwrap().input.len()
        );
        s.edit_search(SearchEdit::CaretStart, &lines);
        s.edit_search(SearchEdit::CaretLeft, &lines);
        assert_eq!(s.search_session().unwrap().caret, 0);
    }

    #[test]
    fn killing_to_the_end_keeps_what_is_before_the_caret() {
        let lines = haystack();
        let mut s = typed("needlex", &lines);
        s.edit_search(SearchEdit::CaretLeft, &lines);
        s.edit_search(SearchEdit::KillToEnd, &lines);
        assert_eq!(s.search_session().unwrap().input, "needle");
        // The preview follows every edit, not just insertions.
        assert_eq!(s.preview_count(), 3);
    }

    /// A resize or a `--watch` reload can shrink the surface while the prompt
    /// is open. The remembered position must not put the viewport past the end.
    #[test]
    fn a_shorter_document_does_not_strand_the_restored_viewport() {
        let lines = haystack();
        let mut s = typed("needle", &lines);
        assert_eq!(s.search_session().unwrap().origin, (0, 0));
        s.viewport.top = 90;
        s.search_session_mut().unwrap().origin = (90, 40);

        let short: SearchIndex = lines[..30].iter().cloned().collect();
        s.resize(short.len(), 10, 10, 40);
        s.insert_search_char('x', &short);
        assert!(
            s.viewport.top <= s.max_top(),
            "top={} max={}",
            s.viewport.top,
            s.max_top()
        );

        s.cancel_search();
        assert!(
            s.viewport.top <= s.max_top(),
            "top={} max={}",
            s.viewport.top,
            s.max_top()
        );
        assert!(s.viewport.left <= s.max_left());
    }

    #[test]
    fn a_relayout_keeps_the_current_match_near_the_reader() {
        let lines = haystack();
        let mut s = typed("needle", &lines);
        s.confirm_search(&lines);
        s.apply(Action::NextMatch);
        s.apply(Action::NextMatch);
        assert_eq!(s.search.current().unwrap().line, 80);

        // Same surface, laid out again: the highlight stays where the reader
        // is instead of going back to the first match.
        s.recompute_searches(&lines);
        assert_eq!(s.search.current().unwrap().line, 80);
    }

    #[test]
    fn a_relayout_also_re_runs_the_query_being_typed() {
        let lines = haystack();
        let mut s = typed("needle", &lines);
        let widened: SearchIndex = lines.iter().map(|l| format!("{l} {l}")).collect();
        s.recompute_searches(&widened);
        // Two occurrences per line now, and the prompt's matches follow.
        assert_eq!(s.preview_count(), 6);
    }

    #[test]
    fn cycling_past_the_last_match_says_it_wrapped() {
        let lines = haystack();
        let mut s = typed("needle", &lines);
        s.confirm_search(&lines);
        s.apply(Action::NextMatch);
        assert_eq!(s.status, None);
        s.apply(Action::NextMatch);
        assert_eq!(s.status, None);
        // Fourth match of three: back to the first.
        s.apply(Action::NextMatch);
        assert_eq!(s.status.as_deref(), Some("wrapped"));
        s.apply(Action::PreviousMatch);
        assert_eq!(s.status.as_deref(), Some("wrapped"));
    }

    #[test]
    fn cancelling_a_search_returns_to_normal_mode() {
        let mut s = state();
        s.apply(Action::StartSearch);
        assert_eq!(s.mode(), Mode::Search);
        s.search_session_mut().unwrap().input.push('a');
        s.cancel_search();
        assert_eq!(s.mode(), Mode::Normal);
        assert!(s.search_session().is_none());
    }

    fn with_outline() -> PagerState {
        let mut s = state();
        s.set_outline(vec![
            OutlineItem {
                level: 1,
                text: "Title".into(),
                line: 0,
            },
            OutlineItem {
                level: 2,
                text: "First".into(),
                line: 20,
            },
            OutlineItem {
                level: 2,
                text: "Second".into(),
                line: 60,
            },
        ]);
        s
    }

    #[test]
    fn the_outline_opens_on_the_section_being_read() {
        let mut s = with_outline();
        s.viewport.top = 25;
        s.apply(Action::ToggleOutline);
        assert_eq!(s.mode(), Mode::Outline);
        assert_eq!(s.outline.selected, 1);
    }

    #[test]
    fn toggling_twice_returns_to_the_document() {
        let mut s = with_outline();
        s.apply(Action::ToggleOutline);
        s.apply(Action::ToggleOutline);
        assert_eq!(s.mode(), Mode::Normal);
    }

    #[test]
    fn a_document_without_headings_reports_instead_of_opening() {
        let mut s = state();
        s.apply(Action::ToggleOutline);
        assert_eq!(s.mode(), Mode::Normal);
        assert_eq!(s.status.as_deref(), Some("no headings"));
    }

    #[test]
    fn outline_selection_clamps_at_both_ends() {
        let mut s = with_outline();
        s.apply(Action::ToggleOutline);
        s.move_outline(-5);
        assert_eq!(s.outline.selected, 0);
        s.move_outline(9);
        assert_eq!(s.outline.selected, 2);
    }

    #[test]
    fn confirming_moves_the_viewport_to_the_heading() {
        let mut s = with_outline();
        s.apply(Action::ToggleOutline);
        s.move_outline(1);
        s.confirm_outline();
        assert_eq!(s.mode(), Mode::Normal);
        assert_eq!(s.viewport.top, 20);
    }

    #[test]
    fn cancelling_leaves_the_viewport_alone() {
        let mut s = with_outline();
        s.viewport.top = 40;
        s.apply(Action::ToggleOutline);
        s.move_outline(-2);
        s.cancel_outline();
        assert_eq!(s.mode(), Mode::Normal);
        assert_eq!(s.viewport.top, 40);
    }

    #[test]
    fn a_relayout_keeps_the_selection_within_bounds() {
        let mut s = with_outline();
        s.apply(Action::ToggleOutline);
        s.move_outline(2);
        s.set_outline(vec![OutlineItem {
            level: 1,
            text: "Title".into(),
            line: 3,
        }]);
        assert_eq!(s.outline.selected, 0);
        // Losing every heading must not leave the overlay open.
        s.set_outline(Vec::new());
        assert_eq!(s.mode(), Mode::Normal);
    }

    #[test]
    fn the_help_overlay_toggles_and_scrolls_within_its_list() {
        let mut s = state();
        s.apply(Action::ToggleHelp);
        assert_eq!(s.mode(), Mode::Help);
        s.scroll_help(-1, 15, 8);
        assert_eq!(s.help_top, 0);
        s.scroll_help(20, 15, 8);
        assert_eq!(s.help_top, 7);
        s.apply(Action::ToggleHelp);
        assert_eq!(s.mode(), Mode::Normal);
        // Reopening starts from the top of the list.
        s.apply(Action::ToggleHelp);
        assert_eq!(s.help_top, 0);
    }

    #[test]
    fn opening_the_help_does_not_move_the_document() {
        let mut s = state();
        s.viewport.top = 30;
        s.initial_line = Some(30);
        s.apply(Action::ToggleHelp);
        assert_eq!(s.viewport.top, 30);
        assert_eq!(s.initial_line, Some(30));
    }

    #[test]
    fn movement_retires_the_initial_line_highlight() {
        let mut s = state();
        s.initial_line = Some(5);
        s.apply(Action::LineDown);
        assert_eq!(s.initial_line, None);
    }

    #[test]
    fn source_line_maps_to_the_covering_rendered_line() {
        let lines = vec![mapped(1, 1), mapped(3, 5), mapped(9, 9)];
        assert_eq!(rendered_line_for_source(&lines, 1), Some(0));
        assert_eq!(rendered_line_for_source(&lines, 4), Some(1));
        assert_eq!(rendered_line_for_source(&lines, 9), Some(2));
    }

    #[test]
    fn an_unmapped_source_line_falls_forward_then_backward() {
        let lines = vec![mapped(1, 1), mapped(5, 5)];
        // Line 3 has no rendered line; the next mapped one wins.
        assert_eq!(rendered_line_for_source(&lines, 3), Some(1));
        // Past the end, the last mapped line wins.
        assert_eq!(rendered_line_for_source(&lines, 99), Some(1));
        assert_eq!(rendered_line_for_source(&[], 1), None);
    }

    #[test]
    fn the_anchor_skips_unmapped_lines() {
        let lines = vec![mapped(1, 1), RenderedLine::blank(), mapped(7, 7)];
        assert_eq!(source_line_at(&lines, 1), Some(7));
        assert_eq!(source_line_at(&lines, 0), Some(1));
    }
}
