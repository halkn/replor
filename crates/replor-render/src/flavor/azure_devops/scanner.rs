use crate::flavor::fence::Fence;

/// One piece of the document in source order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Segment {
    Markdown { start: usize, end: usize },
    Toc { start: usize, end: usize },
    Tosp { start: usize, end: usize },
    Container(Container),
    Details(Details),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Container {
    pub(super) name: String,
    pub(super) argument: String,
    pub(super) body: String,
    pub(super) start: usize,
    pub(super) end: usize,
    /// A container that ran to the end of the file without a closing `:::`.
    pub(super) unterminated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Details {
    pub(super) summary: String,
    pub(super) body_start: usize,
    pub(super) body_end: usize,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) unterminated: bool,
}

pub(super) fn scan(text: &str, range_start: usize, range_end: usize) -> Vec<Segment> {
    if range_start >= range_end || range_end > text.len() {
        return Vec::new();
    }
    let mut segments = Vec::new();
    let mut markdown_start = range_start;
    let mut fence = None;

    let push_markdown = |segments: &mut Vec<Segment>, start: usize, end: usize| {
        if start < end {
            segments.push(Segment::Markdown { start, end });
        }
    };

    // Offsets stay absolute so segments compose with the enclosing document.
    let lines: Vec<(usize, &str)> = line_offsets(&text[range_start..range_end])
        .into_iter()
        .map(|(offset, line)| (range_start + offset, line))
        .collect();
    let mut index = 0usize;
    while index < lines.len() {
        let (line_start, line) = lines[index];
        let trimmed = line.trim();

        // Everything inside a code fence is literal, including Azure macros.
        if Fence::consume(&mut fence, line) {
            index += 1;
            continue;
        }

        if trimmed == "[[_TOC_]]" {
            push_markdown(&mut segments, markdown_start, line_start);
            segments.push(Segment::Toc {
                start: line_start,
                end: line_start + line.len(),
            });
            markdown_start = next_start(&lines, index);
            index += 1;
            continue;
        }
        if trimmed == "[[_TOSP_]]" {
            push_markdown(&mut segments, markdown_start, line_start);
            segments.push(Segment::Tosp {
                start: line_start,
                end: line_start + line.len(),
            });
            markdown_start = next_start(&lines, index);
            index += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(":::") {
            let header = rest.trim();
            // A bare `:::` with no name closes nothing here; treat it as text.
            if !header.is_empty() {
                push_markdown(&mut segments, markdown_start, line_start);
                let (container, next) = read_container(&lines, index, header, line_start);
                segments.push(Segment::Container(container));
                markdown_start = next_start(&lines, next.saturating_sub(1));
                index = next;
                continue;
            }
        }
        if details_open(trimmed)
            && let Some((details, next)) = read_details(&lines, index, line_start)
        {
            push_markdown(&mut segments, markdown_start, line_start);
            segments.push(Segment::Details(details));
            markdown_start = next_start(&lines, next.saturating_sub(1));
            index = next;
            continue;
        }

        index += 1;
    }

    push_markdown(&mut segments, markdown_start, range_end);
    segments
}

fn line_offsets(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for line in text.split_inclusive('\n') {
        out.push((start, line.trim_end_matches(['\n', '\r'])));
        start += line.len();
    }
    if out.is_empty() {
        out.push((0, ""));
    }
    out
}

fn next_start(lines: &[(usize, &str)], index: usize) -> usize {
    lines
        .get(index + 1)
        .map(|(start, _)| *start)
        .unwrap_or(usize::MAX)
}

fn read_container(
    lines: &[(usize, &str)],
    index: usize,
    header: &str,
    start: usize,
) -> (Container, usize) {
    let (name, argument) = match header.split_once(char::is_whitespace) {
        Some((name, rest)) => (name.to_string(), rest.trim().to_string()),
        None => (header.to_string(), String::new()),
    };
    let mut body = String::new();
    let mut cursor = index + 1;
    // Where the body would end if no closing `:::` is ever found. Stopping at
    // the first blank line keeps an unterminated container from swallowing the
    // rest of the document, which would make everything after it unreadable.
    let mut fallback: Option<(usize, String, usize)> = None;
    let mut fence = None;
    while cursor < lines.len() {
        let (line_start, line) = lines[cursor];
        // A `:::` inside a code fence is literal text, not a closing marker.
        if Fence::consume(&mut fence, line) {
            body.push_str(line);
            body.push('\n');
            cursor += 1;
            continue;
        }
        if line.trim().is_empty() && fallback.is_none() {
            fallback = Some((cursor, body.clone(), line_start));
        }
        if line.trim() == ":::" {
            return (
                Container {
                    name,
                    argument,
                    body,
                    start,
                    end: line_start + line.len(),
                    unterminated: false,
                },
                cursor + 1,
            );
        }
        body.push_str(line);
        body.push('\n');
        cursor += 1;
    }
    match fallback {
        Some((blank_cursor, partial, blank_start)) => (
            Container {
                name,
                argument,
                body: partial,
                start,
                end: blank_start,
                unterminated: true,
            },
            blank_cursor,
        ),
        None => {
            let end = lines.last().map(|(s, l)| s + l.len()).unwrap_or(start);
            (
                Container {
                    name,
                    argument,
                    body,
                    start,
                    end,
                    unterminated: true,
                },
                cursor,
            )
        }
    }
}

const CLOSE_TAG: &str = "</details>";

fn details_open(text: &str) -> bool {
    text.strip_prefix("<details").is_some_and(|rest| {
        rest.is_empty() || rest.starts_with(|c: char| c == '>' || c.is_ascii_whitespace())
    })
}

/// Find the extent of a `<details>` block, or `None` when its opening tag never
/// ends: a `<details` without `>` is not a tag, so it stays text.
///
/// Only a `<summary>` directly inside this block belongs to it, and the body is
/// everything between the opening tag (or the summary, when present) and the
/// `</details>` that returns the nesting depth to zero. Both bounds are byte
/// offsets rather than whole lines, so a one-line block keeps its body.
fn read_details(lines: &[(usize, &str)], index: usize, start: usize) -> Option<(Details, usize)> {
    let mut summary = String::new();
    let mut summary_seen = false;
    let mut body_start: Option<usize> = None;
    let mut depth = 0usize;
    let mut cursor = index;
    let mut last_end = start;
    let mut fence = None;
    let mut opening_tag = false;

    while cursor < lines.len() {
        let (line_start, line) = lines[cursor];
        last_end = line_start + line.len();
        let mut from = 0usize;
        if opening_tag {
            let Some(gt) = line.find('>') else {
                cursor += 1;
                continue;
            };
            from = gt + 1;
            opening_tag = false;
            body_start.get_or_insert(line_start + from);
        }
        if Fence::consume(&mut fence, line) {
            cursor += 1;
            continue;
        }

        while let Some(relative) = line[from..].find('<') {
            let at = from + relative;
            if details_open(&line[at..]) {
                depth += 1;
                let Some(gt) = line[at..].find('>') else {
                    opening_tag = true;
                    break;
                };
                from = at + gt + 1;
                body_start.get_or_insert(line_start + from);
                continue;
            }
            if depth == 1
                && !summary_seen
                && let Some((text, after)) = extract_summary(&line[at..])
            {
                summary = text;
                summary_seen = true;
                from = at + after;
                body_start = Some(line_start + from);
                continue;
            }
            if line[at..].starts_with(CLOSE_TAG) {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let close = line_start + at;
                    let body_start = body_start?.min(close);
                    return Some((
                        Details {
                            summary,
                            body_start,
                            body_end: close,
                            start,
                            end: close + CLOSE_TAG.len(),
                            unterminated: false,
                        },
                        cursor + 1,
                    ));
                }
                from = at + CLOSE_TAG.len();
            } else {
                from = at + 1;
            }
        }

        cursor += 1;
    }
    Some((
        Details {
            summary,
            body_start: body_start?.min(last_end),
            body_end: last_end,
            start,
            end: last_end,
            unterminated: true,
        },
        cursor,
    ))
}

/// The summary text and the byte offset just past `</summary>`, when `text`
/// starts with a `<summary>` that closes on the same line.
fn extract_summary(text: &str) -> Option<(String, usize)> {
    let rest = text.strip_prefix("<summary>")?;
    let open = "<summary>".len();
    let close = rest.find("</summary>")? + open;
    Some((
        text[open..close].trim().to_string(),
        close + "</summary>".len(),
    ))
}
