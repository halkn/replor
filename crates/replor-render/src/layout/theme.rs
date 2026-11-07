//! Backend-neutral styling for semantic roles.

use clap::ValueEnum;

use crate::layout::{StyleRole, SyntaxKind};
use crate::markdown::model::AlertKind;

/// The theme the reader asked for; `Variant` is what it resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ThemeChoice {
    Auto,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,

    BrightBlack,

    BrightGreen,
    BrightYellow,
    BrightBlue,

    BrightCyan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
    pub strikethrough: bool,
    pub reverse: bool,
}

impl Style {
    const fn fg(color: Color) -> Self {
        Self {
            fg: Some(color),
            ..Self::plain()
        }
    }

    const fn plain() -> Self {
        Self {
            fg: None,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
            dim: false,
            strikethrough: false,
            reverse: false,
        }
    }

    const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    const fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    const fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    const fn reverse(mut self) -> Self {
        self.reverse = true;
        self
    }

    #[cfg(test)]
    pub fn is_plain(&self) -> bool {
        *self == Self::plain()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Dark,
    Light,
}

impl Variant {
    /// Theme `auto` never issues a blocking terminal query. It uses the
    /// conventional `COLORFGBG` hint when present and falls back to dark.
    pub fn resolve(option: ThemeChoice) -> Self {
        match option {
            ThemeChoice::Dark => Variant::Dark,
            ThemeChoice::Light => Variant::Light,
            ThemeChoice::Auto => Self::from_env().unwrap_or(Variant::Dark),
        }
    }

    fn from_env() -> Option<Self> {
        // COLORFGBG is "fg;bg"; a low background index means a dark terminal.
        let value = std::env::var("COLORFGBG").ok()?;
        let bg = value.rsplit(';').next()?.trim().parse::<u8>().ok()?;
        Some(if (7..=15).contains(&bg) {
            Variant::Light
        } else {
            Variant::Dark
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub variant: Variant,
    color: bool,
}

impl Theme {
    pub fn new(variant: Variant) -> Self {
        Self {
            variant,
            color: true,
        }
    }

    pub fn with_color(mut self, choice: crate::output::ColorChoice) -> Self {
        self.color = choice == crate::output::ColorChoice::Ansi;
        self
    }

    pub fn style(&self, role: StyleRole) -> Style {
        let mut style = self.colored_style(role);
        if !self.color {
            style.fg = None;
            style.bg = None;
        }
        style
    }

    fn colored_style(&self, role: StyleRole) -> Style {
        let accent = match self.variant {
            Variant::Dark => Color::BrightCyan,
            Variant::Light => Color::Blue,
        };
        let subtle = match self.variant {
            Variant::Dark => Color::BrightBlack,
            Variant::Light => Color::BrightBlack,
        };
        match role {
            StyleRole::Normal => Style::plain(),
            StyleRole::Muted => Style::fg(subtle).dim(),
            // Six levels cannot all be told apart in 16 colours, so the styling
            // carries three tiers and the `#` count carries the exact level.
            StyleRole::Heading(1) => Style::fg(accent).bold().underline(),
            StyleRole::Heading(2) => Style::fg(accent).bold(),
            StyleRole::Heading(3) => Style::fg(accent),
            StyleRole::Heading(_) => Style::fg(accent).dim(),
            StyleRole::Strong => Style::plain().bold(),
            StyleRole::Emphasis => Style::plain().italic(),
            StyleRole::Strike => Style::plain().strikethrough(),
            StyleRole::InlineCode | StyleRole::Code => Style::fg(match self.variant {
                Variant::Dark => Color::BrightYellow,
                Variant::Light => Color::Magenta,
            }),
            StyleRole::CodeBorder | StyleRole::TableBorder | StyleRole::DiagramBorder => {
                Style::fg(subtle)
            }
            // Colour only. The underline is reserved for runs a backend has
            // actually made clickable, so it means one thing everywhere.
            StyleRole::Link => Style::fg(match self.variant {
                Variant::Dark => Color::BrightBlue,
                Variant::Light => Color::Blue,
            }),
            StyleRole::LinkTarget => Style::fg(subtle).dim(),
            StyleRole::Quote => Style::fg(match self.variant {
                Variant::Dark => Color::BrightGreen,
                Variant::Light => Color::Green,
            }),
            StyleRole::Alert(kind) => Style::fg(self.alert(kind)),
            StyleRole::ListMarker => Style::fg(accent),
            StyleRole::TaskChecked => Style::fg(Color::Green),
            StyleRole::TaskUnchecked => Style::fg(subtle),
            StyleRole::Diagram => Style::plain(),
            StyleRole::Warning => Style::fg(Color::Yellow),
            StyleRole::Error => Style::fg(Color::Red),
            StyleRole::SearchMatch => Style::plain().reverse(),
            // Reversed as well, so a match reads the same way wherever it is,
            // but on magenta: the plain reverse is the terminal's own
            // foreground, and yellow sits too close to it to be told apart at a
            // glance.
            StyleRole::CurrentMatch => Style::fg(Color::Magenta).reverse().bold(),
            StyleRole::InitialLine => Style::fg(Color::Yellow).dim(),
            StyleRole::Status => Style::plain().reverse(),
            StyleRole::Syntax(kind) => self.syntax(kind),
        }
    }

    /// Alert colours follow the severity the kind names, and stay inside the
    /// same 16-colour palette as everything else. The bright variants only
    /// appear on dark, where the plain ones are too dim to read as chrome.
    fn alert(&self, kind: AlertKind) -> Color {
        let dark = self.variant == Variant::Dark;
        match kind {
            AlertKind::Note => {
                if dark {
                    Color::BrightBlue
                } else {
                    Color::Blue
                }
            }
            AlertKind::Tip => {
                if dark {
                    Color::BrightGreen
                } else {
                    Color::Green
                }
            }
            AlertKind::Important => Color::Magenta,
            AlertKind::Warning => Color::Yellow,
            AlertKind::Caution => Color::Red,
        }
    }

    /// Token colours stay inside the same 16-colour palette as everything else,
    /// and stay clear of the border and link colours so a code block still
    /// reads as one region.
    fn syntax(&self, kind: SyntaxKind) -> Style {
        let dark = self.variant == Variant::Dark;
        match kind {
            SyntaxKind::Keyword => Style::fg(Color::Magenta),
            SyntaxKind::String => Style::fg(if dark {
                Color::BrightGreen
            } else {
                Color::Green
            }),
            SyntaxKind::Number => Style::fg(Color::Yellow),
            SyntaxKind::Comment => Style::fg(Color::BrightBlack).dim(),
            SyntaxKind::Type => Style::fg(if dark { Color::BrightCyan } else { Color::Blue }),
            SyntaxKind::Function => Style::fg(if dark { Color::BrightBlue } else { Color::Blue }),
            // Punctuation is only nudged away from plain text, never coloured
            // strongly enough to compete with the tokens around it.
            SyntaxKind::Punctuation => Style::fg(Color::BrightBlack),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_theme_options_win() {
        assert_eq!(Variant::resolve(ThemeChoice::Dark), Variant::Dark);
        assert_eq!(Variant::resolve(ThemeChoice::Light), Variant::Light);
    }

    #[test]
    fn normal_text_has_no_styling() {
        assert!(
            Theme::new(Variant::Dark)
                .style(StyleRole::Normal)
                .is_plain()
        );
    }

    #[test]
    fn heading_levels_are_distinguishable() {
        let t = Theme::new(Variant::Dark);
        // Three tiers of styling; the `#` count carries the exact level.
        for level in 1..4 {
            assert_ne!(
                t.style(StyleRole::Heading(level)),
                t.style(StyleRole::Heading(level + 1)),
                "levels {level} and {}",
                level + 1
            );
        }
    }

    /// Five kinds, five colours: an alert must never be mistaken for another.
    #[test]
    fn alert_kinds_have_distinct_colours() {
        let kinds = [
            AlertKind::Note,
            AlertKind::Tip,
            AlertKind::Important,
            AlertKind::Warning,
            AlertKind::Caution,
        ];
        for variant in [Variant::Dark, Variant::Light] {
            let theme = Theme::new(variant);
            let mut seen = Vec::new();
            for kind in kinds {
                let style = theme.style(StyleRole::Alert(kind));
                assert!(!style.is_plain(), "{kind:?} in {variant:?}");
                assert!(!seen.contains(&style), "{kind:?} repeats a colour");
                seen.push(style);
            }
        }
    }

    #[test]
    fn both_variants_style_every_role() {
        let roles = [
            StyleRole::Muted,
            StyleRole::Strong,
            StyleRole::Emphasis,
            StyleRole::Strike,
            StyleRole::InlineCode,
            StyleRole::Link,
            StyleRole::Quote,
            StyleRole::Alert(AlertKind::Note),
            StyleRole::ListMarker,
            StyleRole::Warning,
            StyleRole::Error,
        ];
        for variant in [Variant::Dark, Variant::Light] {
            let theme = Theme::new(variant);
            for role in roles {
                assert!(!theme.style(role).is_plain(), "{role:?} in {variant:?}");
            }
        }
    }
}
