//! Nerd Font glyphs.
//!
//! This is the only module that names a glyph codepoint, in the same way
//! `merman` is confined to `diagram::mermaid`. Everything else asks an
//! `IconSet` for a prefix and concatenates it.
//!
//! A glyph is always followed by a space, so the prefix occupies two display
//! columns. Nerd Font codepoints live in the Private Use Area, which is East
//! Asian Width Ambiguous and therefore one column wide by `display_width`;
//! terminals that draw them two columns wide are off by one instead of by two,
//! and the space keeps the glyph from touching the label.

use clap::ValueEnum;

use crate::markdown::model::AlertKind;

/// Which glyphs the rendered surface uses.
///
/// `Unicode` produces an empty prefix everywhere, so the default output is
/// byte-for-byte what it was before icons existed. `nerd` is opt-in and never
/// detected: whether a Nerd Font is installed is a property of the terminal's
/// font, and asking the terminal would mean writing to the tty and waiting for
/// an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum IconSet {
    #[default]
    Unicode,
    Nerd,
}

impl IconSet {
    /// The label of a GFM alert, prefixed when the set has a glyph for it.
    pub fn alert(self, kind: AlertKind) -> String {
        self.slot(match kind {
            AlertKind::Note => '\u{f02fd}',
            AlertKind::Tip => '\u{f0336}',
            AlertKind::Important => '\u{f017e}',
            AlertKind::Warning => '\u{f002a}',
            AlertKind::Caution => '\u{f0ce6}',
        })
    }

    /// The language of a fenced code block, by its info string token.
    pub fn language(self, token: &str) -> String {
        self.slot(lookup(LANGUAGES, token).unwrap_or(GENERIC_CODE))
    }

    /// An image destination. The kind is what the glyph carries, so the
    /// extension is not consulted: every image gets the same picture glyph.
    pub fn image(self) -> String {
        self.slot(IMAGE)
    }

    /// A non-image destination, by its extension.
    pub fn attachment(self, dest: &str) -> String {
        self.slot(
            extension(dest)
                .and_then(|e| lookup(ATTACHMENTS, e))
                .unwrap_or(ATTACHMENT),
        )
    }

    /// A glyph and the space that pads it to two display columns, or nothing.
    fn slot(self, glyph: char) -> String {
        match self {
            IconSet::Unicode => String::new(),
            IconSet::Nerd => format!("{glyph} "),
        }
    }
}

/// Generic glyphs, used when the table has no entry.
const GENERIC_CODE: char = '\u{f022e}';
const IMAGE: char = '\u{f02e9}';
const ATTACHMENT: char = '\u{f03e2}';

/// Info string tokens to glyphs. Aliases sit next to the name they stand for,
/// so a fence written `rs` and one written `rust` look the same.
const LANGUAGES: &[(&str, char)] = &[
    ("rust", '\u{f1617}'),
    ("rs", '\u{f1617}'),
    ("python", '\u{f0320}'),
    ("py", '\u{f0320}'),
    ("go", '\u{f07d3}'),
    ("c", '\u{f0671}'),
    ("cpp", '\u{f0672}'),
    ("c++", '\u{f0672}'),
    ("csharp", '\u{f031b}'),
    ("cs", '\u{f031b}'),
    ("java", '\u{f0b37}'),
    ("javascript", '\u{f031e}'),
    ("js", '\u{f031e}'),
    ("jsx", '\u{f031e}'),
    ("typescript", '\u{f06e6}'),
    ("ts", '\u{f06e6}'),
    ("tsx", '\u{f06e6}'),
    ("ruby", '\u{f0d2d}'),
    ("rb", '\u{f0d2d}'),
    ("php", '\u{f031f}'),
    ("swift", '\u{f06e5}'),
    ("kotlin", '\u{f1219}'),
    ("kt", '\u{f1219}'),
    ("haskell", '\u{f0c92}'),
    ("hs", '\u{f0c92}'),
    ("lua", '\u{f08b1}'),
    ("r", '\u{f07d4}'),
    ("nix", '\u{f1105}'),
    ("html", '\u{f031d}'),
    ("css", '\u{f031c}'),
    ("scss", '\u{f031c}'),
    ("sass", '\u{f031c}'),
    ("less", '\u{f031c}'),
    ("markdown", '\u{f0354}'),
    ("md", '\u{f0354}'),
    ("xml", '\u{f05c0}'),
    ("json", '\u{f0626}'),
    ("yaml", '\u{f0493}'),
    ("yml", '\u{f0493}'),
    ("toml", '\u{f0493}'),
    ("ini", '\u{f0493}'),
    ("conf", '\u{f0493}'),
    ("sh", '\u{f018d}'),
    ("bash", '\u{f018d}'),
    ("zsh", '\u{f018d}'),
    ("fish", '\u{f018d}'),
    ("shell", '\u{f018d}'),
    ("console", '\u{f018d}'),
    ("sql", '\u{f01bc}'),
    ("dockerfile", '\u{f0868}'),
    ("docker", '\u{f0868}'),
    ("diff", '\u{f02a2}'),
    ("patch", '\u{f02a2}'),
];

/// Attachment extensions to glyphs. Only the kinds a reader can act on differ;
/// everything else is a paperclip.
const ATTACHMENTS: &[(&str, char)] = &[
    ("pdf", '\u{f0226}'),
    ("zip", '\u{f05c4}'),
    ("gz", '\u{f05c4}'),
    ("tar", '\u{f05c4}'),
];

fn lookup(table: &[(&str, char)], key: &str) -> Option<char> {
    let key = key.to_ascii_lowercase();
    table
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, glyph)| *glyph)
}

/// The extension of a destination, without reading or resolving it.
fn extension(dest: &str) -> Option<&str> {
    let name = dest.rsplit(['/', '\\']).next()?;
    let name = name.split(['?', '#']).next()?;
    name.rsplit_once('.').map(|(_, ext)| ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::wrap::display_width;

    /// The two-column slot assumes every glyph measures one column. A glyph
    /// that measures two would push the label out of the space reserved for it.
    #[test]
    fn every_glyph_measures_one_column() {
        let glyphs = LANGUAGES
            .iter()
            .chain(ATTACHMENTS)
            .map(|(_, glyph)| *glyph)
            .chain([GENERIC_CODE, IMAGE, ATTACHMENT])
            .chain(
                [
                    AlertKind::Note,
                    AlertKind::Tip,
                    AlertKind::Important,
                    AlertKind::Warning,
                    AlertKind::Caution,
                ]
                .iter()
                .map(|kind| {
                    IconSet::Nerd
                        .alert(*kind)
                        .chars()
                        .next()
                        .expect("a glyph and its space")
                }),
            );
        for glyph in glyphs {
            assert_eq!(
                display_width(&glyph.to_string()),
                1,
                "U+{:X} must be one column wide",
                glyph as u32
            );
        }
    }

    #[test]
    fn a_prefix_is_two_columns_wide() {
        assert_eq!(display_width(&IconSet::Nerd.alert(AlertKind::Note)), 2);
        assert_eq!(display_width(&IconSet::Nerd.language("rust")), 2);
        assert_eq!(display_width(&IconSet::Nerd.image()), 2);
    }

    /// The default set adds nothing at all, which is what keeps the rendered
    /// surface identical to what it was before icons existed.
    #[test]
    fn the_unicode_set_has_no_prefix() {
        assert_eq!(IconSet::default(), IconSet::Unicode);
        assert!(IconSet::Unicode.alert(AlertKind::Warning).is_empty());
        assert!(IconSet::Unicode.language("rust").is_empty());
        assert!(IconSet::Unicode.image().is_empty());
        assert!(IconSet::Unicode.attachment("design.pdf").is_empty());
    }

    #[test]
    fn an_alias_matches_the_language_it_stands_for() {
        assert_eq!(IconSet::Nerd.language("rs"), IconSet::Nerd.language("rust"));
        assert_eq!(
            IconSet::Nerd.language("YAML"),
            IconSet::Nerd.language("yml")
        );
    }

    #[test]
    fn an_unknown_language_falls_back_to_a_generic_glyph() {
        assert_eq!(
            IconSet::Nerd.language("brainfuck"),
            format!("{GENERIC_CODE} ")
        );
        assert_eq!(IconSet::Nerd.language(""), format!("{GENERIC_CODE} "));
    }

    #[test]
    fn an_attachment_is_recognised_by_its_extension() {
        assert_eq!(
            IconSet::Nerd.attachment("docs/report.PDF"),
            format!("{} ", '\u{f0226}')
        );
        assert_eq!(
            IconSet::Nerd.attachment(".attachments/design.xlsx"),
            format!("{ATTACHMENT} ")
        );
        assert_eq!(
            IconSet::Nerd.attachment("no-extension"),
            format!("{ATTACHMENT} ")
        );
    }
}
