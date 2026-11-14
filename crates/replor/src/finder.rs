//! Finding a file by its path: the `Ctrl-p` prompt of the Files lens.
//!
//! The candidates are listed on a thread of their own, through the Snapshot,
//! and collected by the frame loop between key presses, so a large
//! repository can be read and searched while the listing goes on.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use replor_render::snapshot::{Entry, Snapshot, WalkLimits, Walked, walk};

/// replor-go's bounds (`docs/plans/initial/DESIGN.md`, 6.2).
pub const LIMITS: WalkLimits = WalkLimits {
    entries: 200_000,
    depth: 64,
};

/// Entries sent at once, so the frame loop is not woken for every file.
const BATCH: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Listing,
    Complete,
    /// A limit was reached, so some files are not candidates.
    Partial,
}

enum Message {
    Entries(Vec<Entry>),
    Done(Walked),
}

pub struct Candidate {
    pub entry: Entry,
    /// The path lowercased once, rather than for every key.
    lower: String,
    /// Where the file name starts in `lower`.
    name: usize,
}

impl Candidate {
    fn new(entry: Entry) -> Self {
        let lower = entry.path.as_str().to_lowercase();
        let name = lower.rfind('/').map_or(0, |slash| slash + 1);
        Self { entry, lower, name }
    }
}

/// Every file of the snapshot, as far as the listing has got.
pub struct Candidates {
    entries: Vec<Candidate>,
    /// Dropped when the listing is replaced, which makes its thread stop at
    /// its next send.
    receiver: Option<Receiver<Message>>,
    progress: Progress,
}

impl Candidates {
    pub fn start(snapshot: Arc<dyn Snapshot>, limits: WalkLimits) -> Self {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut batch = Vec::new();
            let walked = walk(&*snapshot, limits, |entry| {
                batch.push(entry);
                if batch.len() < BATCH {
                    return true;
                }
                sender
                    .send(Message::Entries(std::mem::take(&mut batch)))
                    .is_ok()
            });
            if walked != Walked::Stopped {
                let _ = sender.send(Message::Entries(batch));
                let _ = sender.send(Message::Done(walked));
            }
        });
        Self {
            entries: Vec::new(),
            receiver: Some(receiver),
            progress: Progress::Listing,
        }
    }

    pub fn entries(&self) -> &[Candidate] {
        &self.entries
    }

    pub fn progress(&self) -> Progress {
        self.progress
    }

    /// Takes what the listing has sent so far, without waiting. Returns
    /// whether anything changed.
    pub fn receive(&mut self) -> bool {
        let mut changed = false;
        while let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(message) => {
                    self.take(message);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => self.lost(),
            }
        }
        changed
    }

    /// Waits for the listing to end.
    pub fn wait(&mut self) {
        while let Some(receiver) = &self.receiver {
            match receiver.recv() {
                Ok(message) => self.take(message),
                Err(_) => self.lost(),
            }
        }
    }

    fn take(&mut self, message: Message) {
        match message {
            Message::Entries(entries) => {
                self.entries.extend(entries.into_iter().map(Candidate::new))
            }
            Message::Done(walked) => {
                self.progress = match walked {
                    Walked::Complete => Progress::Complete,
                    Walked::Partial | Walked::Stopped => Progress::Partial,
                };
                self.receiver = None;
            }
        }
    }

    /// The thread ended without saying how, which only a panic does: what it
    /// sent is kept, and shown as partial.
    fn lost(&mut self) {
        self.progress = Progress::Partial;
        self.receiver = None;
    }
}

/// The candidates `query` matches, best first, as indices into `candidates`.
///
/// The query's characters must appear in the path in order, ignoring case;
/// the fewer characters between them, the better. A match within the file
/// name comes before one that needs the directories, so `app` finds
/// `src/app.rs` before `src/pager/mod.rs`. Ties go by path.
pub fn rank(candidates: &[Candidate], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    let mut matches: Vec<((bool, usize), usize)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            let score = match fuzzy_score(&candidate.lower[candidate.name..], &query) {
                Some(score) => (false, score),
                None => (true, fuzzy_score(&candidate.lower, &query)?),
            };
            Some((score, index))
        })
        .collect();
    matches.sort_by(|(a, i), (b, j)| {
        a.cmp(b)
            .then_with(|| candidates[*i].entry.path.cmp(&candidates[*j].entry.path))
    });
    matches.into_iter().map(|(_, index)| index).collect()
}

/// The characters skipped between the query's, each matched as early as it
/// can be (docsail's `fuzzy_score`).
fn fuzzy_score(value: &str, query: &str) -> Option<usize> {
    let mut offset = 0;
    let mut score = 0;
    for character in query.chars() {
        let found = value[offset..].find(character)?;
        score += found;
        offset += found + character.len_utf8();
    }
    Some(score)
}

/// The open prompt: what has been typed and which match is selected.
#[derive(Debug, Default)]
pub struct Finder {
    pub query: String,
    pub matches: Vec<usize>,
    pub selected: usize,
}

impl Finder {
    /// Ranks the candidates again. The selection stays on the same file
    /// while more of them arrive, and goes back to the best match when the
    /// query changes.
    pub fn refresh(&mut self, candidates: &[Candidate], query_changed: bool) {
        let kept = (!query_changed)
            .then(|| self.matches.get(self.selected).copied())
            .flatten();
        self.matches = rank(candidates, &self.query);
        self.selected = kept
            .and_then(|index| self.matches.iter().position(|&m| m == index))
            .unwrap_or(0);
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.matches.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    pub fn chosen<'a>(&self, candidates: &'a [Candidate]) -> Option<&'a Entry> {
        let index = *self.matches.get(self.selected)?;
        candidates.get(index).map(|candidate| &candidate.entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replor_render::snapshot::{EntryKind, SnapshotPath};

    fn candidates(paths: &[&str]) -> Vec<Candidate> {
        paths
            .iter()
            .map(|path| {
                let path = SnapshotPath::new(path).unwrap();
                Candidate::new(Entry {
                    name: path.name().unwrap().to_string(),
                    path,
                    kind: EntryKind::File,
                })
            })
            .collect()
    }

    fn ranked(paths: &[&str], query: &str) -> Vec<String> {
        let candidates = candidates(paths);
        rank(&candidates, query)
            .into_iter()
            .map(|index| candidates[index].entry.path.to_string())
            .collect()
    }

    #[test]
    fn the_query_keeps_paths_holding_its_characters_in_order() {
        let paths = [
            "docs/design/p1-core.md",
            "docs/design/concept.md",
            "README.md",
        ];
        assert_eq!(ranked(&paths, "p1co"), ["docs/design/p1-core.md"]);
        assert_eq!(ranked(&paths, "oc1p"), Vec::<String>::new());
        // Case is ignored on both sides.
        assert_eq!(ranked(&paths, "readme"), ["README.md"]);
        assert_eq!(ranked(&paths, "P1-CORE"), ["docs/design/p1-core.md"]);
    }

    #[test]
    fn a_closer_match_comes_first() {
        let paths = ["a/xpxrxs.md", "a/parse.md", "a/prs.md"];
        assert_eq!(
            ranked(&paths, "prs"),
            ["a/prs.md", "a/parse.md", "a/xpxrxs.md"]
        );
    }

    #[test]
    fn a_match_in_the_file_name_comes_before_one_through_directories() {
        let paths = ["app/src/mod.rs", "src/pager/app.rs", "src/app.rs"];
        // `src/app.rs` and `src/pager/app.rs` match within the name equally
        // well, so the path decides between them.
        assert_eq!(
            ranked(&paths, "app"),
            ["src/app.rs", "src/pager/app.rs", "app/src/mod.rs"]
        );
        assert_eq!(ranked(&paths, "pager/app"), ["src/pager/app.rs"]);
    }

    #[test]
    fn an_empty_query_lists_every_path_in_order() {
        let paths = ["b.md", "a/c.md", "a.md"];
        assert_eq!(ranked(&paths, ""), ["a.md", "a/c.md", "b.md"]);
    }

    #[test]
    fn the_selection_stays_on_its_file_while_candidates_arrive() {
        let mut all = candidates(&["b.md", "c.md"]);
        let mut finder = Finder::default();
        finder.refresh(&all, true);
        finder.move_by(1);
        assert_eq!(finder.chosen(&all).unwrap().path.as_str(), "c.md");
        all.extend(candidates(&["a.md"]));
        finder.refresh(&all, false);
        assert_eq!(finder.chosen(&all).unwrap().path.as_str(), "c.md");
        finder.query.push('b');
        finder.refresh(&all, true);
        assert_eq!(finder.chosen(&all).unwrap().path.as_str(), "b.md");
        finder.move_by(5);
        assert_eq!(finder.selected, 0);
    }
}
