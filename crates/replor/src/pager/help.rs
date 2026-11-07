//! The key list shown by the `?` overlay.
//!
//! This table is the source of truth for the bindings: the overlay must stand
//! on its own, and a test checks that every key it lists does something in the
//! pane it is listed for.

/// Where a binding applies. The tree follows docsail and the preview follows
/// mdvu's pager, so neither set of habits has to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Tree,
    Preview,
    Both,
}

/// One row of the overlay: the keys that trigger an action, and what it does.
pub struct Entry {
    /// Individual key names, joined for display and parsed by the test that
    /// checks every advertised key is actually bound.
    pub keys: &'static [&'static str],
    pub action: &'static str,
    pub pane: Pane,
}

pub const ENTRIES: &[Entry] = &[
    Entry {
        keys: &["Tab"],
        action: "Switch between the tree and the preview",
        pane: Pane::Both,
    },
    Entry {
        keys: &["j", "Down"],
        action: "Tree: next entry",
        pane: Pane::Tree,
    },
    Entry {
        keys: &["k", "Up"],
        action: "Tree: previous entry",
        pane: Pane::Tree,
    },
    Entry {
        keys: &["Enter"],
        action: "Tree: open a file, expand or collapse a directory",
        pane: Pane::Tree,
    },
    Entry {
        keys: &["j", "Down"],
        action: "Down one line",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["k", "Up"],
        action: "Up one line",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["Ctrl-d", "Ctrl-u"],
        action: "Half screen down / up",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["Space", "PageDown"],
        action: "One screen down",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["b", "PageUp"],
        action: "One screen up",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["g", "Home"],
        action: "Top of the document",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["G", "End"],
        action: "End of the document",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["h", "Left"],
        action: "Scroll left",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["l", "Right"],
        action: "Scroll right",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["0"],
        action: "Reset horizontal scroll",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["/"],
        action: "Search, Enter to confirm, Esc to cancel",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["n", "N"],
        action: "Next / previous match",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["t"],
        action: "Heading list",
        pane: Pane::Preview,
    },
    Entry {
        keys: &["r"],
        action: "Reload the tree and the open file",
        pane: Pane::Both,
    },
    Entry {
        keys: &["?"],
        action: "This help",
        pane: Pane::Both,
    },
    Entry {
        keys: &["q", "Esc"],
        action: "Quit",
        pane: Pane::Both,
    },
];

/// Display width of the key column, so the actions line up.
pub fn key_column() -> usize {
    ENTRIES
        .iter()
        .map(|entry| entry.keys.join(", ").chars().count())
        .max()
        .unwrap_or(0)
}

/// One row as `keys` padded to the key column, then the action.
pub fn row(entry: &Entry, column: usize) -> String {
    format!("{:<column$}  {}", entry.keys.join(", "), entry.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_line_up_under_one_key_column() {
        let column = key_column();
        for entry in ENTRIES {
            let row = row(entry, column);
            assert_eq!(&row[column + 2..], entry.action, "{row}");
        }
    }
}
