//! Frame rendering. Everything drawn here comes from the already laid out
//! surface; no Markdown parsing or diagram rendering happens per frame.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color as RatColor, Modifier, Style as RatStyle};
use ratatui::text::{Line as RatLine, Span as RatSpan};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

use crate::pager::help;
use crate::pager::state::{Mode, PagerState};
use replor_render::layout::theme::{Color, Style, Theme};
use replor_render::layout::wrap::display_width;
use replor_render::layout::{RenderedLine, StyleRole};
use replor_render::navigation::LinkKey;

/// A search match on screen: the one being visited, or one of the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Highlight {
    Match,
    Current,
}

impl Highlight {
    fn role(self) -> StyleRole {
        match self {
            Highlight::Match => StyleRole::SearchMatch,
            Highlight::Current => StyleRole::CurrentMatch,
        }
    }
}

/// A run of text on screen with the role it should be drawn in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleSpan {
    pub text: String,
    pub role: StyleRole,
    pub highlighted: Option<Highlight>,
    /// The destination is a real URL. Underlined, as in the stdout backend, so
    /// it stands out from a relative path or a `#123` styled the same way.
    pub linked: bool,
    /// The link this run is the label of.
    pub key: Option<LinkKey>,
}

/// The line above the status bar while a link is selected or refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkBar {
    pub text: String,
    /// Where the link stands among the document's links, such as `2/7`.
    pub position: String,
    /// The link was not followed, and `text` says why.
    pub refused: bool,
}

pub struct ViewContext<'a> {
    pub title: &'a str,
    pub flavor: &'a str,
    /// The line of the file at the top, the `line` of the status bar's
    /// `line/lines`, or 0 when no line of it is there. A line of the file,
    /// not of the document, which may have lines of its own around it.
    pub top_line: usize,
    /// The file's lines, the `lines` of the status bar's `line/lines`.
    pub lines: usize,
    pub diagnostics: usize,
    pub theme: Theme,
    /// Drawn reversed, so the reader sees which link `Enter` follows.
    pub selected_link: Option<LinkKey>,
    pub link_bar: Option<&'a LinkBar>,
}

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    lines: &[RenderedLine],
    state: &PagerState,
    ctx: &ViewContext<'_>,
) {
    let [body, status] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

    let rows: Vec<RatLine> = (state.viewport.top
        ..(state.viewport.top + state.viewport.height).min(lines.len()))
        .map(|index| {
            let search = state.visible_search();
            let current = search.current();
            let highlights: Vec<(usize, usize, Highlight)> = search
                .matches_on(index)
                .map(|m| {
                    let which = if current == Some(*m) {
                        Highlight::Current
                    } else {
                        Highlight::Match
                    };
                    (m.start, m.end, which)
                })
                .collect();
            let initial = state.initial_line == Some(index);
            let spans = visible(
                &lines[index],
                state.viewport.left,
                body.width as usize,
                &highlights,
            );
            RatLine::from(
                spans
                    .into_iter()
                    .map(|span| {
                        let role = if initial && span.role == StyleRole::Normal {
                            StyleRole::InitialLine
                        } else {
                            span.role
                        };
                        let mut style = match span.highlighted {
                            Some(highlight) => ctx.theme.style(highlight.role()),
                            None => ctx.theme.style(role),
                        };
                        if span.linked {
                            style.underline = true;
                        }
                        let mut style = convert(style);
                        if span.key.is_some() && span.key == ctx.selected_link {
                            style = style.add_modifier(Modifier::REVERSED);
                        }
                        RatSpan::styled(span.text, style)
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    frame.render_widget(Paragraph::new(rows), body);
    if let Some(bar) = ctx.link_bar
        && body.height > 1
    {
        let area = Rect {
            y: body.y + body.height - 1,
            height: 1,
            ..body
        };
        frame.render_widget(Clear, area);
        frame.render_widget(link_bar(bar, ctx, area.width as usize), area);
    }
    frame.render_widget(status_bar(state, ctx, status.width as usize), status);
    match state.mode() {
        Mode::Outline => draw_outline(frame, state, ctx, body),
        Mode::Help => draw_help(frame, state, ctx, body),
        _ => {}
    }
}

/// Floating key list. The pager cannot show the README, so the overlay carries
/// the bindings itself; `help::ENTRIES` is where they are defined.
fn draw_help(frame: &mut Frame, state: &PagerState, ctx: &ViewContext<'_>, body: Rect) {
    let area = overlay_area(body, help::ENTRIES.len());
    let inner_width = area.width.saturating_sub(2) as usize;
    let inner_height = help_rows(body.height as usize);
    let column = help::key_column();

    // The terminal can grow between two key presses, so the offset is clamped
    // here rather than only where it is moved.
    let top = state
        .help_top
        .min(help::ENTRIES.len().saturating_sub(inner_height));
    let rows: Vec<RatLine> = help::ENTRIES
        .iter()
        .skip(top)
        .take(inner_height)
        .map(|entry| {
            let text = elide_end(&help::row(entry, column), inner_width);
            RatLine::from(RatSpan::styled(
                text,
                convert(ctx.theme.style(StyleRole::Normal)),
            ))
        })
        .collect();

    let block = Block::bordered()
        .title("Keys")
        .border_style(convert(ctx.theme.style(StyleRole::CodeBorder)));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(rows).block(block), area);
}

/// Floating heading list. Drawn over the document so the reader keeps their
/// place; `Clear` prevents the text underneath from showing through.
fn draw_outline(frame: &mut Frame, state: &PagerState, ctx: &ViewContext<'_>, body: Rect) {
    let items = &state.outline.items;
    let area = overlay_area(body, items.len());
    let inner_width = area.width.saturating_sub(2) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;

    let rows: Vec<RatLine> = outline_window(state.outline.selected, items.len(), inner_height)
        .map(|index| {
            let item = &items[index];
            let indent = "  ".repeat(item.level.saturating_sub(1) as usize);
            let label = elide_end(&format!("{indent}{}", item.text), inner_width);
            let role = if index == state.outline.selected {
                StyleRole::SearchMatch
            } else {
                StyleRole::Normal
            };
            RatLine::from(RatSpan::styled(label, convert(ctx.theme.style(role))))
        })
        .collect();

    let block = Block::bordered()
        .title("Headings")
        .border_style(convert(ctx.theme.style(StyleRole::CodeBorder)));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(rows).block(block), area);
}

/// Which items are on screen: the selection stays visible without moving the
/// window more than it has to.
fn outline_window(selected: usize, total: usize, height: usize) -> std::ops::Range<usize> {
    if height == 0 || total == 0 {
        return 0..0;
    }
    let top = selected.saturating_sub(height.saturating_sub(1));
    let top = top.min(total.saturating_sub(height));
    top..(top + height).min(total)
}

/// How many help entries the overlay shows at once. The key map needs the same
/// number to bound scrolling, so both sides call this.
pub fn help_rows(body_height: usize) -> usize {
    let body_height = u16::try_from(body_height).unwrap_or(u16::MAX);
    let height = (help::ENTRIES.len() as u16 + 2).min(body_height.saturating_sub(2).max(3));
    height.saturating_sub(2) as usize
}

fn overlay_area(body: Rect, items: usize) -> Rect {
    let width = body.width.saturating_sub(4).clamp(1, 60).max(1);
    let height = (items as u16 + 2).min(body.height.saturating_sub(2).max(3));
    let x = body.x + (body.width.saturating_sub(width)) / 2;
    let y = body.y + (body.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Cut a label to `budget` display columns, marking the cut with an ellipsis.
pub fn elide_end(text: &str, budget: usize) -> String {
    if display_width(text) <= budget {
        return text.to_string();
    }
    if budget <= 1 {
        return String::new();
    }
    let mut kept = String::new();
    let mut used = 1usize; // the trailing ellipsis
    for grapheme in text.graphemes(true) {
        let w = display_width(grapheme);
        if used + w > budget {
            break;
        }
        used += w;
        kept.push_str(grapheme);
    }
    format!("{kept}…")
}

/// Slice one rendered line to the horizontal window, splitting spans where a
/// search match starts or ends. Grapheme boundaries and display widths are
/// respected so wide characters are never cut in half.
pub fn visible(
    line: &RenderedLine,
    left: usize,
    width: usize,
    highlights: &[(usize, usize, Highlight)],
) -> Vec<VisibleSpan> {
    let mut out: Vec<VisibleSpan> = Vec::new();
    let mut column = 0usize;
    let mut offset = 0usize;

    for span in &line.spans {
        for grapheme in span.text.graphemes(true) {
            let cell_width = display_width(grapheme);
            let start = offset;
            offset += grapheme.len();

            // Skip everything scrolled off to the left.
            if column + cell_width <= left {
                column += cell_width;
                continue;
            }
            if column >= left + width {
                return out;
            }
            // A wide character straddling the left edge is dropped rather than
            // drawn half off screen.
            if column < left {
                column += cell_width;
                continue;
            }
            column += cell_width;

            let highlighted = highlights
                .iter()
                .find(|(s, e, _)| start >= *s && start < *e)
                .map(|(_, _, which)| *which);
            let linked = span.link.is_some();
            match out.last_mut() {
                Some(last)
                    if last.role == span.role
                        && last.highlighted == highlighted
                        && last.linked == linked
                        && last.key == span.key =>
                {
                    last.text.push_str(grapheme);
                }
                _ => out.push(VisibleSpan {
                    text: grapheme.to_string(),
                    role: span.role,
                    highlighted,
                    linked,
                    key: span.key,
                }),
            }
        }
    }
    out
}

/// The selected link's destination, or why it was not followed, with its
/// place among the links on the right.
fn link_bar<'a>(bar: &LinkBar, ctx: &ViewContext<'_>, width: usize) -> Paragraph<'a> {
    let (marker, role) = match bar.refused {
        true => ("! ", StyleRole::Warning),
        false => ("→ ", StyleRole::LinkTarget),
    };
    let right = match bar.position.is_empty() {
        true => String::new(),
        false => format!("  [{}]", bar.position),
    };
    let budget = width.saturating_sub(display_width(marker) + display_width(&right));
    let text = elide_end(&bar.text, budget);
    let gap =
        width.saturating_sub(display_width(marker) + display_width(&text) + display_width(&right));
    let style = convert(ctx.theme.style(role));
    let muted = convert(ctx.theme.style(StyleRole::Muted));
    Paragraph::new(RatLine::from(vec![
        RatSpan::styled(marker, style),
        RatSpan::styled(text, style),
        RatSpan::styled(" ".repeat(gap), muted),
        RatSpan::styled(right, muted),
    ]))
}

fn status_bar<'a>(state: &PagerState, ctx: &ViewContext<'_>, width: usize) -> Paragraph<'a> {
    let style = convert(ctx.theme.style(StyleRole::Status));
    if let Some(session) = state.search_session() {
        let tally = tally(state.visible_search());
        // The tally sits where it will sit once the search is confirmed, so
        // confirming does not make it jump across the bar. A prompt too long to
        // share the line keeps the room: the caret must stay on screen.
        let room = display_width(&tally) + 2;
        let show_tally = !session.input.is_empty() && width > room + MIN_PROMPT;
        let budget = if show_tally { width - room } else { width };
        let mut spans = prompt_spans(session, style, budget);
        if show_tally {
            let used: usize = spans.iter().map(|s| display_width(&s.content)).sum();
            let gap = width.saturating_sub(used + display_width(&tally)).max(1);
            spans.push(RatSpan::styled(
                format!("{}{tally}", " ".repeat(gap)),
                style,
            ));
        }
        return Paragraph::new(RatLine::from(spans));
    }

    let mut right = format!(
        "{}  {}/{}  {}%",
        ctx.flavor,
        ctx.top_line,
        ctx.lines,
        state.percentage()
    );
    if ctx.diagnostics > 0 {
        right.push_str(&format!("  {} diagnostics", ctx.diagnostics));
    }
    if let Some(status) = &state.status {
        right.push_str(&format!("  {status}"));
    }
    if state.search.is_active() {
        right.push_str(&format!(
            "  /{} {}",
            state.search.query(),
            tally(&state.search)
        ));
    }

    // The status bar stays on one line: the path gives up its head first.
    let budget = width.saturating_sub(display_width(&right) + 2);
    let title = elide_start(ctx.title, budget);
    let gap = width.saturating_sub(display_width(&title) + display_width(&right));
    Paragraph::new(RatLine::from(RatSpan::styled(
        format!("{title}{}{right}", " ".repeat(gap.max(1))),
        style,
    )))
}

/// Where the reader is within the matches: `(3/12)`, or that there are none.
fn tally(search: &crate::pager::search::Search) -> String {
    match search.position() {
        Some(index) => format!("({}/{})", index + 1, search.count()),
        None => "(no match)".to_string(),
    }
}

/// Columns the prompt keeps for itself before the tally is dropped.
const MIN_PROMPT: usize = 8;

/// The prompt with its caret, windowed to `budget` columns. `TerminalGuard`
/// owns cursor visibility and keeps it hidden, so the caret is a cell drawn
/// against the status bar rather than the terminal cursor.
fn prompt_spans<'a>(
    session: &crate::pager::search_session::SearchSession,
    style: RatStyle,
    budget: usize,
) -> Vec<RatSpan<'a>> {
    let caret = session.caret.min(session.input.len());
    let (under, after) = match session.input[caret..].graphemes(true).next() {
        Some(grapheme) => (
            grapheme.to_string(),
            &session.input[caret + grapheme.len()..],
        ),
        None => (" ".to_string(), ""),
    };
    let before = format!("/{}", &session.input[..caret]);

    // A query wider than the bar scrolls: the caret stays at the right edge
    // instead of being typed into an invisible tail.
    let head = display_width(&before) + display_width(&under);
    let before = match head.checked_sub(budget) {
        Some(over) if over > 0 => drop_columns(&before, over),
        _ => before,
    };
    let left = budget.saturating_sub(display_width(&before) + display_width(&under));
    let after = take_columns(after, left);

    vec![
        RatSpan::styled(before, style),
        // The status bar is already reversed, so reversing again would draw
        // nothing; the caret is the one cell that is not.
        RatSpan::styled(under, style.remove_modifier(Modifier::REVERSED)),
        RatSpan::styled(after.to_string(), style),
    ]
}

/// Drop `columns` display columns from the front, on grapheme boundaries.
fn drop_columns(text: &str, columns: usize) -> String {
    let mut dropped = 0usize;
    let mut kept = String::new();
    for grapheme in text.graphemes(true) {
        if dropped >= columns {
            kept.push_str(grapheme);
        } else {
            dropped += display_width(grapheme);
        }
    }
    kept
}

/// Keep at most `columns` display columns from the front.
fn take_columns(text: &str, columns: usize) -> String {
    let mut used = 0usize;
    let mut kept = String::new();
    for grapheme in text.graphemes(true) {
        let width = display_width(grapheme);
        if used + width > columns {
            break;
        }
        used += width;
        kept.push_str(grapheme);
    }
    kept
}

/// Drop leading path components so the file name stays visible.
pub fn elide_start(text: &str, budget: usize) -> String {
    if display_width(text) <= budget {
        return text.to_string();
    }
    if budget <= 1 {
        return String::new();
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 1usize; // the leading ellipsis
    for grapheme in text.graphemes(true).rev() {
        let w = display_width(grapheme);
        if used + w > budget {
            break;
        }
        used += w;
        kept.push(grapheme);
    }
    kept.reverse();
    format!("…{}", kept.concat())
}

pub(crate) fn convert(style: Style) -> RatStyle {
    let mut out = RatStyle::default();
    if let Some(fg) = style.fg {
        out = out.fg(color(fg));
    }
    if let Some(bg) = style.bg {
        out = out.bg(color(bg));
    }
    let mut modifiers = Modifier::empty();
    if style.bold {
        modifiers |= Modifier::BOLD;
    }
    if style.dim {
        modifiers |= Modifier::DIM;
    }
    if style.italic {
        modifiers |= Modifier::ITALIC;
    }
    if style.underline {
        modifiers |= Modifier::UNDERLINED;
    }
    if style.reverse {
        modifiers |= Modifier::REVERSED;
    }
    if style.strikethrough {
        modifiers |= Modifier::CROSSED_OUT;
    }
    out.add_modifier(modifiers)
}

fn color(value: Color) -> RatColor {
    match value {
        Color::Red => RatColor::Red,
        Color::Green => RatColor::Green,
        Color::Yellow => RatColor::Yellow,
        Color::Blue => RatColor::Blue,
        Color::Magenta => RatColor::Magenta,
        Color::BrightBlack => RatColor::DarkGray,
        Color::BrightGreen => RatColor::LightGreen,
        Color::BrightYellow => RatColor::LightYellow,
        Color::BrightBlue => RatColor::LightBlue,
        Color::BrightCyan => RatColor::LightCyan,
    }
}

/// Widest rendered line, used to bound horizontal scrolling.
pub fn widest_line(lines: &[RenderedLine]) -> usize {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| display_width(&s.text))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_pager_has_no_colors_and_keeps_the_current_match_visible() {
        let lines = vec![line("needle")];
        let texts = lines.iter().map(RenderedLine::text).collect();
        let mut state = PagerState::new(1, 6, 4, 30);
        state.apply(crate::pager::state::Action::StartSearch);
        for c in "needle".chars() {
            state.insert_search_char(c, &texts);
        }
        state.confirm_search(&texts);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 5)).unwrap();
        let ctx = ViewContext {
            selected_link: None,
            link_bar: None,
            title: "doc.md",
            flavor: "gfm",
            top_line: 1,
            lines: 1,
            diagnostics: 0,
            theme: Theme::new(replor_render::layout::theme::Variant::Dark)
                .with_color(replor_render::output::ColorChoice::Plain),
        };
        terminal
            .draw(|frame| draw(frame, frame.area(), &lines, &state, &ctx))
            .unwrap();
        let buffer = terminal.backend().buffer();
        for cell in &buffer.content {
            assert_eq!(cell.fg, RatColor::Reset);
            assert_eq!(cell.bg, RatColor::Reset);
        }
        assert!(buffer[(0, 0)].modifier.contains(Modifier::REVERSED));
    }
    use replor_render::layout::RenderedSpan;

    fn line(text: &str) -> RenderedLine {
        RenderedLine {
            spans: vec![RenderedSpan::new(text, StyleRole::Normal)],
            source_range: None,
            no_wrap: false,
            image: None,
        }
    }

    fn text_of(spans: &[VisibleSpan]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn the_window_starts_at_the_horizontal_offset() {
        let l = line("abcdefghij");
        assert_eq!(text_of(&visible(&l, 0, 4, &[])), "abcd");
        assert_eq!(text_of(&visible(&l, 4, 4, &[])), "efgh");
        assert_eq!(text_of(&visible(&l, 8, 4, &[])), "ij");
    }

    #[test]
    fn wide_characters_are_never_split() {
        let l = line("あいうえお");
        assert_eq!(text_of(&visible(&l, 0, 4, &[])), "あい");
        // Offset 1 lands mid-character; that character is dropped, not halved.
        assert_eq!(text_of(&visible(&l, 1, 4, &[])), "いう");
    }

    #[test]
    fn scrolling_past_the_end_yields_nothing() {
        assert!(visible(&line("abc"), 10, 4, &[]).is_empty());
    }

    #[test]
    fn matches_are_marked_for_highlighting() {
        let l = line("find the needle here");
        let spans = visible(
            &l,
            0,
            40,
            &[(5, 8, Highlight::Match), (9, 15, Highlight::Current)],
        );
        let marked = |which: Highlight| -> String {
            spans
                .iter()
                .filter(|s| s.highlighted == Some(which))
                .map(|s| s.text.as_str())
                .collect()
        };
        assert_eq!(marked(Highlight::Current), "needle");
        assert_eq!(marked(Highlight::Match), "the");
        assert_eq!(text_of(&spans), "find the needle here");
    }

    #[test]
    fn a_long_path_keeps_its_tail() {
        let elided = elide_start("a/very/long/path/to/document.md", 14);
        assert!(elided.ends_with("document.md"));
        assert!(elided.starts_with('…'));
        assert!(display_width(&elided) <= 14);
    }

    #[test]
    fn a_short_path_is_untouched() {
        assert_eq!(elide_start("doc.md", 20), "doc.md");
    }

    /// Draw a frame and return it as text, one string per row.
    fn screen(state: &PagerState, lines: &[RenderedLine]) -> Vec<String> {
        let backend = ratatui::backend::TestBackend::new(state.viewport.width as u16, 12);
        let mut terminal = ratatui::Terminal::new(backend).expect("test backend");
        let ctx = ViewContext {
            selected_link: None,
            link_bar: None,
            title: "doc.md",
            flavor: "gfm",
            top_line: 1,
            lines: lines.len(),
            diagnostics: 0,
            theme: Theme::new(replor_render::layout::theme::Variant::Dark),
        };
        terminal
            .draw(|frame| draw(frame, frame.area(), lines, state, &ctx))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_overlay_lists_headings_over_the_document() {
        let lines: Vec<RenderedLine> = (0..40).map(|i| line(&format!("body {i}"))).collect();
        let mut state = PagerState::new(lines.len(), 20, 11, 40);
        state.set_outline(vec![
            crate::pager::state::OutlineItem {
                level: 1,
                text: "Title".into(),
                line: 0,
            },
            crate::pager::state::OutlineItem {
                level: 2,
                text: "Section".into(),
                line: 10,
            },
        ]);

        let closed = screen(&state, &lines).join("\n");
        assert!(!closed.contains("Headings"));

        state.apply(crate::pager::state::Action::ToggleOutline);
        let open = screen(&state, &lines).join("\n");
        assert!(open.contains("Headings"), "{open}");
        assert!(open.contains("Title"), "{open}");
        // Nested headings are indented under their parent.
        assert!(open.contains("  Section"), "{open}");
    }

    /// Every match is reversed, but the one `n` moved to is coloured, so the
    /// reader can tell where they are standing.
    #[test]
    fn the_current_match_is_drawn_apart_from_the_others() {
        let lines = vec![line("needle and needle")];
        let texts: crate::pager::search::SearchIndex = lines.iter().map(|l| l.text()).collect();
        let mut state = PagerState::new(1, 20, 3, 40);
        state.apply(crate::pager::state::Action::StartSearch);
        for c in "needle".chars() {
            state.insert_search_char(c, &texts);
        }
        state.confirm_search(&texts);

        let theme = Theme::new(replor_render::layout::theme::Variant::Dark);
        let current = convert(theme.style(StyleRole::CurrentMatch));
        let other = convert(theme.style(StyleRole::SearchMatch));
        assert_ne!(current, other);

        let backend = ratatui::backend::TestBackend::new(40, 3);
        let mut terminal = ratatui::Terminal::new(backend).expect("test backend");
        let ctx = ViewContext {
            selected_link: None,
            link_bar: None,
            title: "doc.md",
            flavor: "gfm",
            top_line: 1,
            lines: 1,
            diagnostics: 0,
            theme,
        };
        let mut style_at = |state: &PagerState, x: u16| {
            terminal
                .draw(|frame| draw(frame, frame.area(), &lines, state, &ctx))
                .expect("draw");
            let style = terminal.backend().buffer()[(x, 0)].style();
            // A cell carries resolved defaults the theme's style leaves unset.
            (style.fg, style.add_modifier)
        };
        let expected = |style: RatStyle| {
            (
                Some(style.fg.unwrap_or(RatColor::Reset)),
                style.add_modifier,
            )
        };
        let current = expected(current);
        let other = expected(other);
        // "needle and needle": the first match is current, the second is not.
        assert_eq!(style_at(&state, 0), current);
        assert_eq!(style_at(&state, 11), other);

        state.apply(crate::pager::state::Action::NextMatch);
        assert_eq!(style_at(&state, 0), other);
        assert_eq!(style_at(&state, 11), current);
    }

    #[test]
    fn the_status_bar_says_which_match_is_current() {
        let lines: Vec<RenderedLine> = (0..12)
            .map(|i| line(if i % 4 == 0 { "needle" } else { "x" }))
            .collect();
        let texts: crate::pager::search::SearchIndex = lines.iter().map(|l| l.text()).collect();
        let mut state = PagerState::new(lines.len(), 20, 5, 40);

        state.apply(crate::pager::state::Action::StartSearch);
        for c in "needle".chars() {
            state.insert_search_char(c, &texts);
        }
        // While typing, the prompt carries the count of what is typed so far,
        // at the same edge it will keep once the search is confirmed.
        let typing = screen(&state, &lines).pop().expect("a status bar");
        assert!(typing.starts_with("/needle"), "{typing}");
        assert!(typing.ends_with("(1/3)"), "{typing}");

        state.confirm_search(&texts);
        state.apply(crate::pager::state::Action::NextMatch);
        let bar = screen(&state, &lines).join("\n");
        assert!(bar.contains("/needle (2/3)"), "{bar}");
    }

    /// The status bar is drawn reversed, so the caret has to be the cell that
    /// is not; reversing it again would draw nothing.
    #[test]
    fn the_caret_cell_stands_out_from_the_prompt() {
        let style = convert(
            Theme::new(replor_render::layout::theme::Variant::Dark).style(StyleRole::Status),
        );
        let mut state = PagerState::new(10, 20, 5, 40);
        state.apply(crate::pager::state::Action::StartSearch);
        for c in "abc".chars() {
            state.insert_search_char(c, &crate::pager::search::SearchIndex::default());
        }
        state.edit_search(
            crate::pager::state::SearchEdit::CaretStart,
            &crate::pager::search::SearchIndex::default(),
        );

        let spans = prompt_spans(state.search_session().unwrap(), style, 40);
        assert_eq!(spans[0].content, "/");
        assert_eq!(spans[1].content, "a");
        assert_ne!(spans[1].style, spans[0].style);
        assert_eq!(spans[2].content, "bc");
        assert_eq!(spans[2].style, spans[0].style);
    }

    #[test]
    fn a_query_wider_than_the_bar_scrolls_with_the_caret() {
        let style = convert(
            Theme::new(replor_render::layout::theme::Variant::Dark).style(StyleRole::Status),
        );
        let mut state = PagerState::new(10, 20, 5, 40);
        state.apply(crate::pager::state::Action::StartSearch);
        for c in "abcdefghijklmnopqrstuvwxyz".chars() {
            state.insert_search_char(c, &crate::pager::search::SearchIndex::default());
        }

        let spans = prompt_spans(state.search_session().unwrap(), style, 10);
        let width: usize = spans.iter().map(|s| display_width(&s.content)).sum();
        assert!(width <= 10, "{width}");
        // The caret is still on screen, at the end of what is shown.
        assert_eq!(spans[1].content, " ");
        assert!(spans[0].content.ends_with('z'), "{}", spans[0].content);
    }

    #[test]
    fn the_help_overlay_lists_the_bindings() {
        let lines: Vec<RenderedLine> = (0..40).map(|i| line(&format!("body {i}"))).collect();
        let mut state = PagerState::new(lines.len(), 20, 11, 60);

        state.apply(crate::pager::state::Action::ToggleHelp);
        let open = screen(&state, &lines).join("\n");
        assert!(open.contains("Keys"), "{open}");
        assert!(open.contains("Tree: next entry"), "{open}");

        // A terminal too short for the whole list still reaches its end.
        assert!(!open.contains("Quit"), "{open}");
        state.scroll_help(
            help::ENTRIES.len() as isize,
            help::ENTRIES.len(),
            help_rows(state.viewport.height),
        );
        let scrolled = screen(&state, &lines).join("\n");
        assert!(scrolled.contains("Quit"), "{scrolled}");

        // A terminal that grows between key presses shows the whole list, with
        // no gap left by the offset the short screen needed.
        let taller: Vec<RenderedLine> = lines.clone();
        state.resize(taller.len(), 20, 30, 60);
        let backend = ratatui::backend::TestBackend::new(60, 31);
        let mut terminal = ratatui::Terminal::new(backend).expect("test backend");
        let ctx = ViewContext {
            selected_link: None,
            link_bar: None,
            title: "doc.md",
            flavor: "gfm",
            top_line: 1,
            lines: taller.len(),
            diagnostics: 0,
            theme: Theme::new(replor_render::layout::theme::Variant::Dark),
        };
        terminal
            .draw(|frame| draw(frame, frame.area(), &taller, &state, &ctx))
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let grown: String = (0..31)
            .map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(grown.contains("Down one line"), "{grown}");
        assert!(grown.contains("Quit"), "{grown}");

        state.apply(crate::pager::state::Action::ToggleHelp);
        assert!(!screen(&state, &lines).join("\n").contains("Keys"));
    }

    /// The pager cannot make anything clickable, but it marks the same runs the
    /// stdout backend would: a real URL is underlined, a link-styled run with
    /// any other destination is not.
    #[test]
    fn a_url_is_underlined_in_the_pager() {
        let line = RenderedLine {
            spans: vec![
                RenderedSpan::new("docs", StyleRole::Link).with_link("https://example.com"),
                RenderedSpan::new(" local", StyleRole::Link),
            ],
            source_range: None,
            no_wrap: false,
            image: None,
        };
        let lines = vec![line];
        let state = PagerState::new(1, 20, 5, 40);

        let backend = ratatui::backend::TestBackend::new(40, 3);
        let mut terminal = ratatui::Terminal::new(backend).expect("test backend");
        let ctx = ViewContext {
            selected_link: None,
            link_bar: None,
            title: "doc.md",
            flavor: "gfm",
            top_line: 1,
            lines: 1,
            diagnostics: 0,
            theme: Theme::new(replor_render::layout::theme::Variant::Dark),
        };
        terminal
            .draw(|frame| draw(frame, frame.area(), &lines, &state, &ctx))
            .expect("draw");

        let buffer = terminal.backend().buffer();
        let underlined = |x: u16| {
            buffer[(x, 0)]
                .style()
                .add_modifier
                .contains(Modifier::UNDERLINED)
        };
        assert!(underlined(0), "the URL label should be underlined");
        assert!(!underlined(6), "a non-URL link should not be");
    }

    #[test]
    fn the_outline_window_follows_the_selection() {
        assert_eq!(outline_window(0, 10, 3), 0..3);
        assert_eq!(outline_window(2, 10, 3), 0..3);
        assert_eq!(outline_window(5, 10, 3), 3..6);
        assert_eq!(outline_window(9, 10, 3), 7..10);
        // A list shorter than the window is shown whole.
        assert_eq!(outline_window(1, 2, 5), 0..2);
        assert_eq!(outline_window(0, 0, 5), 0..0);
    }

    #[test]
    fn a_long_heading_is_cut_at_a_character_boundary() {
        let cut = elide_end("日本語の長い見出し", 8);
        assert!(cut.ends_with('…'));
        assert!(display_width(&cut) <= 8);
        assert_eq!(elide_end("short", 20), "short");
    }

    #[test]
    fn widest_line_measures_display_width() {
        let lines = vec![line("abc"), line("日本語")];
        assert_eq!(widest_line(&lines), 6);
    }
}
