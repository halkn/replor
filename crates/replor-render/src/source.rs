/// A byte range in the source document, paired with the 1-based line span it
/// covers. Every user-authored block carries one so the pager can map rendered
/// lines back to the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRange {
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: usize,
    pub line_end: usize,
}

/// The whole input plus a line index built once at load time.
#[derive(Debug, Clone)]
pub struct SourceText {
    text: String,
    /// Byte offset of the first character of each line.
    line_starts: Vec<usize>,
}

impl SourceText {
    pub fn new(text: String) -> Self {
        let mut line_starts = vec![0usize];
        line_starts.extend(
            text.bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .map(|(i, _)| i + 1),
        );
        // A trailing newline does not begin a real line.
        if line_starts.last() == Some(&text.len()) && !text.is_empty() {
            line_starts.pop();
        }
        Self { text, line_starts }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// 1-based line containing `byte`. Offsets past the end clamp to the last line.
    pub fn line_of(&self, byte: usize) -> usize {
        match self.line_starts.binary_search(&byte) {
            Ok(i) => i + 1,
            Err(i) => i.max(1),
        }
    }

    /// Build a range from a byte span produced by the parser.
    pub fn range(&self, byte_start: usize, byte_end: usize) -> SourceRange {
        let byte_end = byte_end.min(self.text.len());
        let byte_start = byte_start.min(byte_end);
        // An exclusive end that sits exactly on a line start belongs to the
        // previous line, otherwise a block would claim the line after it.
        let last_byte = byte_end.saturating_sub(1).max(byte_start);
        SourceRange {
            byte_start,
            byte_end,
            line_start: self.line_of(byte_start),
            line_end: self.line_of(last_byte),
        }
    }

    /// The range of one 1-based line, clamped into the document.
    pub fn line_range(&self, line: usize) -> SourceRange {
        let line = line.clamp(1, self.line_count());
        SourceRange {
            byte_start: self.line_starts[line - 1],
            byte_end: self
                .line_starts
                .get(line)
                .copied()
                .unwrap_or(self.text.len()),
            line_start: line,
            line_end: line,
        }
    }

    /// Text of a 1-based line, without its line terminator.
    #[cfg(test)]
    pub fn line(&self, line: usize) -> Option<&str> {
        (1..=self.line_count()).contains(&line).then(|| {
            let range = self.line_range(line);
            self.text[range.byte_start..range.byte_end].trim_end_matches(['\n', '\r'])
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_lf_documents() {
        let s = SourceText::new("a\nbb\nccc\n".to_string());
        assert_eq!(s.line_count(), 3);
        assert_eq!(s.line_of(0), 1);
        assert_eq!(s.line_of(2), 2);
        assert_eq!(s.line_of(5), 3);
        assert_eq!(s.line(2), Some("bb"));
    }

    #[test]
    fn indexes_crlf_documents() {
        let s = SourceText::new("a\r\nbb\r\n".to_string());
        assert_eq!(s.line_count(), 2);
        assert_eq!(s.line(1), Some("a"));
        assert_eq!(s.line(2), Some("bb"));
    }

    #[test]
    fn counts_a_final_line_without_a_terminator() {
        let s = SourceText::new("a\nb".to_string());
        assert_eq!(s.line_count(), 2);
        assert_eq!(s.line(2), Some("b"));
    }

    #[test]
    fn an_empty_document_has_one_line() {
        let s = SourceText::new(String::new());
        assert_eq!(s.line_count(), 1);
        assert_eq!(s.line(1), Some(""));
    }

    #[test]
    fn exclusive_end_stays_on_the_last_covered_line() {
        let s = SourceText::new("# Title\n\npara\n".to_string());
        // "# Title\n" spans bytes 0..8 but must report line 1..1, not 1..2.
        let r = s.range(0, 8);
        assert_eq!((r.line_start, r.line_end), (1, 1));
    }

    #[test]
    fn multiline_ranges_span_their_lines() {
        let s = SourceText::new("a\nb\nc\n".to_string());
        let r = s.range(0, 4);
        assert_eq!((r.line_start, r.line_end), (1, 2));
    }
}
