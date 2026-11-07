use unicode_segmentation::UnicodeSegmentation;

use super::search::Search;
use super::state::SearchEdit;

#[derive(Debug, Clone)]
pub struct SearchSession {
    pub input: String,
    /// Always a grapheme boundary, including after an edit joins two clusters.
    pub caret: usize,
    pub(super) preview: Search,
    pub(super) origin: (usize, usize),
}

impl SearchSession {
    pub fn new(top: usize, left: usize) -> Self {
        Self {
            input: String::new(),
            caret: 0,
            preview: Search::default(),
            origin: (top, left),
        }
    }

    pub fn insert(&mut self, c: char) {
        self.input.insert(self.caret, c);
        self.caret += c.len_utf8();
        self.normalize_caret();
    }

    /// Returns whether the query changed, so caret movement avoids a search.
    pub fn edit(&mut self, edit: SearchEdit) -> bool {
        match edit {
            SearchEdit::CaretLeft => self.caret = self.previous_boundary(),
            SearchEdit::CaretRight => self.caret = self.next_boundary(),
            SearchEdit::CaretStart => self.caret = 0,
            SearchEdit::CaretEnd => self.caret = self.input.len(),
            SearchEdit::Backspace => {
                let from = self.previous_boundary();
                self.input.replace_range(from..self.caret, "");
                self.caret = from;
            }
            SearchEdit::Delete => {
                let to = self.next_boundary();
                self.input.replace_range(self.caret..to, "");
            }
            SearchEdit::DeleteWordBefore => {
                let from = self.word_start();
                self.input.replace_range(from..self.caret, "");
                self.caret = from;
            }
            SearchEdit::KillToStart => {
                self.input.replace_range(..self.caret, "");
                self.caret = 0;
            }
            SearchEdit::KillToEnd => self.input.truncate(self.caret),
        }
        self.normalize_caret();
        !matches!(
            edit,
            SearchEdit::CaretLeft
                | SearchEdit::CaretRight
                | SearchEdit::CaretStart
                | SearchEdit::CaretEnd
        )
    }

    fn normalize_caret(&mut self) {
        self.caret = self
            .input
            .grapheme_indices(true)
            .map(|(at, _)| at)
            .find(|at| *at >= self.caret)
            .unwrap_or(self.input.len());
    }

    fn previous_boundary(&self) -> usize {
        self.input[..self.caret]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(at, _)| at)
    }

    fn next_boundary(&self) -> usize {
        self.input[self.caret..]
            .graphemes(true)
            .next()
            .map_or(self.caret, |g| self.caret + g.len())
    }

    fn word_start(&self) -> usize {
        let head = &self.input[..self.caret];
        match head.trim_end().rfind(char::is_whitespace) {
            Some(at) => at + head[at..].chars().next().map_or(1, char::len_utf8),
            None => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserting_a_joiner_keeps_the_caret_on_a_grapheme_boundary() {
        let mut session = SearchSession::new(0, 0);
        for c in "👩💻".chars() {
            session.insert(c);
        }
        session.edit(SearchEdit::CaretLeft);
        session.insert('\u{200d}');
        assert_eq!(session.input, "👩‍💻");
        assert_eq!(session.caret, session.input.len());
        session.edit(SearchEdit::Backspace);
        assert!(session.input.is_empty());
        assert_eq!(session.caret, 0);
    }
}
