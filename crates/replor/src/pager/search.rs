//! Search over rendered plain text.
//!
//! Matching runs against what is on screen rather than the Markdown source, so
//! a query finds what the reader can actually see.

use std::cell::OnceCell;
use std::ops::{Deref, Range};

#[derive(Debug, Clone, Default)]
pub struct SearchIndex {
    lines: Vec<String>,
    /// Built by the first case-insensitive search: the index is rebuilt on
    /// every resize and reload, mostly for a reader who never searches.
    folded: OnceCell<Vec<FoldedLine>>,
}

impl SearchIndex {
    fn folded(&self) -> &[FoldedLine] {
        self.folded.get_or_init(|| {
            self.lines
                .iter()
                .map(|text| FoldedLine::new(text))
                .collect()
        })
    }
}

#[derive(Debug, Clone)]
struct FoldedLine {
    text: String,
    changes: Vec<(Range<usize>, Range<usize>)>,
}

impl FoldedLine {
    fn new(text: &str) -> Self {
        let mut changes = Vec::new();
        let mut folded = String::with_capacity(text.len());
        for (original, ch) in text.char_indices() {
            let start = folded.len();
            folded.extend(ch.to_lowercase());
            if folded.len() - start != ch.len_utf8() {
                changes.push((start..folded.len(), original..original + ch.len_utf8()));
            }
        }
        Self {
            text: folded,
            changes,
        }
    }

    fn original_offset(&self, offset: usize, end: bool) -> usize {
        let index = self
            .changes
            .partition_point(|(folded, _)| folded.end <= offset);
        if let Some((folded, original)) = self.changes.get(index)
            && offset >= folded.start
        {
            return if end && offset > folded.start {
                original.end
            } else {
                original.start
            };
        }
        match index.checked_sub(1).and_then(|i| self.changes.get(i)) {
            Some((folded, original)) => offset - folded.end + original.end,
            None => offset,
        }
    }
}

impl FromIterator<String> for SearchIndex {
    fn from_iter<T: IntoIterator<Item = String>>(lines: T) -> Self {
        Self {
            lines: lines.into_iter().collect(),
            folded: OnceCell::new(),
        }
    }
}

impl Deref for SearchIndex {
    type Target = [String];

    fn deref(&self) -> &Self::Target {
        &self.lines
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    /// Byte offsets into the rendered line's text.
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Search {
    query: String,
    matches: Vec<Match>,
    current: Option<usize>,
    /// The current match is where the next step goes rather than where the
    /// reader is: it was chosen without going there (`aim_from`).
    aimed: bool,
}

impl Search {
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn is_active(&self) -> bool {
        !self.query.is_empty()
    }

    #[cfg(test)]
    pub fn matches(&self) -> &[Match] {
        &self.matches
    }

    pub fn current(&self) -> Option<Match> {
        self.current.and_then(|i| self.matches.get(i).copied())
    }

    pub fn count(&self) -> usize {
        self.matches.len()
    }

    /// Matches on a given rendered line, for highlighting.
    pub fn matches_on(&self, line: usize) -> impl Iterator<Item = &Match> {
        let start = self.matches.partition_point(|m| m.line < line);
        let end = self.matches.partition_point(|m| m.line <= line);
        self.matches[start..end].iter()
    }

    /// Apply a query. An empty query keeps the previous one, which is what
    /// pressing Enter on an empty search prompt should do.
    pub fn set_query(&mut self, query: &str, lines: &SearchIndex) {
        if !query.is_empty() {
            self.query = query.to_string();
        }
        self.recompute(lines);
    }

    /// Apply a query as typed, including an empty one. Used by the prompt while
    /// it is being edited, where deleting the last character has to clear the
    /// matches rather than fall back to the previous query.
    pub fn replace_query(&mut self, query: &str, lines: &SearchIndex) {
        self.query = query.to_string();
        self.recompute(lines);
    }

    /// Zero-based position of the current match, for the status bar.
    pub fn position(&self) -> Option<usize> {
        self.current
    }

    /// Re-run the search. Called after a re-layout, since rendered line numbers
    /// change with width. The cursor lands on the first match; callers that
    /// want it near the reader follow with `select_from`.
    pub fn recompute(&mut self, lines: &SearchIndex) {
        self.matches.clear();
        self.aimed = false;
        if self.query.is_empty() {
            self.current = None;
            return;
        }
        // Smart case, as in moar and vim: a query typed in lower case matches
        // any case, and adding a capital asks for that capital.
        let fold = !self.query.chars().any(char::is_uppercase);
        let needle = if fold {
            self.query.to_lowercase()
        } else {
            self.query.clone()
        };
        let folded = fold.then(|| lines.folded());
        for (index, line) in lines.lines.iter().enumerate() {
            let folded = folded.map(|folded| &folded[index]);
            let haystack = folded.map_or(line.as_str(), |folded| folded.text.as_str());
            let mut from = 0usize;
            while let Some(found) = haystack[from..].find(&needle) {
                let start = from + found;
                let end = start + needle.len();
                self.matches.push(match folded {
                    None => Match {
                        line: index,
                        start,
                        end,
                    },
                    Some(folded) => Match {
                        line: index,
                        start: folded.original_offset(start, false),
                        end: folded.original_offset(end, true),
                    },
                });
                from = end.max(start + 1);
                if from >= haystack.len() {
                    break;
                }
            }
        }
        self.current = if self.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Select the first match at or after `line`, so a search starts from the
    /// current viewport rather than from the top of the document.
    pub fn select_from(&mut self, line: usize) {
        self.aimed = false;
        if self.matches.is_empty() {
            self.current = None;
            return;
        }
        self.current = Some(
            self.matches
                .iter()
                .position(|m| m.line >= line)
                .unwrap_or(0),
        );
    }

    /// Select the first match at or after `line` without going to it, so the
    /// next step goes there and the previous to the one before: none is
    /// skipped while the reader has seen neither.
    pub fn aim_from(&mut self, line: usize) {
        self.select_from(line);
        self.aimed = self.current.is_some();
    }

    pub fn is_aimed(&self) -> bool {
        self.aimed
    }

    // mdvu's name, kept so the pager reads as it does there.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Match> {
        self.step(1)
    }

    pub fn previous(&mut self) -> Option<Match> {
        self.step(-1)
    }

    fn step(&mut self, delta: isize) -> Option<Match> {
        if self.matches.is_empty() {
            return None;
        }
        let len = self.matches.len() as isize;
        let current = self.current.unwrap_or(0) as isize;
        let delta = if std::mem::take(&mut self.aimed) && delta > 0 {
            0
        } else {
            delta
        };
        let next = (current + delta).rem_euclid(len) as usize;
        self.current = Some(next);
        self.matches.get(next).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folded_copy_is_built_only_for_a_case_insensitive_search() {
        let lines: SearchIndex = vec!["Alpha".to_string()].into_iter().collect();
        let mut search = Search::default();
        search.set_query("Alpha", &lines);
        assert!(lines.folded.get().is_none());
        search.set_query("alpha", &lines);
        assert_eq!(search.count(), 1);
        assert!(lines.folded.get().is_some());
    }

    #[test]
    fn folded_offsets_map_back_to_original_characters() {
        let lines: SearchIndex = vec!["İẞ tail".to_string()].into_iter().collect();
        let mut search = Search::default();
        search.set_query("ß", &lines);
        let found = search.current().unwrap();
        assert_eq!(&lines[0][found.start..found.end], "ẞ");
        search.set_query("i", &lines);
        let found = search.current().unwrap();
        assert_eq!(&lines[0][found.start..found.end], "İ");
    }

    fn lines() -> SearchIndex {
        vec![
            "The quick brown fox".to_string(),
            "jumps over".to_string(),
            "the lazy dog, the end".to_string(),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn a_lower_case_query_matches_any_case() {
        let mut s = Search::default();
        s.set_query("the", &lines());
        assert_eq!(s.count(), 3);
        assert_eq!(s.matches()[0].line, 0);
    }

    #[test]
    fn a_capital_in_the_query_asks_for_that_capital() {
        let lines: SearchIndex = vec!["The quick".to_string(), "the lazy".to_string()]
            .into_iter()
            .collect();
        let mut s = Search::default();
        s.set_query("The", &lines);
        assert_eq!(s.count(), 1);
        assert_eq!(s.matches()[0].line, 0);
        // Offsets still locate the match when the query is not folded.
        let m = s.matches()[0];
        assert_eq!(&lines[m.line][m.start..m.end], "The");
    }

    #[test]
    fn multiple_matches_on_one_line_are_all_found() {
        let mut s = Search::default();
        s.set_query("the", &lines());
        let on_last: Vec<_> = s.matches_on(2).collect();
        assert_eq!(on_last.len(), 2);
    }

    #[test]
    fn offsets_locate_the_match_within_the_line() {
        let mut s = Search::default();
        s.set_query("brown", &lines());
        let m = s.matches()[0];
        assert_eq!(&lines()[m.line][m.start..m.end], "brown");
    }

    #[test]
    fn navigation_cycles_in_both_directions() {
        let mut s = Search::default();
        s.set_query("the", &lines());
        assert_eq!(s.current().unwrap().line, 0);
        s.next();
        assert_eq!(s.current().unwrap().line, 2);
        s.next();
        assert_eq!(s.current().unwrap().line, 2);
        s.next();
        // Wraps back to the first match.
        assert_eq!(s.current().unwrap().line, 0);
        s.previous();
        assert_eq!(s.current().unwrap().line, 2);
    }

    #[test]
    fn an_empty_query_keeps_the_previous_one() {
        let mut s = Search::default();
        s.set_query("fox", &lines());
        s.set_query("", &lines());
        assert_eq!(s.query(), "fox");
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn a_query_with_no_match_reports_zero() {
        let mut s = Search::default();
        s.set_query("absent", &lines());
        assert_eq!(s.count(), 0);
        assert!(s.current().is_none());
        assert!(s.next().is_none());
    }

    #[test]
    fn selection_starts_from_the_viewport() {
        let mut s = Search::default();
        s.set_query("the", &lines());
        s.select_from(2);
        assert_eq!(s.current().unwrap().line, 2);
    }

    #[test]
    fn japanese_queries_match() {
        let lines: SearchIndex = vec!["日本語のテキスト".to_string(), "英語の text".to_string()]
            .into_iter()
            .collect();
        let mut s = Search::default();
        s.set_query("テキスト", &lines);
        assert_eq!(s.count(), 1);
        let m = s.matches()[0];
        assert_eq!(&lines[m.line][m.start..m.end], "テキスト");
    }
}
