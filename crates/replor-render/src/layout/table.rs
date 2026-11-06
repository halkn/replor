//! GFM table layout with display-width aware column allocation.
//!
//! Columns are sized from intrinsic minimum and preferred widths. When even the
//! minimum widths do not fit, the table degrades to a vertical list rather than
//! truncating cells.

use crate::layout::inline::{InlineContext, segments};
use crate::layout::wrap::{display_width, min_unbreakable_width, spans_width, wrap_spans};
use crate::layout::{RenderedLine, RenderedSpan, StyleRole};
use crate::markdown::model::{Alignment, TableBlock, TableCell};
use crate::source::{SourceRange, SourceText};

/// Columns left for a value before the label is stacked on its own line.
const MIN_VALUE_WIDTH: usize = 8;

/// A cell already wrapped to its column width.
type CellLines = Vec<Vec<RenderedSpan>>;

pub fn layout_table(
    table: &TableBlock,
    width: usize,
    ctx: &InlineContext,
    source: &SourceText,
) -> Vec<RenderedLine> {
    let columns = column_count(table);
    if columns == 0 {
        return Vec::new();
    }

    let cells = cell_segments(table, columns, ctx);
    let (mins, preferred) = intrinsic_widths(&cells, columns);
    // Chrome is one border per gap plus a space either side of every column.
    let chrome = columns * 3 + 1;
    let available = width.saturating_sub(chrome);

    let Some(widths) = allocate(&mins, &preferred, available) else {
        return vertical_fallback(&cells, columns, width, &row_ranges(table));
    };

    let ranges = row_ranges(table);
    // The rule under the header stands where the `|---|` row is written.
    let delimiter = source.line_range(ranges[0].line_end + 1);
    render_grid(table, &cells, &widths, columns, &ranges, delimiter)
}

/// One range per entry in `cell_segments`, header first, so each rendered line
/// maps back to the row it shows. The whole table stands in for a row the
/// parser did not record.
fn row_ranges(table: &TableBlock) -> Vec<SourceRange> {
    (0..=table.rows.len())
        .map(|row| table.row_ranges.get(row).copied().unwrap_or(table.range))
        .collect()
}

fn column_count(table: &TableBlock) -> usize {
    let body = table.rows.iter().map(Vec::len).max().unwrap_or(0);
    table.header.len().max(body).max(table.alignments.len())
}

/// Flatten every cell into hard-break separated inline segments, one row at a
/// time with the header first.
fn cell_segments(table: &TableBlock, columns: usize, ctx: &InlineContext) -> Vec<Vec<CellLines>> {
    let empty = TableCell::default();
    std::iter::once(&table.header)
        .chain(table.rows.iter())
        .map(|row| {
            (0..columns)
                .map(|i| segments(&row.get(i).unwrap_or(&empty).content, ctx))
                .collect()
        })
        .collect()
}

fn intrinsic_widths(rows: &[Vec<CellLines>], columns: usize) -> (Vec<usize>, Vec<usize>) {
    let mut mins = vec![1usize; columns];
    let mut preferred = vec![1usize; columns];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            // Preferred is the widest segment on a single line; minimum is the
            // widest run with no break opportunity, so a column is never made
            // so narrow that a word has to be split.
            let pref = cell.iter().map(|s| spans_width(s)).max().unwrap_or(0);
            let min = cell
                .iter()
                .map(|s| min_unbreakable_width(s))
                .max()
                .unwrap_or(0);
            preferred[i] = preferred[i].max(pref);
            mins[i] = mins[i].max(min.max(1));
        }
    }
    (mins, preferred)
}

/// Give every column its preferred width when the table fits, otherwise grow
/// columns from their minimum in proportion to what they asked for. `None`
/// means even the minimums do not fit.
fn allocate(mins: &[usize], preferred: &[usize], available: usize) -> Option<Vec<usize>> {
    let min_total: usize = mins.iter().sum();
    if min_total > available {
        return None;
    }
    let preferred_total: usize = preferred.iter().sum();
    if preferred_total <= available {
        return Some(preferred.to_vec());
    }

    let mut widths = mins.to_vec();
    let mut slack = available - min_total;
    let demand: usize = preferred
        .iter()
        .zip(mins)
        .map(|(p, m)| p.saturating_sub(*m))
        .sum();
    if demand == 0 {
        return Some(widths);
    }
    for (i, width) in widths.iter_mut().enumerate() {
        let want = preferred[i].saturating_sub(mins[i]);
        let share = (slack.min(demand) * want) / demand;
        *width += share;
    }
    // Hand any rounding remainder to the columns that still want more.
    let mut used: usize = widths.iter().sum();
    slack = available.saturating_sub(used);
    while slack > 0 {
        let mut progressed = false;
        for (i, width) in widths.iter_mut().enumerate() {
            if slack == 0 {
                break;
            }
            if *width < preferred[i] {
                *width += 1;
                slack -= 1;
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
        used = widths.iter().sum();
        let _ = used;
    }
    Some(widths)
}

fn render_grid(
    table: &TableBlock,
    rows: &[Vec<CellLines>],
    widths: &[usize],
    columns: usize,
    ranges: &[SourceRange],
    delimiter: SourceRange,
) -> Vec<RenderedLine> {
    let mut out = Vec::new();
    // The top border belongs to the header and the bottom one to the last row.
    out.push(border_line(widths, '┌', '┬', '┐', ranges[0]));
    let mut rows_iter = rows.iter().zip(ranges);
    if let Some((header, &range)) = rows_iter.next() {
        out.extend(render_row(header, widths, table, columns, range, true));
        out.push(border_line(widths, '├', '┼', '┤', delimiter));
    }
    for (row, &range) in rows_iter {
        out.extend(render_row(row, widths, table, columns, range, false));
    }
    let last = *ranges.last().expect("the header is always present");
    out.push(border_line(widths, '└', '┴', '┘', last));
    out
}

fn border_line(
    widths: &[usize],
    left: char,
    mid: char,
    right: char,
    range: SourceRange,
) -> RenderedLine {
    let mut text = String::new();
    text.push(left);
    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            text.push(mid);
        }
        text.extend(std::iter::repeat_n('─', w + 2));
    }
    text.push(right);
    RenderedLine {
        spans: vec![RenderedSpan::new(text, StyleRole::TableBorder)],
        source_range: Some(range),
        no_wrap: true,
        image: None,
    }
}

fn render_row(
    row: &[CellLines],
    widths: &[usize],
    table: &TableBlock,
    columns: usize,
    range: SourceRange,
    header: bool,
) -> Vec<RenderedLine> {
    let wrapped: Vec<Vec<Vec<RenderedSpan>>> = (0..columns)
        .map(|i| {
            let empty: CellLines = Vec::new();
            let cell = row.get(i).unwrap_or(&empty);
            let mut lines: Vec<Vec<RenderedSpan>> = cell
                .iter()
                .flat_map(|segment| wrap_spans(segment, widths[i]))
                .collect();
            if lines.is_empty() {
                lines.push(Vec::new());
            }
            lines
        })
        .collect();

    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
    (0..height)
        .map(|line_index| {
            let mut spans = vec![RenderedSpan::new("│", StyleRole::TableBorder)];
            for (i, cell) in wrapped.iter().enumerate() {
                let empty: Vec<RenderedSpan> = Vec::new();
                let content = cell.get(line_index).unwrap_or(&empty);
                spans.push(RenderedSpan::new(" ", StyleRole::Normal));
                spans.extend(pad(content, widths[i], alignment(table, i), header));
                spans.push(RenderedSpan::new(" ", StyleRole::Normal));
                spans.push(RenderedSpan::new("│", StyleRole::TableBorder));
            }
            RenderedLine {
                spans,
                source_range: Some(range),
                no_wrap: true,
                image: None,
            }
        })
        .collect()
}

fn alignment(table: &TableBlock, column: usize) -> Alignment {
    table
        .alignments
        .get(column)
        .copied()
        .unwrap_or(Alignment::None)
}

fn pad(
    content: &[RenderedSpan],
    width: usize,
    align: Alignment,
    header: bool,
) -> Vec<RenderedSpan> {
    let used = spans_width(content);
    let slack = width.saturating_sub(used);
    let (left, right) = match align {
        Alignment::Right => (slack, 0),
        Alignment::Center => (slack / 2, slack - slack / 2),
        Alignment::Left | Alignment::None => (0, slack),
    };
    let mut spans = Vec::new();
    if left > 0 {
        spans.push(RenderedSpan::new(" ".repeat(left), StyleRole::Normal));
    }
    for span in content {
        // Header cells are emphasised unless the cell already carries a role.
        let role = if header && span.role == StyleRole::Normal {
            StyleRole::Strong
        } else {
            span.role
        };
        spans.push(RenderedSpan::new(span.text.clone(), role));
    }
    if right > 0 {
        spans.push(RenderedSpan::new(" ".repeat(right), StyleRole::Normal));
    }
    spans
}

/// Used when the terminal is too narrow for a grid. Every cell stays readable.
fn vertical_fallback(
    rows: &[Vec<CellLines>],
    columns: usize,
    width: usize,
    ranges: &[SourceRange],
) -> Vec<RenderedLine> {
    let headers: Vec<String> = (0..columns)
        .map(|i| {
            let label: String = rows
                .first()
                .and_then(|h| h.get(i))
                .map(|cell| {
                    cell.iter()
                        .flat_map(|s| s.iter().map(|sp| sp.text.as_str()))
                        .collect()
                })
                .unwrap_or_default();
            if label.trim().is_empty() {
                format!("Column {}", i + 1)
            } else {
                label
            }
        })
        .collect();

    let mut out = Vec::new();
    for (index, (row, &range)) in rows.iter().zip(ranges).skip(1).enumerate() {
        out.push(RenderedLine {
            spans: vec![RenderedSpan::new(
                format!("Row {}", index + 1),
                StyleRole::Strong,
            )],
            source_range: Some(range),
            no_wrap: false,
            image: None,
        });
        for (i, cell) in row.iter().enumerate().take(columns) {
            let label = format!("  {}: ", headers[i]);
            // Labels are measured in display columns, not characters, so a
            // Japanese header does not push the value past the target width.
            let label_width = display_width(&label);
            // A label that leaves too little room goes on its own line instead
            // of squeezing the value into a sliver.
            let stacked = label_width + MIN_VALUE_WIDTH > width;
            let indent_width = if stacked { 4 } else { label_width };
            let body_width = width.saturating_sub(indent_width).max(1);

            let lines: Vec<Vec<RenderedSpan>> = cell
                .iter()
                .flat_map(|segment| wrap_spans(segment, body_width))
                .collect();
            if stacked {
                out.push(RenderedLine {
                    spans: vec![RenderedSpan::new(
                        label.trim_end().to_string(),
                        StyleRole::Muted,
                    )],
                    source_range: Some(range),
                    no_wrap: false,
                    image: None,
                });
            }
            let indent = " ".repeat(indent_width);
            for (line_index, content) in lines.iter().enumerate() {
                let mut spans = Vec::new();
                if line_index == 0 && !stacked {
                    spans.push(RenderedSpan::new(label.clone(), StyleRole::Muted));
                } else {
                    spans.push(RenderedSpan::new(indent.clone(), StyleRole::Normal));
                }
                spans.extend(content.iter().cloned());
                out.push(RenderedLine {
                    spans,
                    source_range: Some(range),
                    no_wrap: false,
                    image: None,
                });
            }
        }
        out.push(RenderedLine::blank());
    }
    out.pop();
    out
}
