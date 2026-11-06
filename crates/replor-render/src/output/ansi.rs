use std::io::Write;

use crate::layout::RenderedDocument;
use crate::layout::theme::{Color, Style, Theme};
use crate::output::trimmed;

const RESET: &str = "\x1b[0m";
/// OSC 8 introducer and terminator. ST (`ESC \`) is used rather than BEL
/// because more terminals accept it inside multiplexers.
const LINK_OPEN: &str = "\x1b]8;;";
const ST: &str = "\x1b\\";

pub fn write_document(
    out: &mut impl Write,
    document: &RenderedDocument,
    theme: &Theme,
    hyperlinks: bool,
) -> std::io::Result<()> {
    for line in &document.lines {
        // An image starts where the line's own text ends, so an indent or a
        // quote border still lines up. Trailing spaces are part of that indent
        // here and are kept.
        let spans = match line.image {
            Some(_) => line.spans.clone(),
            None => trimmed(line),
        };
        for span in spans {
            let link = span.link.as_deref().filter(|_| hyperlinks);
            if let Some(url) = link {
                write!(out, "{LINK_OPEN}{url}{ST}")?;
            }
            let mut style = theme.style(span.role);
            // Underline marks a real URL, which is narrower than what is styled
            // as a link: a relative path, a work item and a mention all look
            // like links but are text. It does not depend on `hyperlinks`, so
            // the mark means the same thing in every mode.
            if span.link.is_some() {
                style.underline = true;
            }
            match sgr(style) {
                Some(prefix) => write!(out, "{prefix}{}{RESET}", span.text)?,
                None => out.write_all(span.text.as_bytes())?,
            }
            if link.is_some() {
                write!(out, "{LINK_OPEN}{ST}")?;
            }
        }
        // The image is drawn at the cursor without moving it, and the blank
        // lines the layout reserved below scroll it into view like any other
        // content.
        if let Some(image) = &line.image {
            out.write_all(image.escape().as_bytes())?;
        }
        out.write_all(b"\n")?;
    }
    out.flush()
}

/// SGR introducer for a style, or `None` when the style adds nothing.
pub fn sgr(style: Style) -> Option<String> {
    let mut codes: Vec<u8> = Vec::new();
    if style.bold {
        codes.push(1);
    }
    if style.dim {
        codes.push(2);
    }
    if style.italic {
        codes.push(3);
    }
    if style.underline {
        codes.push(4);
    }
    if style.reverse {
        codes.push(7);
    }
    if style.strikethrough {
        codes.push(9);
    }
    if let Some(fg) = style.fg {
        codes.push(foreground(fg));
    }
    if let Some(bg) = style.bg {
        codes.push(foreground(bg) + 10);
    }
    if codes.is_empty() {
        return None;
    }
    let body = codes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(";");
    Some(format!("\x1b[{body}m"))
}

fn foreground(color: Color) -> u8 {
    match color {
        Color::Red => 31,
        Color::Green => 32,
        Color::Yellow => 33,
        Color::Blue => 34,
        Color::Magenta => 35,
        Color::BrightBlack => 90,
        Color::BrightGreen => 92,
        Color::BrightYellow => 93,
        Color::BrightBlue => 94,
        Color::BrightCyan => 96,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::theme::Variant;
    use crate::layout::{RenderedSpan, StyleRole};

    #[test]
    fn plain_styles_emit_nothing() {
        assert_eq!(sgr(Style::default()), None);
    }

    fn document(span: RenderedSpan) -> RenderedDocument {
        RenderedDocument {
            lines: vec![crate::layout::RenderedLine {
                spans: vec![span],
                ..Default::default()
            }],
            diagnostics: Vec::new(),
        }
    }

    fn render(span: RenderedSpan, hyperlinks: bool) -> String {
        let mut out: Vec<u8> = Vec::new();
        write_document(
            &mut out,
            &document(span),
            &Theme::new(Variant::Dark),
            hyperlinks,
        )
        .expect("writing to a vector cannot fail");
        String::from_utf8(out).expect("output is utf-8")
    }

    #[test]
    fn a_linked_span_is_wrapped_in_osc_8() {
        let span = RenderedSpan::new("docs", StyleRole::Link).with_link("https://example.com");
        let out = render(span, true);
        assert!(out.starts_with("\x1b]8;;https://example.com\x1b\\"));
        assert!(out.trim_end().ends_with("\x1b]8;;\x1b\\"));
        assert!(out.contains("docs"));
    }

    #[test]
    fn hyperlinks_disabled_leaves_the_text_untouched() {
        let span = RenderedSpan::new("docs", StyleRole::Link).with_link("https://example.com");
        assert!(!render(span, false).contains("]8;;"));
    }

    /// Underline says "the destination is a URL", so it marks the same runs
    /// whether or not they were emitted as hyperlinks.
    #[test]
    fn only_a_url_destination_is_underlined() {
        let underlined = |text: &str| text.contains("\x1b[4") || text.contains(";4m");

        let url = RenderedSpan::new("docs", StyleRole::Link).with_link("https://example.com");
        assert!(underlined(&render(url.clone(), true)));
        assert!(underlined(&render(url, false)));

        // A link-styled run with no URL behind it is colour only.
        assert!(!underlined(&render(
            RenderedSpan::new("./other.md", StyleRole::Link),
            true
        )));
    }

    #[test]
    fn attributes_and_colors_combine() {
        let style = Theme::new(Variant::Dark).style(StyleRole::Heading(1));
        let sequence = sgr(style).expect("heading should be styled");
        assert!(sequence.starts_with("\x1b["));
        assert!(sequence.ends_with('m'));
        assert!(sequence.contains('1'));
    }
}
