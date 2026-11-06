//! Display-width aware line breaking.
//!
//! Widths come from `unicode-width` and breaks land on grapheme boundaries, so
//! byte length and scalar count are never used as a width. Japanese text breaks
//! between characters rather than between words, with JIS X 4051 kinsoku
//! applied while chunks are built rather than as a pass over finished lines: a
//! 行頭禁則 character is absorbed into the chunk before it and a 行末禁則
//! character absorbs the chunk after it. A forbidden break position therefore
//! cannot be produced in the first place, so the rules cannot undo each other.
//!
//! The sets cover the CJK and halfwidth katakana classes only. ASCII
//! punctuation is deliberately left out: Latin text already breaks on spaces,
//! so the rules would be a no-op there, and classifying ASCII would create
//! break opportunities inside words, which `min_unbreakable_width` reports to
//! the table layout as a smaller intrinsic minimum.
//!
//! Kinsoku wins over the requested width. A run with no legal break point is
//! kept whole and its line overflows (追い込み) rather than breaking at a
//! forbidden position.

use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

use unicode_width::UnicodeWidthStr;

use crate::layout::{RenderedSpan, StyleRole};

/// JIS X 4051 行頭禁則文字: must not begin a line.
fn is_no_break_start(c: char) -> bool {
    matches!(c,
        // Cl.02 終わり括弧類
        '）' | '〕' | '］' | '｝' | '〉' | '》' | '」' | '』' | '】' | '｠' | '〙' | '〗' | '»'
        // Cl.03 ハイフン類
        | '‐' | '〜'
        // Cl.04 区切り約物
        | '！' | '？' | '‼' | '⁇' | '⁈' | '⁉'
        // Cl.05 中点類
        | '・' | '：' | '；'
        // Cl.06 句点類
        | '。' | '．'
        // Cl.07 読点類
        | '、' | '，'
        // Cl.08 繰返し記号
        | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ' | '々' | '〻'
        // Cl.09 長音記号
        | 'ー'
        // Cl.10 小書きの仮名
        | 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'っ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ' | 'ゕ' | 'ゖ'
        | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ッ' | 'ャ' | 'ュ' | 'ョ' | 'ヮ' | 'ヵ' | 'ヶ'
        | '\u{31F0}'..='\u{31FF}'
        // Halfwidth katakana: 。 、 」 and the small kana run ｧ..ｯ, plus ｰ.
        | '｡' | '､' | '｣' | '\u{FF67}'..='\u{FF6F}' | 'ｰ'
    )
}

/// JIS X 4051 行末禁則文字: must not end a line.
fn is_no_break_end(c: char) -> bool {
    matches!(
        c,
        // Cl.01 始め括弧類
        '（' | '〔' | '［' | '｛' | '〈' | '《' | '「' | '『' | '【' | '｟' | '〘' | '〖' | '«'
        // Halfwidth katakana 「
        | '｢'
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    text: String,
    role: StyleRole,
    width: usize,
    /// Shared so a long link does not allocate its URL once per grapheme.
    link: Option<Rc<str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkKind {
    Space,
    Word,
}

#[derive(Debug, Clone)]
struct Chunk {
    cells: Vec<Cell>,
    width: usize,
    kind: ChunkKind,
    /// Holds a wide character. A narrow run must not join such a chunk, or the
    /// CJK/Latin boundary would become unbreakable.
    wide: bool,
    /// Ends with a 行末禁則 character, so the next cell has to join this chunk
    /// rather than start a new one.
    pending_open: bool,
}

pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub fn spans_width(spans: &[RenderedSpan]) -> usize {
    spans.iter().map(|s| display_width(&s.text)).sum()
}

/// Width of the widest run that has no break opportunity inside it, such as a
/// long identifier. Used as a table column's intrinsic minimum: narrower than
/// this and the run would have to be split mid-word.
pub fn min_unbreakable_width(spans: &[RenderedSpan]) -> usize {
    chunk(cells(spans))
        .iter()
        .filter(|c| c.kind != ChunkKind::Space)
        .map(|c| c.width)
        .max()
        .unwrap_or(0)
}

/// Break `spans` into lines no wider than `width`. A `width` of zero yields a
/// single unbroken line.
pub fn wrap_spans(spans: &[RenderedSpan], width: usize) -> Vec<Vec<RenderedSpan>> {
    if width == 0 {
        return vec![merge(cells(spans))];
    }
    let chunks = chunk(cells(spans));
    if chunks.is_empty() {
        return vec![Vec::new()];
    }

    let mut lines: Vec<Vec<Cell>> = Vec::new();
    let mut line: Vec<Chunk> = Vec::new();
    let mut used = 0usize;

    for ch in chunks {
        // A chunk carries its own kinsoku context, so a break between chunks is
        // always legal and the line needs no adjustment. `flatten` drops the
        // trailing whitespace that the break exposes.
        if used + ch.width > width && !line.is_empty() {
            lines.push(flatten(std::mem::take(&mut line)));
            used = 0;
        }
        // Leading whitespace on a continuation line is dropped, including a
        // space that overflowed: the break it caused is already represented by
        // the line boundary.
        if line.is_empty() && ch.kind == ChunkKind::Space {
            continue;
        }
        used += ch.width;
        line.push(ch);
    }
    if !line.is_empty() {
        lines.push(flatten(line));
    }

    lines
        .into_iter()
        .flat_map(|cells| split_oversized(cells, width))
        .map(merge)
        .collect()
}

fn cells(spans: &[RenderedSpan]) -> Vec<Cell> {
    let mut out = Vec::new();
    for span in spans {
        let link: Option<Rc<str>> = span.link.as_deref().map(Rc::from);
        for g in span.text.graphemes(true) {
            out.push(Cell {
                text: g.to_string(),
                role: span.role,
                width: display_width(g),
                link: link.clone(),
            });
        }
    }
    out
}

/// Group cells into the units a line break may fall between. Kinsoku is applied
/// here: a 行頭禁則 character joins the chunk before it and a 行末禁則 character
/// pulls the next cell into its own, so neither can end up on the wrong side of
/// a break.
fn chunk(cells: Vec<Cell>) -> Vec<Chunk> {
    let mut chunks: Vec<Chunk> = Vec::new();
    for cell in cells {
        let kind = cell_kind(&cell);
        let first = cell.text.chars().next().unwrap_or(' ');

        if let Some(last) = chunks.last_mut() {
            let was_pending = last.pending_open;
            // A 行頭禁則 character has nothing to attach to after a space, so it
            // is left to start a chunk of its own rather than swallowing the
            // space, which a continuation line would drop.
            let absorbs = was_pending || (is_no_break_start(first) && last.kind == ChunkKind::Word);
            if absorbs {
                last.width += cell.width;
                last.wide |= cell.width > 1;
                // A run of opening brackets, or a space after one, keeps looking
                // for the character it must not be separated from.
                last.pending_open =
                    is_no_break_end(first) || (was_pending && kind == ChunkKind::Space);
                last.cells.push(cell);
                continue;
            }
        }

        let standalone = kind != ChunkKind::Word || cell.width > 1;
        let merges = match chunks.last() {
            Some(last) => {
                last.kind == kind
                    && !(standalone && kind == ChunkKind::Word)
                    && !(last.wide && kind == ChunkKind::Word)
            }
            None => false,
        };
        if merges {
            let last = chunks.last_mut().expect("checked above");
            last.width += cell.width;
            last.wide |= cell.width > 1;
            last.cells.push(cell);
        } else {
            chunks.push(Chunk {
                width: cell.width,
                wide: cell.width > 1,
                pending_open: is_no_break_end(first),
                cells: vec![cell],
                kind,
            });
        }
    }
    chunks
}

fn cell_kind(cell: &Cell) -> ChunkKind {
    if cell.text.chars().all(char::is_whitespace) {
        ChunkKind::Space
    } else {
        ChunkKind::Word
    }
}

fn flatten(chunks: Vec<Chunk>) -> Vec<Cell> {
    let mut cells: Vec<Cell> = chunks.into_iter().flat_map(|c| c.cells).collect();
    while cells
        .last()
        .is_some_and(|c| c.text.chars().all(char::is_whitespace))
    {
        cells.pop();
    }
    cells
}

/// A run wider than the line, such as a long URL, has no break opportunity, so
/// it is split by cells. Kinsoku still applies: a split that would strand a
/// 行頭禁則 character at the head of a line, or leave a 行末禁則 character at
/// the tail, is deferred to the next legal position and the line overflows
/// instead (追い込み). Content is never dropped.
fn split_oversized(cells: Vec<Cell>, width: usize) -> Vec<Vec<Cell>> {
    if cells.iter().map(|c| c.width).sum::<usize>() <= width {
        return vec![cells];
    }
    let mut lines = Vec::new();
    let mut current: Vec<Cell> = Vec::new();
    let mut used = 0usize;
    for cell in cells {
        if used + cell.width > width && !current.is_empty() && may_break_between(&current, &cell) {
            lines.push(std::mem::take(&mut current));
            used = 0;
        }
        used += cell.width;
        current.push(cell);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Whether a line may end with `current` and the next one begin with `next`.
fn may_break_between(current: &[Cell], next: &Cell) -> bool {
    let base = |cell: &Cell| cell.text.chars().next();
    !base(next).is_some_and(is_no_break_start)
        && !current.last().and_then(base).is_some_and(is_no_break_end)
}

fn merge(cells: Vec<Cell>) -> Vec<RenderedSpan> {
    let mut spans: Vec<RenderedSpan> = Vec::new();
    for cell in cells {
        match spans.last_mut() {
            Some(last)
                if last.role == cell.role && last.link.as_deref() == cell.link.as_deref() =>
            {
                last.text.push_str(&cell.text)
            }
            _ => {
                let span = RenderedSpan::new(cell.text, cell.role);
                spans.push(match cell.link {
                    Some(link) => span.with_link(link.as_ref()),
                    None => span,
                });
            }
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal(text: &str) -> Vec<RenderedSpan> {
        vec![RenderedSpan::new(text, StyleRole::Normal)]
    }

    fn texts(lines: &[Vec<RenderedSpan>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.iter().map(|s| s.text.as_str()).collect())
            .collect()
    }

    fn widths(lines: &[Vec<RenderedSpan>]) -> Vec<usize> {
        lines.iter().map(|l| spans_width(l)).collect()
    }

    #[test]
    fn measures_wide_characters_as_two_columns() {
        assert_eq!(display_width("日本語"), 6);
        assert_eq!(display_width("abc"), 3);
    }

    #[test]
    fn breaks_latin_text_between_words() {
        let out = wrap_spans(&normal("the quick brown fox"), 10);
        assert_eq!(texts(&out), vec!["the quick", "brown fox"]);
    }

    #[test]
    fn never_exceeds_the_requested_width() {
        let out = wrap_spans(&normal("日本語のテキストを折り返します"), 8);
        assert!(widths(&out).iter().all(|w| *w <= 8), "{:?}", widths(&out));
    }

    #[test]
    fn breaks_japanese_between_characters_without_adding_spaces() {
        let out = wrap_spans(&normal("日本語テキスト"), 6);
        assert_eq!(texts(&out), vec!["日本語", "テキス", "ト"]);
    }

    #[test]
    fn keeps_closing_punctuation_off_the_start_of_a_line() {
        // Width 8 would otherwise leave "。" alone at the head of line 2.
        let out = wrap_spans(&normal("あいうえお。かき"), 8);
        for line in texts(&out).iter().skip(1) {
            assert!(!line.starts_with('。'), "{line:?}");
        }
    }

    #[test]
    fn keeps_opening_punctuation_off_the_end_of_a_line() {
        let out = wrap_spans(&normal("あいう「かきくけこ」"), 8);
        for line in texts(&out) {
            assert!(!line.ends_with('「'), "{line:?}");
        }
    }

    /// Every JIS X 4051 行頭禁則 class beyond the closing brackets the original
    /// set covered. One representative character each; the width is chosen so
    /// the character lands exactly where a naive break would put it.
    #[test]
    fn keeps_every_no_break_start_class_off_the_start_of_a_line() {
        for (text, forbidden) in [
            ("あいうえおかきくちょっと", 'ょ'), // Cl.10 小書きの仮名
            ("あいうえおかきくサーバー", 'ー'), // Cl.09 長音記号
            ("あいうえおかきくと！です", '！'), // Cl.04 区切り約物
            ("あいうえおかきくけ・こさ", '・'), // Cl.05 中点類
            ("あいうえおかきく人々の暮", '々'), // Cl.08 繰返し記号
            ("あいうえおかきくけこ、さ", '、'), // Cl.07 読点類
        ] {
            let out = texts(&wrap_spans(&normal(text), 16));
            for line in out.iter().skip(1) {
                assert!(
                    !line.starts_with(forbidden),
                    "{forbidden:?} started a line in {out:?}"
                );
            }
            assert_eq!(out.concat(), text, "characters were lost: {out:?}");
        }
    }

    /// Regression: the closing rule used to pull a chunk down and re-expose the
    /// opening bracket that the opening rule had just cleared, so `「` ended the
    /// line after all. Kinsoku now lives in `chunk`, where the two cannot fight.
    #[test]
    fn a_closing_bracket_does_not_strand_the_opening_one_at_the_line_end() {
        let out = texts(&wrap_spans(&normal("あいうえおかきく「け」"), 20));
        assert_eq!(out, vec!["あいうえおかきく", "「け」"]);
    }

    #[test]
    fn a_run_of_closing_punctuation_stays_with_the_text_it_follows() {
        let out = texts(&wrap_spans(&normal("あいうえおかきくけ「こ」。"), 20));
        assert_eq!(out, vec!["あいうえおかきくけ", "「こ」。"]);
    }

    /// 追い込み: with no legal break position left, the line is allowed to run
    /// past the requested width rather than break where kinsoku forbids.
    #[test]
    fn overflows_the_width_rather_than_breaking_at_a_forbidden_position() {
        let text = "あ。。。。。。。。。。。";
        let out = wrap_spans(&normal(text), 20);
        assert_eq!(texts(&out), vec![text]);
        assert!(
            widths(&out)[0] > 20,
            "expected an overflow: {:?}",
            widths(&out)
        );
    }

    #[test]
    fn min_unbreakable_width_measures_the_longest_run_without_a_break() {
        assert_eq!(min_unbreakable_width(&normal("abc defgh")), 5);
        // Wide characters break between each other, so a bare CJK run is 2.
        assert_eq!(min_unbreakable_width(&normal("日本語")), 2);
        // Kinsoku makes `す。` a single unit, which is the cell's real minimum.
        assert_eq!(min_unbreakable_width(&normal("日本語です。")), 4);
        assert_eq!(min_unbreakable_width(&[]), 0);
    }

    #[test]
    fn splits_a_word_longer_than_the_line() {
        let out = wrap_spans(&normal("aaaaaaaaaaaa"), 5);
        assert!(widths(&out).iter().all(|w| *w <= 5));
        assert_eq!(texts(&out).concat(), "aaaaaaaaaaaa");
    }

    #[test]
    fn preserves_roles_across_a_break() {
        let spans = vec![
            RenderedSpan::new("hello ", StyleRole::Normal),
            RenderedSpan::new("world", StyleRole::Strong),
        ];
        let out = wrap_spans(&spans, 6);
        assert_eq!(texts(&out), vec!["hello", "world"]);
        assert_eq!(out[1][0].role, StyleRole::Strong);
    }

    #[test]
    fn a_link_survives_a_line_break_and_does_not_bleed_into_its_neighbour() {
        let spans = vec![
            RenderedSpan::new("alpha beta", StyleRole::Link).with_link("https://example.com"),
            RenderedSpan::new(" gamma", StyleRole::Link),
        ];
        let url = || Some("https://example.com".to_string());
        let links = |lines: &[Vec<RenderedSpan>]| -> Vec<Vec<Option<String>>> {
            lines
                .iter()
                .map(|line| line.iter().map(|s| s.link.clone()).collect())
                .collect()
        };

        // Both halves of a broken label stay clickable.
        let broken = wrap_spans(&spans[..1], 7);
        assert_eq!(links(&broken), vec![vec![url()], vec![url()]]);

        // The unlinked run stays a separate span even though the role matches.
        let joined = wrap_spans(&spans, 40);
        assert_eq!(links(&joined), vec![vec![url(), None]]);
    }

    #[test]
    fn merges_adjacent_cells_of_the_same_role() {
        let out = wrap_spans(&normal("abc def"), 80);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 1);
    }

    #[test]
    fn a_latin_run_may_break_away_from_preceding_japanese() {
        // The Latin run must not fuse to the wide character before it, which
        // would leave an unbreakable chunk and waste most of the line.
        let out = wrap_spans(&normal("日本語日本語abcdefghij日本語です"), 20);
        assert!(widths(&out).iter().all(|w| *w <= 20));
        assert_eq!(texts(&out)[0], "日本語日本語");
        assert_eq!(texts(&out).concat(), "日本語日本語abcdefghij日本語です");
    }

    #[test]
    fn mixed_japanese_and_latin_stays_within_width() {
        let out = wrap_spans(&normal("これは mixed テキスト with English です"), 12);
        assert!(widths(&out).iter().all(|w| *w <= 12), "{:?}", texts(&out));
        assert!(texts(&out).concat().contains("English"));
    }

    /// Regression: the space that caused the break used to be carried onto the
    /// continuation line, which only the width boundary exposes.
    #[test]
    fn a_space_that_overflows_does_not_indent_the_next_line() {
        let out = wrap_spans(
            &normal("Helpful advice for doing things better or more easily."),
            38,
        );
        assert_eq!(
            texts(&out),
            vec!["Helpful advice for doing things better", "or more easily."]
        );
    }

    #[test]
    fn a_space_that_overflows_does_not_leave_a_blank_line_in_a_paragraph() {
        let text = "Helpful advice for doing things better \
                    abcdefghijklmnopqrstuvwxyzabcdefghijkl end.";
        let out = texts(&wrap_spans(&normal(text), 38));
        assert!(!out.iter().any(|l| l.is_empty()), "{out:?}");
        assert_eq!(out.join(" "), text);
    }

    #[test]
    fn empty_input_yields_one_empty_line() {
        assert_eq!(wrap_spans(&[], 10), vec![Vec::new()]);
    }

    #[test]
    fn zero_width_does_not_wrap() {
        let out = wrap_spans(&normal("a b c"), 0);
        assert_eq!(texts(&out), vec!["a b c"]);
    }
}
