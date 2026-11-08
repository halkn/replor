//! Where a link in the preview leads, and the places to go back to.
//!
//! A link is resolved on the snapshot's paths, so a link out of the snapshot
//! is refused by the same rules as an image, whatever the snapshot is.

use replor_render::snapshot::{DocumentSource, SnapshotError, SnapshotPath};

/// Where a link destination leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// A heading in the open document, by its anchor id.
    Here(String),
    /// Another file, and a heading in it when the link names one.
    File {
        path: SnapshotPath,
        fragment: Option<String>,
    },
    /// A URL. Shown, never opened.
    External(String),
}

/// Resolves `dest` from the open document. `Err` says why the link cannot be
/// followed.
pub fn resolve(source: Option<&DocumentSource>, dest: &str) -> Result<Destination, String> {
    if has_scheme(dest) {
        return Ok(Destination::External(dest.to_string()));
    }
    let (path, fragment) = match dest.split_once('#') {
        Some((path, fragment)) => (path, Some(percent_decoded(fragment))),
        None => (dest, None),
    };
    if path.is_empty() {
        return match fragment {
            Some(fragment) => Ok(Destination::Here(fragment)),
            None => Err("the link is empty".to_string()),
        };
    }
    let source = source.ok_or("this document has no files to link to")?;
    let path =
        SnapshotPath::from_reference(source.path(), source.content_root(), &percent_decoded(path))
            .map_err(|_| "the link leaves the repository".to_string())?;
    // Checked now so a link that cannot be read is not followed at all.
    source.snapshot().resolve(&path).map_err(|err| match err {
        SnapshotError::OutsideRoot => "the link leaves the repository".to_string(),
        SnapshotError::NotFound => format!("{path} does not exist"),
        SnapshotError::NotAFile => format!("{path} is not a file"),
        other => format!("{path}: {other}"),
    })?;
    Ok(Destination::File {
        path,
        fragment: fragment.filter(|f| !f.is_empty()),
    })
}

/// The id a fragment names. GitHub ids are lower case, and a link written by
/// hand may not be.
pub fn find_anchor<'a>(
    anchors: &'a [replor_render::navigation::Anchor],
    fragment: &str,
) -> Option<&'a replor_render::navigation::Anchor> {
    anchors
        .iter()
        .find(|anchor| anchor.id == fragment)
        .or_else(|| {
            let lower = fragment.to_lowercase();
            anchors.iter().find(|anchor| anchor.id == lower)
        })
}

/// `scheme:` per RFC 3986, at least two characters so a Windows drive letter
/// is not taken for one.
fn has_scheme(dest: &str) -> bool {
    let Some((scheme, _)) = dest.split_once(':') else {
        return false;
    };
    scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |b: Option<&u8>| b.and_then(|b| char::from(*b).to_digit(16));
        match (bytes[at], hex(bytes.get(at + 1)), hex(bytes.get(at + 2))) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                at += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

/// A file and a line of the file, which a re-layout cannot move. It is the
/// file's line, not the document's, so it holds across Preview and Source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub path: SnapshotPath,
    pub line: usize,
}

/// Places to go back and forward to, as a browser keeps them.
#[derive(Debug, Default)]
pub struct History {
    back: Vec<Place>,
    forward: Vec<Place>,
}

/// Places kept in each direction. The oldest is dropped beyond this.
pub const HISTORY_LIMIT: usize = 100;

impl History {
    /// Records `from` before following a link. Going somewhere new forgets
    /// the places ahead.
    pub fn visit(&mut self, from: Place) {
        push(&mut self.back, from);
        self.forward.clear();
    }

    /// The place to go back to, with `current` kept to come forward to.
    pub fn back(&mut self, current: Place) -> Option<Place> {
        let place = self.back.pop()?;
        push(&mut self.forward, current);
        Some(place)
    }

    pub fn forward(&mut self, current: Place) -> Option<Place> {
        let place = self.forward.pop()?;
        push(&mut self.back, current);
        Some(place)
    }
}

fn push(stack: &mut Vec<Place>, place: Place) {
    if stack.len() == HISTORY_LIMIT {
        stack.remove(0);
    }
    stack.push(place);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(path: &str, line: usize) -> Place {
        Place {
            path: SnapshotPath::new(path).unwrap(),
            line,
        }
    }

    #[test]
    fn back_and_forward_walk_the_places() {
        let mut history = History::default();
        history.visit(place("a.md", 1));
        history.visit(place("b.md", 5));
        assert_eq!(history.back(place("c.md", 9)), Some(place("b.md", 5)));
        assert_eq!(history.back(place("b.md", 5)), Some(place("a.md", 1)));
        assert_eq!(history.back(place("a.md", 1)), None);
        assert_eq!(history.forward(place("a.md", 1)), Some(place("b.md", 5)));
        // Following a link from here forgets what was ahead.
        history.visit(place("b.md", 5));
        assert_eq!(history.forward(place("d.md", 1)), None);
    }

    #[test]
    fn the_oldest_place_is_dropped_beyond_the_limit() {
        let mut history = History::default();
        for line in 0..HISTORY_LIMIT + 5 {
            history.visit(place("a.md", line));
        }
        let mut count = 0;
        let mut current = place("z.md", 0);
        while let Some(previous) = history.back(current.clone()) {
            count += 1;
            current = previous;
        }
        assert_eq!(count, HISTORY_LIMIT);
        assert_eq!(current.line, 5);
    }

    #[test]
    fn a_url_is_external_and_a_fragment_alone_stays_here() {
        assert_eq!(
            resolve(None, "https://example.com/x"),
            Ok(Destination::External("https://example.com/x".into()))
        );
        assert_eq!(
            resolve(None, "#s1-%E6%99%AE%E6%AE%B5"),
            Ok(Destination::Here("s1-普段".into()))
        );
        assert!(resolve(None, "a.md").is_err());
        assert!(!has_scheme("C:/x.md"));
    }
}
