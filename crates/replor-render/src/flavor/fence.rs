#[derive(Debug, Clone, Copy)]
pub(super) struct Fence {
    marker: u8,
    length: usize,
}

impl Fence {
    fn start(line: &str) -> Option<Self> {
        let text = line.trim_start_matches(' ');
        if line.len() - text.len() > 3 {
            return None;
        }
        let marker = *text.as_bytes().first()?;
        if !matches!(marker, b'`' | b'~') {
            return None;
        }
        let length = text.bytes().take_while(|b| *b == marker).count();
        if length < 3 || (marker == b'`' && text[length..].contains('`')) {
            return None;
        }
        Some(Self { marker, length })
    }

    fn closes(self, line: &str) -> bool {
        let text = line.trim_start_matches(' ');
        if line.len() - text.len() > 3 {
            return false;
        }
        let length = text.bytes().take_while(|b| *b == self.marker).count();
        length >= self.length && text[length..].trim().is_empty()
    }

    /// Returns whether this line belongs to a fenced code block.
    pub fn consume(state: &mut Option<Self>, line: &str) -> bool {
        if let Some(fence) = *state {
            if fence.closes(line) {
                *state = None;
            }
            true
        } else if let Some(fence) = Self::start(line) {
            *state = Some(fence);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_matching_bare_fence_of_sufficient_length_closes() {
        for opening in ["````rust", "~~~~"] {
            let marker = &opening[..1];
            let mut state = None;
            assert!(Fence::consume(&mut state, opening));
            for line in [
                marker.repeat(3),
                format!("{} text", marker.repeat(4)),
                format!("    {}", marker.repeat(4)),
            ] {
                assert!(Fence::consume(&mut state, &line));
                assert!(state.is_some());
            }
            assert!(Fence::consume(&mut state, &marker.repeat(5)));
            assert!(state.is_none());
        }
        assert!(!Fence::consume(&mut None, "```a`b"));
    }
}
