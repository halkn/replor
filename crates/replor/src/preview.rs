//! What the right pane shows for a file.
//!
//! Every kind of file becomes a document the render core lays out, so the
//! pager has one surface to scroll and search: Markdown as itself, a picture
//! as a one-image document, text as a fenced code block, and anything else as
//! a short notice.

use std::sync::Arc;

use replor_render::diagram::{MermaidMode, MermaidWidth};
use replor_render::document;
use replor_render::flavor::Flavor;
use replor_render::image::ImageSupport;
use replor_render::layout::PreparedDocument;
use replor_render::layout::icons::IconSet;
use replor_render::layout::inline::InlineContext;
use replor_render::snapshot::{DocumentSource, Snapshot, SnapshotError, SnapshotPath};

/// Text files larger than this are not shown. Highlighting runs before the
/// first frame and syntect takes about 0.1 ms a line (ADR-0001, S3), so an
/// unbounded file would freeze the UI; a notice in its place costs little.
pub const MAX_TEXT_BYTES: u64 = 1024 * 1024;
pub const MAX_TEXT_LINES: usize = 5_000;

const MARKDOWN: &[&str] = &["md", "markdown", "mdown", "mkd"];
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub flavor: Flavor,
    pub mermaid: MermaidMode,
    pub mermaid_width: MermaidWidth,
    pub highlight: bool,
    pub images: Option<ImageSupport>,
    pub icons: IconSet,
}

pub struct Content {
    pub document: PreparedDocument,
    /// The path shown in the status bar, already free of terminal controls.
    pub title: String,
    /// What the status bar says the file is: a flavor, `text`, `source` or
    /// `image`.
    pub kind: String,
    /// Lines the document has above the file's first line: the opening fence
    /// of a file shown as code.
    pub line_offset: usize,
    /// `Ok` when the document was made from a Markdown file's text, which its
    /// Source is then made from; otherwise why it has none. Anything else is
    /// `Err`, so a document left out by mistake is refused rather than its
    /// notice shown as the file's text.
    pub markdown: Result<(), String>,
}

const NOT_MARKDOWN: &str = "not Markdown";

/// The opening fence a file shown as code has above its first line.
const FENCE_LINES: usize = 1;

pub fn open(snapshot: &Arc<dyn Snapshot>, path: &SnapshotPath, options: &Options) -> Content {
    let extension = extension(path);
    let content = |document, kind: &str, line_offset| Content {
        document,
        title: title(path),
        kind: kind.to_string(),
        line_offset,
        markdown: Err(NOT_MARKDOWN.to_string()),
    };
    if is_markdown(path) {
        let kind = flavor_name(options.flavor);
        match markdown(snapshot, path, options) {
            Ok(document) => Content {
                markdown: Ok(()),
                ..content(document, &kind, 0)
            },
            Err(reason) => Content {
                markdown: Err(reason.clone()),
                ..content(plain(&cannot_show(&reason), options), &kind, 0)
            },
        }
    } else if IMAGES.contains(&extension.as_str()) {
        content(picture(snapshot, path, options), "image", 0)
    } else {
        match text(snapshot, path, &extension, options) {
            Ok(document) => content(document, "text", FENCE_LINES),
            Err(reason) => content(plain(&cannot_show(&reason), options), "text", 0),
        }
    }
}

/// Whether `path` has a Source view besides its Preview. Text is already
/// shown as source, and a picture has none.
pub fn is_markdown(path: &SnapshotPath) -> bool {
    MARKDOWN.contains(&extension(path).as_str())
}

/// A Markdown file as its text, made from the text `rendering` was made from
/// rather than read again, so the two show the same version of the file. It
/// has the same limits as any text file: the Markdown limit is far larger,
/// and highlighting that much would freeze the UI. Fails with the reason when
/// it cannot be shown.
pub fn source(
    rendering: &Content,
    path: &SnapshotPath,
    options: &Options,
) -> Result<Content, String> {
    rendering.markdown.clone()?;
    let body = rendering.document.document().source.as_str();
    if body.len() as u64 > MAX_TEXT_BYTES {
        return Err(larger_than(MAX_TEXT_BYTES));
    }
    if is_binary(body.as_bytes()) {
        return Err(BINARY.to_string());
    }
    let document = code(body, &extension(path), options)?;
    Ok(Content {
        document,
        title: title(path),
        kind: "source".to_string(),
        line_offset: FENCE_LINES,
        // The source's own text is the fenced file, not Markdown to show.
        markdown: Err(NOT_MARKDOWN.to_string()),
    })
}

/// The pane before any file is chosen, or with a message of its own.
pub fn notice(message: &str, options: &Options) -> Content {
    Content {
        document: plain(&format!("{}\n", escape(message)), options),
        title: String::new(),
        kind: String::new(),
        line_offset: 0,
        markdown: Err(NOT_MARKDOWN.to_string()),
    }
}

fn title(path: &SnapshotPath) -> String {
    replor_render::text::visible(path.as_str()).into_owned()
}

fn extension(path: &SnapshotPath) -> String {
    path.name()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

fn markdown(
    snapshot: &Arc<dyn Snapshot>,
    path: &SnapshotPath,
    options: &Options,
) -> Result<PreparedDocument, String> {
    let loaded =
        document::load(Arc::clone(snapshot), path.clone()).map_err(|err| err.to_string())?;
    let built = document::build(
        loaded.text,
        options.flavor,
        options.mermaid,
        options.mermaid_width,
    );
    Ok(PreparedDocument::new(
        built,
        InlineContext {
            base_dir: Some(loaded.base_dir),
            source: Some(loaded.source),
            ..inline(options)
        },
    ))
}

/// A one-image document read from the snapshot, so the picture goes through
/// the same checks as an image a document names. A terminal without images
/// shows the placeholder with the file's name.
fn picture(
    snapshot: &Arc<dyn Snapshot>,
    path: &SnapshotPath,
    options: &Options,
) -> PreparedDocument {
    let resolved = match snapshot.resolve(path) {
        Ok(resolved) => resolved,
        Err(err) => return plain(&cannot_show(&err.to_string()), options),
    };
    let name = resolved.name().unwrap_or_default();
    let text = format!("![{}]({})\n", escape(name), percent_encoded(name));
    let built = document::build(text, Flavor::Gfm, MermaidMode::Off, options.mermaid_width);
    PreparedDocument::new(
        built,
        InlineContext {
            source: Some(DocumentSource::new(Arc::clone(snapshot), resolved)),
            ..inline(options)
        },
    )
}

/// The file as a fenced code block, or why it cannot be shown.
fn text(
    snapshot: &Arc<dyn Snapshot>,
    path: &SnapshotPath,
    extension: &str,
    options: &Options,
) -> Result<PreparedDocument, String> {
    let bytes = match snapshot.read(path, MAX_TEXT_BYTES) {
        Ok(bytes) => bytes,
        Err(SnapshotError::TooLarge { limit }) => return Err(larger_than(limit)),
        Err(err) => return Err(err.to_string()),
    };
    // Before decoding, so a binary file is called that rather than not UTF-8.
    if is_binary(&bytes) {
        return Err(BINARY.to_string());
    }
    let Ok(body) = document::decode(bytes, path.as_str()) else {
        return Err("not UTF-8 text".to_string());
    };
    code(&body, extension, options)
}

const BINARY: &str = "binary file";

/// Git's test for a binary file: a NUL in the first 8000 bytes.
fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|&b| b == 0)
}

fn larger_than(limit: u64) -> String {
    format!("larger than {limit} bytes")
}

/// Text as a fenced code block in the language `extension` names, or why it
/// is too long to show.
fn code(body: &str, extension: &str, options: &Options) -> Result<PreparedDocument, String> {
    if body.lines().count() > MAX_TEXT_LINES {
        return Err(format!("more than {MAX_TEXT_LINES} lines"));
    }
    let longest = longest_run(body, '`');
    let fence = "`".repeat(longest.max(2) + 1);
    let newline = if body.ends_with('\n') || body.is_empty() {
        ""
    } else {
        "\n"
    };
    // The extension is the fence's language, which is how the highlighter
    // looks up a syntax.
    let text = format!("{fence}{extension}\n{body}{newline}{fence}\n");
    let built = document::build(text, Flavor::Gfm, MermaidMode::Off, options.mermaid_width);
    Ok(PreparedDocument::new(built, inline(options)))
}

fn plain(text: &str, options: &Options) -> PreparedDocument {
    let built = document::build(
        text.to_string(),
        Flavor::Gfm,
        MermaidMode::Off,
        options.mermaid_width,
    );
    PreparedDocument::new(built, inline(options))
}

fn cannot_show(reason: &str) -> String {
    format!("**Cannot show this file:** {}\n", escape(reason))
}

fn inline(options: &Options) -> InlineContext {
    InlineContext {
        base_dir: None,
        source: None,
        mermaid: options.mermaid,
        highlight: options.highlight,
        images: options.images,
        icons: options.icons,
    }
}

fn flavor_name(flavor: Flavor) -> String {
    clap::ValueEnum::to_possible_value(&flavor)
        .map(|value| value.get_name().to_string())
        .unwrap_or_default()
}

/// A name or message as literal Markdown text, so a crafted file name cannot
/// become a link or an image.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn percent_encoded(name: &str) -> String {
    name.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn longest_run(text: &str, ch: char) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for c in text.chars() {
        if c == ch {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

#[cfg(test)]
mod tests {
    use super::*;
    use replor_render::layout::{LayoutOptions, layout_document};
    use replor_snapshot::WorktreeSnapshot;
    use std::path::Path;

    const OPTIONS: Options = Options {
        flavor: Flavor::Gfm,
        mermaid: MermaidMode::Unicode,
        mermaid_width: MermaidWidth::Unicode,
        highlight: false,
        images: None,
        icons: IconSet::Unicode,
    };

    fn shown(dir: &Path, name: &str) -> (String, String) {
        let snapshot: Arc<dyn Snapshot> = Arc::new(WorktreeSnapshot::open(dir).unwrap());
        let content = open(&snapshot, &SnapshotPath::new(name).unwrap(), &OPTIONS);
        let rendered = layout_document(&content.document, LayoutOptions::new(80));
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text())
            .collect::<Vec<_>>()
            .join("\n");
        (text, content.kind)
    }

    fn write(dir: &Path, name: &str, bytes: &[u8]) {
        std::fs::write(dir.join(name), bytes).unwrap();
    }

    #[test]
    fn markdown_is_rendered_with_its_diagrams() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.md",
            b"# Title\n\n```mermaid\ngraph LR\n  A[Start] --> B[Done]\n```\n",
        );
        let (text, kind) = shown(dir.path(), "a.md");
        assert!(text.contains("Title"), "{text}");
        assert!(
            text.contains("Start") && !text.contains("graph LR"),
            "{text}"
        );
        assert_eq!(kind, "gfm");
    }

    #[test]
    fn text_is_shown_as_code_even_with_backtick_fences_inside() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", b"fn main() {}\n// ```` not a fence\n");
        let (text, kind) = shown(dir.path(), "a.rs");
        assert!(text.contains("fn main() {}"), "{text}");
        assert!(text.contains("// ```` not a fence"), "{text}");
        assert_eq!(kind, "text");
    }

    #[test]
    fn binary_and_large_files_are_not_shown() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.bin", b"\x7fELF\x00\x01");
        assert!(shown(dir.path(), "a.bin").0.contains("binary file"));
        write(dir.path(), "latin1.txt", b"caf\xe9\n");
        assert!(shown(dir.path(), "latin1.txt").0.contains("not UTF-8"));
        let long = "x\n".repeat(MAX_TEXT_LINES + 1);
        write(dir.path(), "long.txt", long.as_bytes());
        assert!(shown(dir.path(), "long.txt").0.contains("more than"));
        let big = vec![b'x'; MAX_TEXT_BYTES as usize + 1];
        write(dir.path(), "big.txt", &big);
        assert!(shown(dir.path(), "big.txt").0.contains("larger than"));
    }

    /// The rendering's text goes through the same limits as a text file read.
    #[test]
    fn the_source_of_large_or_binary_markdown_is_not_shown() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot: Arc<dyn Snapshot> = Arc::new(WorktreeSnapshot::open(dir.path()).unwrap());
        let source_of = |name: &str| {
            let path = SnapshotPath::new(name).unwrap();
            let rendering = open(&snapshot, &path, &OPTIONS);
            source(&rendering, &path, &OPTIONS).map(|_| ())
        };
        write(
            dir.path(),
            "big.md",
            &vec![b'x'; MAX_TEXT_BYTES as usize + 1],
        );
        let big = source_of("big.md");
        assert!(
            big.as_ref()
                .is_err_and(|reason| reason.contains("larger than")),
            "{big:?}"
        );
        write(dir.path(), "nul.md", b"# A\n\n\x00\n");
        assert_eq!(source_of("nul.md"), Err(BINARY.to_string()));
        write(dir.path(), "ok.md", b"# A\n");
        assert_eq!(source_of("ok.md"), Ok(()));
    }

    #[test]
    fn a_picture_without_images_is_its_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a b.png", b"\x89PNG\r\n\x1a\n");
        let (text, kind) = shown(dir.path(), "a b.png");
        assert!(text.contains("a b"), "{text}");
        assert_eq!(kind, "image");
    }

    #[test]
    fn a_crafted_name_stays_text() {
        assert_eq!(
            escape("![x](/etc/passwd)"),
            "\\!\\[x\\]\\(\\/etc\\/passwd\\)"
        );
        assert_eq!(percent_encoded("a b(1).png"), "a%20b%281%29.png");
    }
}
