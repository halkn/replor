//! Inline IR to styled spans.
//!
//! Hard breaks split the result into segments; the caller wraps each segment
//! independently so an explicit break is never merged away.

use std::path::{Path, PathBuf};

use crate::image::has_scheme;
use crate::layout::icons::IconSet;
use crate::layout::{RenderedSpan, StyleRole};
use crate::markdown::model::{Inline, plain_text};

#[derive(Debug, Clone, Default)]
pub struct InlineContext {
    /// Directory of the input file, used to display relative link targets and
    /// to join document-relative image paths. Link targets are never opened.
    pub base_dir: Option<PathBuf>,
    /// The only directory images may be read from, when it is wider than
    /// `base_dir`. See `image::content_root`.
    pub content_root: Option<PathBuf>,
    /// How diagram blocks are presented. Diagrams are rendered before layout;
    /// this only selects between the rendered form, the source and an omitted
    /// marker.
    pub mermaid: crate::diagram::MermaidMode,
    /// Whether code blocks are split into syntax roles. Off for plain output,
    /// where every role would collapse to the same bytes anyway.
    pub highlight: bool,
    /// What the terminal can draw images with, when it can. `None` keeps every
    /// image a text placeholder.
    pub images: Option<crate::image::ImageSupport>,
    /// Which glyphs the chrome is drawn with. The default set adds nothing, so
    /// a reader without a Nerd Font sees exactly what they saw before.
    pub icons: IconSet,
}

/// Extensions rendered as an image placeholder rather than a generic attachment.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "ico", "avif", "tif", "tiff",
];

pub fn segments(inlines: &[Inline], ctx: &InlineContext) -> Vec<Vec<RenderedSpan>> {
    let mut out = vec![Vec::new()];
    push_inlines(inlines, StyleRole::Normal, ctx, &mut out);
    out.into_iter().map(coalesce).collect()
}

/// Convenience for contexts that cannot show a line break, such as a heading.
pub fn spans(inlines: &[Inline], ctx: &InlineContext) -> Vec<RenderedSpan> {
    let mut merged: Vec<RenderedSpan> = Vec::new();
    for (i, segment) in segments(inlines, ctx).into_iter().enumerate() {
        if i > 0 {
            merged.push(RenderedSpan::new(" ", StyleRole::Normal));
        }
        merged.extend(segment);
    }
    coalesce(merged)
}

fn push_inlines(
    inlines: &[Inline],
    role: StyleRole,
    ctx: &InlineContext,
    out: &mut Vec<Vec<RenderedSpan>>,
) {
    for inline in inlines {
        push_inline(inline, role, ctx, out);
    }
}

fn push_inline(
    inline: &Inline,
    role: StyleRole,
    ctx: &InlineContext,
    out: &mut Vec<Vec<RenderedSpan>>,
) {
    match inline {
        Inline::Text(text) => push(out, text, role),
        Inline::Code(code) => push(out, code, StyleRole::InlineCode),
        Inline::Strong(children) => push_inlines(children, StyleRole::Strong, ctx, out),
        Inline::Emphasis(children) => push_inlines(children, StyleRole::Emphasis, ctx, out),
        Inline::Strikethrough(children) => push_inlines(children, StyleRole::Strike, ctx, out),
        Inline::Link(link) => {
            let start = mark(out);
            // An empty label falls back to the destination, which then must not
            // be repeated as the target.
            let label = if link.content.is_empty() {
                push(out, &link.dest, StyleRole::Link);
                link.dest.clone()
            } else {
                push_inlines(&link.content, StyleRole::Link, ctx, out);
                plain_text(&link.content)
            };
            if let Some(url) = hyperlink(&link.dest) {
                attach_link(out, start, &url);
            }
            if let Some(target) = link_target(&link.dest, &label, ctx) {
                push(out, " (", StyleRole::LinkTarget);
                push(out, &target, StyleRole::LinkTarget);
                push(out, ")", StyleRole::LinkTarget);
            }
        }
        Inline::Image(image) => {
            let name = placeholder_label(&image.alt, &image.dest);
            // With a glyph the kind is legible without the word, so the bracket
            // form gives way to it; without one the word is all there is.
            let label = match (is_image_path(&image.dest), ctx.icons) {
                (true, IconSet::Unicode) => format!("[image: {name}]"),
                (false, IconSet::Unicode) => format!("[attachment: {name}]"),
                (true, icons) => format!("{}{name}", icons.image()),
                (false, icons) => format!("{}{name}", icons.attachment(&image.dest)),
            };
            push(out, &label, StyleRole::Muted);
            push(out, " (", StyleRole::LinkTarget);
            push(
                out,
                &display_target(&image.dest, ctx),
                StyleRole::LinkTarget,
            );
            push(out, ")", StyleRole::LinkTarget);
        }
        Inline::FootnoteRef(label) => push(out, &format!("[^{label}]"), StyleRole::Muted),
        Inline::SoftBreak => push(out, " ", role),
        Inline::HardBreak => out.push(Vec::new()),
        // Raw inline HTML is shown verbatim and never interpreted.
        Inline::RawHtml(html) => push(out, html, StyleRole::Muted),
        // Azure references are styled only; no lookup is performed.
        Inline::WorkItem(id) => push(out, &format!("#{id}"), StyleRole::Link),
        Inline::Mention(alias) => push(out, &format!("@{alias}"), StyleRole::Link),
        // Math is shown as its own source; KaTeX is never evaluated.
        Inline::Math(source) => push(out, source, StyleRole::InlineCode),
    }
}

fn push(out: &mut [Vec<RenderedSpan>], text: &str, role: StyleRole) {
    if text.is_empty() {
        return;
    }
    out.last_mut()
        .expect("at least one segment")
        .push(RenderedSpan::new(crate::text::spaced(text), role));
}

/// Position in the output, used to attach a destination to spans that are about
/// to be produced. A link label may span several segments when it contains a
/// hard break, so both coordinates are needed.
type Mark = (usize, usize);

fn mark(out: &[Vec<RenderedSpan>]) -> Mark {
    let segment = out.len().saturating_sub(1);
    (segment, out.get(segment).map_or(0, Vec::len))
}

/// Point every span produced since `start` at `url`.
fn attach_link(out: &mut [Vec<RenderedSpan>], start: Mark, url: &str) {
    let (first_segment, first_span) = start;
    for (index, segment) in out.iter_mut().enumerate().skip(first_segment) {
        let from = if index == first_segment {
            first_span
        } else {
            0
        };
        for span in segment.iter_mut().skip(from) {
            span.link = Some(url.to_string());
        }
    }
}

/// The URL to make clickable, or `None` when the destination must not become a
/// terminal hyperlink.
///
/// Only `http` and `https` qualify. Relative paths, fragments and other schemes
/// are display-only, matching the rule that `mdvu` never resolves or opens a
/// target. Control characters would break out of the escape sequence, and an
/// over-long destination is more likely to be malformed than useful.
fn hyperlink(dest: &str) -> Option<String> {
    const MAX_URL_LEN: usize = 2083;
    let lowered = dest.to_ascii_lowercase();
    if !(lowered.starts_with("http://") || lowered.starts_with("https://")) {
        return None;
    }
    if dest.len() > MAX_URL_LEN {
        return None;
    }
    if dest.chars().any(|c| c.is_control() || c == '\u{7f}') {
        return None;
    }
    Some(dest.to_string())
}

/// Merge neighbouring spans that share a role, so a line holds as few spans as
/// possible before wrapping and styling.
fn coalesce(spans: Vec<RenderedSpan>) -> Vec<RenderedSpan> {
    let mut merged: Vec<RenderedSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(last) if last.role == span.role && last.link == span.link => {
                last.text.push_str(&span.text)
            }
            _ => merged.push(span),
        }
    }
    merged
}

/// The destination to show after a link label, or `None` when repeating it adds
/// nothing (autolinks and bare URLs used as their own label).
fn link_target(dest: &str, label: &str, ctx: &InlineContext) -> Option<String> {
    if dest.is_empty() || dest == label {
        return None;
    }
    Some(display_target(dest, ctx))
}

fn display_target(dest: &str, ctx: &InlineContext) -> String {
    if !is_relative_path(dest) {
        return dest.to_string();
    }
    match &ctx.base_dir {
        // Purely a display convenience: the path is joined, never opened.
        // Markdown targets are always `/` separated, so the base is normalised
        // rather than joined with the platform separator. This also keeps the
        // rendered output identical on Windows.
        Some(base) if !base.as_os_str().is_empty() => {
            let base = base.to_string_lossy().replace('\\', "/");
            format!("{}/{dest}", base.trim_end_matches('/'))
        }
        _ => dest.to_string(),
    }
}

fn is_relative_path(dest: &str) -> bool {
    !dest.starts_with('#') && !dest.starts_with('/') && !has_scheme(dest)
}

fn is_image_path(dest: &str) -> bool {
    let path = dest.split(['?', '#']).next().unwrap_or(dest);
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn placeholder_label(alt: &str, dest: &str) -> String {
    if !alt.trim().is_empty() {
        return alt.to_string();
    }
    Path::new(dest)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(dest)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::model::{ImageInline, LinkInline};

    fn ctx() -> InlineContext {
        InlineContext::default()
    }

    fn text_of(spans: &[RenderedSpan]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    fn role_of(spans: &[RenderedSpan], needle: &str) -> StyleRole {
        spans
            .iter()
            .find(|s| s.text.contains(needle))
            .unwrap_or_else(|| panic!("no span containing {needle:?} in {spans:?}"))
            .role
    }

    #[test]
    fn nested_emphasis_keeps_the_innermost_role() {
        let inlines = vec![Inline::Strong(vec![Inline::Emphasis(vec![Inline::Text(
            "x".into(),
        )])])];
        let out = spans(&inlines, &ctx());
        assert_eq!(role_of(&out, "x"), StyleRole::Emphasis);
    }

    #[test]
    fn tabs_and_line_breaks_in_text_become_spaces() {
        let inlines = vec![
            Inline::Text("a\tb".into()),
            Inline::Code("c\td".into()),
            Inline::RawHtml("<span\nclass=\"x\">".into()),
        ];
        assert_eq!(
            text_of(&spans(&inlines, &ctx())),
            "a bc d<span class=\"x\">"
        );
    }

    #[test]
    fn hard_breaks_split_segments() {
        let inlines = vec![
            Inline::Text("a".into()),
            Inline::HardBreak,
            Inline::Text("b".into()),
        ];
        let out = segments(&inlines, &ctx());
        assert_eq!(out.len(), 2);
        assert_eq!(text_of(&out[0]), "a");
        assert_eq!(text_of(&out[1]), "b");
    }

    #[test]
    fn soft_breaks_become_spaces() {
        let inlines = vec![
            Inline::Text("a".into()),
            Inline::SoftBreak,
            Inline::Text("b".into()),
        ];
        assert_eq!(text_of(&spans(&inlines, &ctx())), "a b");
    }

    #[test]
    fn links_show_an_informative_destination() {
        let inlines = vec![Inline::Link(LinkInline {
            dest: "https://example.com/x".into(),
            title: None,
            content: vec![Inline::Text("docs".into())],
        })];
        let out = spans(&inlines, &ctx());
        assert_eq!(text_of(&out), "docs (https://example.com/x)");
        assert_eq!(role_of(&out, "docs"), StyleRole::Link);
        assert_eq!(role_of(&out, "example.com"), StyleRole::LinkTarget);
    }

    fn link_of(spans: &[RenderedSpan], needle: &str) -> Option<String> {
        spans
            .iter()
            .find(|s| s.text.contains(needle))
            .unwrap_or_else(|| panic!("no span containing {needle:?} in {spans:?}"))
            .link
            .clone()
    }

    fn linked(dest: &str, label: &str) -> Vec<RenderedSpan> {
        let inlines = vec![Inline::Link(LinkInline {
            dest: dest.into(),
            title: None,
            content: vec![Inline::Text(label.into())],
        })];
        spans(&inlines, &ctx())
    }

    #[test]
    fn an_http_label_carries_its_destination() {
        let out = linked("https://example.com/x", "docs");
        assert_eq!(
            link_of(&out, "docs").as_deref(),
            Some("https://example.com/x")
        );
        // The displayed target is not itself a hyperlink; one link per label.
        assert_eq!(link_of(&out, "(https"), None);
    }

    #[test]
    fn only_http_schemes_become_hyperlinks() {
        for dest in [
            "./relative.md",
            "#anchor",
            "/absolute.md",
            "mailto:a@example.com",
            "javascript:alert(1)",
            "file:///etc/passwd",
            ".attachments/design.xlsx",
        ] {
            assert_eq!(hyperlink(dest), None, "{dest} must not be a hyperlink");
        }
        assert!(hyperlink("http://example.com").is_some());
        assert!(hyperlink("HTTPS://Example.com/A").is_some());
    }

    #[test]
    fn destinations_that_could_break_the_escape_sequence_are_refused() {
        assert_eq!(hyperlink("https://example.com/\x1b]8;;evil"), None);
        assert_eq!(hyperlink("https://example.com/a\nb"), None);
        assert_eq!(
            hyperlink(&format!("https://example.com/{}", "x".repeat(2100))),
            None
        );
    }

    #[test]
    fn a_link_label_split_by_a_hard_break_keeps_the_destination() {
        let inlines = vec![Inline::Link(LinkInline {
            dest: "https://example.com".into(),
            title: None,
            content: vec![
                Inline::Text("first".into()),
                Inline::HardBreak,
                Inline::Text("second".into()),
            ],
        })];
        let out = segments(&inlines, &ctx());
        assert_eq!(
            link_of(&out[0], "first").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            link_of(&out[1], "second").as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn an_empty_label_shows_the_destination_once() {
        let inlines = vec![Inline::Link(LinkInline {
            dest: "https://example.com/x".into(),
            title: None,
            content: Vec::new(),
        })];
        assert_eq!(text_of(&spans(&inlines, &ctx())), "https://example.com/x");
    }

    #[test]
    fn autolinks_do_not_repeat_the_destination() {
        let url = "https://example.com";
        let inlines = vec![Inline::Link(LinkInline {
            dest: url.into(),
            title: None,
            content: vec![Inline::Text(url.into())],
        })];
        assert_eq!(text_of(&spans(&inlines, &ctx())), url);
    }

    #[test]
    fn resolved_targets_always_use_forward_slashes() {
        // A Windows base must not leak a backslash into a Markdown target,
        // which would also make rendered output platform dependent.
        for base in ["docs\\sub", "docs/sub", "docs/sub/"] {
            let c = InlineContext {
                base_dir: Some(PathBuf::from(base)),
                ..Default::default()
            };
            let inlines = vec![Inline::Image(ImageInline {
                dest: ".attachments/a.png".into(),
                alt: "a".into(),
            })];
            assert_eq!(
                text_of(&spans(&inlines, &c)),
                "[image: a] (docs/sub/.attachments/a.png)"
            );
        }
    }

    #[test]
    fn relative_targets_resolve_against_the_input_directory() {
        let c = InlineContext {
            base_dir: Some(PathBuf::from("docs")),
            ..Default::default()
        };
        let inlines = vec![Inline::Link(LinkInline {
            dest: "design.md".into(),
            title: None,
            content: vec![Inline::Text("design".into())],
        })];
        let rendered = text_of(&spans(&inlines, &c));
        assert!(rendered.contains("design.md"));
        assert!(rendered.contains("docs"));
    }

    #[test]
    fn a_target_with_any_scheme_is_shown_as_written() {
        let c = InlineContext {
            base_dir: Some(PathBuf::from("docs")),
            ..Default::default()
        };
        for dest in [
            "mailto:a@example.com",
            "tel:+81-3-0000",
            "vscode:extension/x",
        ] {
            assert_eq!(display_target(dest, &c), dest);
        }
    }

    #[test]
    fn stdin_leaves_relative_targets_unresolved() {
        let inlines = vec![Inline::Link(LinkInline {
            dest: "design.md".into(),
            title: None,
            content: vec![Inline::Text("design".into())],
        })];
        assert_eq!(text_of(&spans(&inlines, &ctx())), "design (design.md)");
    }

    #[test]
    fn anchors_and_urls_are_never_joined_to_a_base() {
        let c = InlineContext {
            base_dir: Some(PathBuf::from("docs")),
            ..Default::default()
        };
        for dest in ["#section", "https://example.com/a", "/abs/x.md"] {
            let inlines = vec![Inline::Link(LinkInline {
                dest: dest.into(),
                title: None,
                content: vec![Inline::Text("l".into())],
            })];
            assert_eq!(text_of(&spans(&inlines, &c)), format!("l ({dest})"));
        }
    }

    #[test]
    fn images_render_as_a_placeholder() {
        let inlines = vec![Inline::Image(ImageInline {
            dest: ".attachments/architecture.png".into(),
            alt: "architecture diagram".into(),
        })];
        assert_eq!(
            text_of(&spans(&inlines, &ctx())),
            "[image: architecture diagram] (.attachments/architecture.png)"
        );
    }

    #[test]
    fn non_image_attachments_are_labelled_as_attachments() {
        let inlines = vec![Inline::Image(ImageInline {
            dest: ".attachments/design.xlsx".into(),
            alt: String::new(),
        })];
        assert_eq!(
            text_of(&spans(&inlines, &ctx())),
            "[attachment: design.xlsx] (.attachments/design.xlsx)"
        );
    }

    /// With a glyph the word would be saying what the picture already says, so
    /// the bracket form gives way to it. The destination still follows.
    #[test]
    fn the_nerd_set_names_the_kind_with_a_glyph_instead_of_a_word() {
        let nerd = InlineContext {
            icons: IconSet::Nerd,
            ..InlineContext::default()
        };
        let image = vec![Inline::Image(ImageInline {
            dest: ".attachments/architecture.png".into(),
            alt: "architecture diagram".into(),
        })];
        assert_eq!(
            text_of(&spans(&image, &nerd)),
            format!(
                "{}architecture diagram (.attachments/architecture.png)",
                IconSet::Nerd.image()
            )
        );
        let attachment = vec![Inline::Image(ImageInline {
            dest: ".attachments/design.xlsx".into(),
            alt: String::new(),
        })];
        assert_eq!(
            text_of(&spans(&attachment, &nerd)),
            format!(
                "{}design.xlsx (.attachments/design.xlsx)",
                IconSet::Nerd.attachment("design.xlsx")
            )
        );
    }

    #[test]
    fn footnote_references_are_visible() {
        let out = spans(&[Inline::FootnoteRef("1".into())], &ctx());
        assert_eq!(text_of(&out), "[^1]");
    }

    #[test]
    fn raw_inline_html_is_shown_verbatim() {
        let out = spans(&[Inline::RawHtml("<br/>".into())], &ctx());
        assert_eq!(text_of(&out), "<br/>");
        assert_eq!(role_of(&out, "<br/>"), StyleRole::Muted);
    }
}
