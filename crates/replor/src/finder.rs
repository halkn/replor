//! Finding a file by its path: the `Ctrl-p` prompt of the Files lens.
//!
//! The candidates are listed on a thread of their own, through the Snapshot,
//! and collected by the frame loop between key presses, so a large
//! repository can be read and searched while the listing goes on.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use replor_render::snapshot::{Entry, Snapshot, WalkLimits, Walked, walk};
use unicode_segmentation::UnicodeSegmentation;

/// replor-go's bounds (`docs/plans/initial/DESIGN.md`, 6.2).
pub const LIMITS: WalkLimits = WalkLimits {
    entries: 200_000,
    depth: 64,
};

/// Entries sent at once, so the frame loop is not woken for every file.
const BATCH: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Listing,
    Complete,
    /// A limit was reached, so some files are not candidates.
    Partial,
    /// The root could not be listed, for this reason.
    Failed(String),
}

enum Message {
    Entries(Vec<Candidate>),
    Done(Result<Walked, String>),
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
                // Lowercased here, so the frame loop only appends.
                batch.push(Candidate::new(entry));
                if batch.len() < BATCH {
                    return true;
                }
                sender
                    .send(Message::Entries(std::mem::take(&mut batch)))
                    .is_ok()
            });
            // Nobody receives these once the walk was stopped.
            let _ = sender.send(Message::Entries(batch));
            let _ = sender.send(Message::Done(walked.map_err(|err| err.to_string())));
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

    pub fn progress(&self) -> &Progress {
        &self.progress
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
            Message::Entries(entries) => self.entries.extend(entries),
            Message::Done(walked) => {
                self.progress = match walked {
                    Ok(Walked::Complete) => Progress::Complete,
                    Ok(_) => Progress::Partial,
                    Err(reason) => Progress::Failed(reason),
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
    // No two keys are equal, as the path tells them apart.
    matches.sort_unstable_by_key(|&(score, index)| (score, &candidates[index].entry.path));
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

/// What a key does while the finder is open, which is every key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinderInput {
    Type(char),
    Erase,
    Move(isize),
    Open,
    Close,
    Quit,
    Ignored,
}

pub fn map(key: KeyEvent) -> FinderInput {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('c') if control => FinderInput::Quit,
        KeyCode::Char('p') if control => FinderInput::Move(-1),
        KeyCode::Char('n') if control => FinderInput::Move(1),
        KeyCode::Char(c) if !control && !key.modifiers.contains(KeyModifiers::ALT) => {
            FinderInput::Type(c)
        }
        KeyCode::Up => FinderInput::Move(-1),
        KeyCode::Down => FinderInput::Move(1),
        KeyCode::Backspace => FinderInput::Erase,
        KeyCode::Enter => FinderInput::Open,
        KeyCode::Esc => FinderInput::Close,
        _ => FinderInput::Ignored,
    }
}

/// The open prompt: what has been typed and which match is selected.
#[derive(Debug, Default)]
pub struct Finder {
    query: String,
    matches: Vec<usize>,
    selected: usize,
}

impl Finder {
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Indices into the candidates, best first.
    pub fn matches(&self) -> &[usize] {
        &self.matches
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn push(&mut self, c: char) {
        self.query.push(c);
    }

    /// Removes the last grapheme, so a combining mark goes with its letter.
    /// Returns whether there was one.
    pub fn erase(&mut self) -> bool {
        match self.query.grapheme_indices(true).next_back() {
            Some((at, _)) => {
                self.query.truncate(at);
                true
            }
            None => false,
        }
    }

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
    fn every_key_is_the_finder_s_while_it_is_open() {
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        // Letters that quit or reload elsewhere are text here.
        assert_eq!(map(key(KeyCode::Char('q'))), FinderInput::Type('q'));
        assert_eq!(map(key(KeyCode::Char('r'))), FinderInput::Type('r'));
        assert_eq!(map(ctrl('p')), FinderInput::Move(-1));
        assert_eq!(map(ctrl('n')), FinderInput::Move(1));
        assert_eq!(map(key(KeyCode::Up)), FinderInput::Move(-1));
        assert_eq!(map(key(KeyCode::Down)), FinderInput::Move(1));
        assert_eq!(map(key(KeyCode::Backspace)), FinderInput::Erase);
        assert_eq!(map(key(KeyCode::Enter)), FinderInput::Open);
        assert_eq!(map(key(KeyCode::Esc)), FinderInput::Close);
        assert_eq!(map(ctrl('c')), FinderInput::Quit);
        assert_eq!(map(ctrl('x')), FinderInput::Ignored);
    }

    #[test]
    fn erase_takes_a_whole_grapheme() {
        let mut finder = Finder::default();
        for c in "ae\u{301}".chars() {
            finder.push(c);
        }
        assert!(finder.erase());
        assert_eq!(finder.query(), "a");
        assert!(finder.erase());
        assert!(!finder.erase());
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
        finder.push('b');
        finder.refresh(&all, true);
        assert_eq!(finder.chosen(&all).unwrap().path.as_str(), "b.md");
        finder.move_by(5);
        assert_eq!(finder.selected(), 0);
    }
}
