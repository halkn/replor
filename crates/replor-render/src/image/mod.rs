//! Inline images: which terminals get them, which files may be read, and how
//! much of the grid an image occupies.
//!
//! Every rule that can turn an image back into a text placeholder lives here,
//! so `layout` and the backends only ever see a resolved `Placement`.

pub mod dimensions;
pub mod protocol;

use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use dimensions::{Format, Pixels};
pub use protocol::Protocol;

use crate::snapshot::{DocumentSource, SnapshotPath};

/// Largest file that will be sent to the terminal. A payload is base64 encoded
/// and travels over the tty, so an unbounded read would stall the terminal.
const MAX_BYTES: u64 = 10 * 1024 * 1024;

/// Tallest image, in cells. An image is useless once it is taller than the
/// viewport, and the layout has no height to measure against.
pub const MAX_ROWS: usize = 20;

/// Used when the terminal reports no pixel size. Cells are about twice as tall
/// as they are wide, which is enough to keep an image from looking stretched.
const ASSUMED_CELL: CellSize = CellSize {
    width: 10,
    height: 20,
};

/// Formats both protocols decode themselves. SVG is absent on purpose: no
/// terminal renders it, and the rasteriser in `diagram` only ever sees SVG that
/// `merman` produced.
const ALLOWED: &[(&str, Format)] = &[
    ("png", Format::Png),
    ("jpg", Format::Jpeg),
    ("jpeg", Format::Jpeg),
    ("gif", Format::Gif),
    ("webp", Format::WebP),
];

/// A monospace cell in a browser, in CSS pixels, for sizing pictures `mdvu`
/// drew itself.
const CSS_CELL_WIDTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSize {
    pub width: u16,
    pub height: u16,
}

/// What the running terminal can draw. Held by `InlineContext`, so a change
/// here re-runs layout the same way a width change does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSupport {
    pub protocol: Protocol,
    pub cell: CellSize,
}

impl ImageSupport {
    /// `window` is `None` when the terminal size is unavailable. The caller
    /// reads it because the render core does not touch the terminal.
    pub fn detect(protocol: Protocol, window: Option<WindowSize>) -> Self {
        let cell = window.map_or(ASSUMED_CELL, |size| {
            cell_size(size.columns, size.rows, size.width, size.height)
        });
        Self { protocol, cell }
    }
}

/// The terminal's size in cells and in pixels, as `TIOCGWINSZ` reports it.
/// A pixel size of 0 means the terminal did not report one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSize {
    pub columns: u16,
    pub rows: u16,
    pub width: u16,
    pub height: u16,
}

/// A resolved image: the file's own bytes and the area they should cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub cols: usize,
    pub rows: usize,
    /// The `ImageAsset`'s id, shared by every placement of it across layouts,
    /// so a kitty terminal keeps one stored copy while the pager scrolls and
    /// resizes.
    pub id: u32,
    protocol: Protocol,
    asset: Rc<ImageAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageAsset {
    id: u32,
    pixels: Pixels,
    bytes: Rc<[u8]>,
    css_width: Option<u32>,
}

impl ImageAsset {
    /// A PNG `mdvu` drew itself, such as a rasterised diagram, that is
    /// `css_width` CSS pixels wide. No file is read, so only the format and the
    /// area are checked.
    ///
    /// Its size follows the CSS pixels a browser would show rather than the
    /// terminal's cell or the picture's resolution: a high-density display
    /// reports device pixels, which would halve the picture, and a large
    /// diagram is drawn at a lower resolution than a small one.
    pub fn from_png(bytes: Rc<[u8]>, css_width: u32) -> Option<Rc<Self>> {
        if dimensions::format_of(&bytes) != Some(Format::Png) {
            return None;
        }
        let pixels = dimensions::dimensions(&bytes)?;
        Some(Self::new(bytes, pixels, Some(css_width)))
    }

    fn new(bytes: Rc<[u8]>, pixels: Pixels, css_width: Option<u32>) -> Rc<Self> {
        static NEXT_ID: AtomicU32 = AtomicU32::new(1);
        Rc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            bytes,
            pixels,
            css_width,
        })
    }

    pub fn place(self: &Rc<Self>, support: ImageSupport, max_cols: usize) -> Option<Rc<Placement>> {
        if max_cols == 0 {
            return None;
        }
        let (cols, rows) = match self.css_width {
            Some(width) => fit_cols(
                self.pixels,
                support.cell,
                (width as usize).div_ceil(CSS_CELL_WIDTH).max(1),
                max_cols,
            ),
            None => fit(self.pixels, support.cell, max_cols),
        };
        Some(Rc::new(Placement {
            cols,
            rows,
            id: self.id,
            protocol: support.protocol,
            asset: Rc::clone(self),
        }))
    }
}

impl Placement {
    /// The escape sequence that sends and draws this image at the cursor in
    /// one step. It leaves the cursor where it was.
    pub fn escape(&self) -> String {
        protocol::place(self.protocol, &self.asset.bytes, self.cols, self.rows)
    }

    /// The escape sequence that stores this image in the terminal under `id`
    /// without drawing it, or `None` when the protocol cannot store images.
    pub fn transmit(&self) -> Option<String> {
        match self.protocol {
            Protocol::Kitty => Some(protocol::transmit(&self.asset.bytes, self.id)),
            Protocol::Iterm2 => None,
        }
    }

    /// Whether the terminal keeps the image, so it can be moved, cropped and
    /// removed without being sent again.
    pub fn stored(&self) -> bool {
        self.protocol == Protocol::Kitty
    }

    /// The escape sequence that removes this image from the screen, or `None`
    /// when the protocol can only repaint the whole screen.
    pub fn remove(&self) -> Option<String> {
        self.stored().then(|| protocol::remove(self.id))
    }

    /// The escape sequence that draws `rows` of this image's rows, starting at
    /// row `skip`, at the cursor after [`Placement::transmit`], or the whole
    /// image where nothing was stored.
    pub fn show(&self, skip: usize, rows: usize) -> String {
        match self.protocol {
            Protocol::Kitty if skip == 0 && rows == self.rows => {
                protocol::show(self.id, self.cols, self.rows)
            }
            Protocol::Kitty => {
                let height = u64::from(self.asset.pixels.height);
                let at = |row: usize| (row as u64 * height / self.rows as u64) as u32;
                let top = at(skip);
                let bottom = at(skip + rows);
                let source = (0, top, self.asset.pixels.width, bottom - top);
                protocol::show_part(self.id, self.cols, rows, source)
            }
            Protocol::Iterm2 => self.escape(),
        }
    }
}

/// Environment facts that decide whether images are drawn. Kept as data so the
/// decision stays a pure function and remains testable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    pub term: Option<String>,
    pub term_program: Option<String>,
    pub kitty_window_id: bool,
    pub konsole_version: bool,
    pub tmux: bool,
}

impl Env {
    pub fn detect() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        Self {
            term: var("TERM"),
            term_program: var("TERM_PROGRAM"),
            kitty_window_id: var("KITTY_WINDOW_ID").is_some(),
            konsole_version: var("KONSOLE_VERSION").is_some(),
            tmux: var("TMUX").is_some(),
        }
    }
}

/// The protocol a terminal is known to support, from environment variables
/// alone. The terminal is never asked, so an unknown terminal gets no images
/// rather than a stray escape sequence in its output.
pub fn protocol_from_env(env: &Env) -> Option<Protocol> {
    // Inside a multiplexer the sequence has to be wrapped for passthrough, and
    // whether that works depends on the outer terminal and the tmux version.
    // Neither is visible here, so `auto` stays off and `--images` opts in.
    if env.tmux {
        return None;
    }
    if env.kitty_window_id
        || env.konsole_version
        || env.term.as_deref() == Some("xterm-kitty")
        || env.term.as_deref() == Some("xterm-ghostty")
    {
        return Some(Protocol::Kitty);
    }
    match env.term_program.as_deref() {
        Some("ghostty") | Some("WezTerm") => Some(Protocol::Kitty),
        Some("iTerm.app") => Some(Protocol::Iterm2),
        _ => None,
    }
}

/// Reads the image `dest` points at, or `None` when any rule below is not met.
/// A rejected image keeps its text placeholder; it is never an error and never
/// changes the exit code.
///
/// The snapshot keeps reads inside its root; the content root is checked here,
/// on the path the snapshot resolved, so a symlink cannot leave it either.
pub fn load(dest: &str, source: Option<&DocumentSource>) -> Option<Rc<ImageAsset>> {
    // A document read from stdin has no snapshot to read from.
    let source = source?;
    let dest = local_path(dest)?;
    let expected = extension_format(Path::new(&dest))?;
    let content_root = source.content_root();
    let path = SnapshotPath::from_reference(source.path(), content_root, &dest).ok()?;
    let resolved = source.snapshot().resolve(&path).ok()?;
    if !resolved.starts_with(content_root) {
        return None;
    }
    let bytes = source.snapshot().read(&resolved, MAX_BYTES).ok()?;
    // A file whose contents disagree with its name is not sent on: the
    // terminal, not `mdvu`, would be the one to decode it.
    if dimensions::format_of(&bytes) != Some(expected) {
        return None;
    }
    let pixels = dimensions::dimensions(&bytes)?;
    Some(ImageAsset::new(bytes.into(), pixels, None))
}

/// A local path, or `None` for anything with a URL scheme. Remote images are
/// not fetched: `mdvu` makes no network requests.
fn local_path(dest: &str) -> Option<String> {
    if dest.is_empty() || has_scheme(dest) {
        return None;
    }
    Some(percent_decoded(dest))
}

/// `scheme:` per RFC 3986, requiring at least two characters so a Windows drive
/// letter is not mistaken for one.
pub(crate) fn has_scheme(dest: &str) -> bool {
    let Some(colon) = dest.find(':') else {
        return false;
    };
    let scheme = &dest[..colon];
    scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

fn percent_decoded(dest: &str) -> String {
    let bytes = dest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match (bytes[at], bytes.get(at + 1), bytes.get(at + 2)) {
            (b'%', Some(high), Some(low)) => {
                match (
                    char::from(*high).to_digit(16),
                    char::from(*low).to_digit(16),
                ) {
                    (Some(high), Some(low)) => {
                        out.push((high * 16 + low) as u8);
                        at += 3;
                    }
                    _ => {
                        out.push(bytes[at]);
                        at += 1;
                    }
                }
            }
            _ => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| dest.to_string())
}

fn extension_format(path: &Path) -> Option<Format> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    ALLOWED
        .iter()
        .find(|(name, _)| *name == extension)
        .map(|(_, format)| *format)
}

/// Cell size in pixels, falling back when the terminal reports none.
fn cell_size(columns: u16, rows: u16, width: u16, height: u16) -> CellSize {
    if columns == 0 || rows == 0 || width == 0 || height == 0 {
        return ASSUMED_CELL;
    }
    CellSize {
        width: (width / columns).max(1),
        height: (height / rows).max(1),
    }
}

/// Cells a picture `natural_cols` wide should cover, never wider than
/// `max_cols` or taller than `MAX_ROWS`, with the aspect ratio kept.
fn fit_cols(
    pixels: Pixels,
    cell: CellSize,
    natural_cols: usize,
    max_cols: usize,
) -> (usize, usize) {
    let (width, height) = (u64::from(pixels.width.max(1)), u64::from(pixels.height));
    let (cell_w, cell_h) = (u64::from(cell.width), u64::from(cell.height));
    let rows_for =
        |cols: usize| ((cols as u64 * cell_w * height).div_ceil(width * cell_h) as usize).max(1);
    let cols = natural_cols.min(max_cols).max(1);
    let rows = rows_for(cols);
    if rows <= MAX_ROWS {
        return (cols, rows);
    }
    let cols = ((MAX_ROWS as u64 * cell_h * width) / (height * cell_w).max(1)) as usize;
    (cols.clamp(1, max_cols), MAX_ROWS)
}

/// Cells the image should cover, never wider than `max_cols` or taller than
/// `MAX_ROWS`, with the aspect ratio kept.
fn fit(pixels: Pixels, cell: CellSize, max_cols: usize) -> (usize, usize) {
    let natural_cols = (pixels.width as usize)
        .div_ceil(usize::from(cell.width))
        .max(1);
    let natural_rows = (pixels.height as usize)
        .div_ceil(usize::from(cell.height))
        .max(1);

    let cols = natural_cols.min(max_cols).max(1);
    let rows = (natural_rows * cols).div_ceil(natural_cols).max(1);
    if rows <= MAX_ROWS {
        return (cols, rows);
    }
    let rows = MAX_ROWS;
    let cols = ((natural_cols * rows) / natural_rows).clamp(1, max_cols);
    (cols, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::memory::MemorySnapshot;
    use std::sync::Arc;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0]);
        out
    }

    fn from_png(
        bytes: Vec<u8>,
        support: ImageSupport,
        max_cols: usize,
        css_width: u32,
    ) -> Option<Rc<Placement>> {
        ImageAsset::from_png(bytes.into(), css_width)?.place(support, max_cols)
    }

    fn resolve(
        dest: &str,
        source: Option<&DocumentSource>,
        support: ImageSupport,
        max_cols: usize,
    ) -> Option<Rc<Placement>> {
        load(dest, source)?.place(support, max_cols)
    }

    /// A wiki keeps its attachments beside the pages rather than under them,
    /// so a page one directory down reaches them with `../`. `outside.png`
    /// sits above the wiki, where nothing in the document may reach.
    fn wiki() -> Arc<MemorySnapshot> {
        let snapshot = MemorySnapshot::new("wiki");
        snapshot.write("wiki/.attachments/architecture.png", png(200, 100));
        snapshot.write("outside.png", png(10, 10));
        snapshot
    }

    const SUPPORT: ImageSupport = ImageSupport {
        protocol: Protocol::Kitty,
        cell: CellSize {
            width: 10,
            height: 20,
        },
    };

    /// 442 CSS px wide: 8 CSS px per column, whatever the terminal's own cell
    /// or the picture's resolution measures.
    #[test]
    fn a_drawn_png_is_sized_by_its_css_pixels() {
        let placement = from_png(png(884, 366), SUPPORT, 80, 442).expect("placed");
        assert_eq!((placement.cols, placement.rows), (56, 12));

        let dense = ImageSupport {
            cell: CellSize {
                width: 18,
                height: 36,
            },
            ..SUPPORT
        };
        let placement = from_png(png(884, 366), dense, 80, 442).expect("placed");
        assert_eq!((placement.cols, placement.rows), (56, 12));

        let coarse = from_png(png(442, 183), SUPPORT, 80, 442).expect("placed");
        assert_eq!((coarse.cols, coarse.rows), (56, 12));
    }

    #[test]
    fn a_drawn_png_still_fits_the_width_and_the_row_cap() {
        let narrow = from_png(png(884, 366), SUPPORT, 28, 442).expect("placed");
        assert_eq!((narrow.cols, narrow.rows), (28, 6));
        let tall = from_png(png(1096, 900), SUPPORT, 80, 548).expect("placed");
        assert_eq!(tall.rows, MAX_ROWS);
        assert!(tall.cols < 69, "{}", tall.cols);
    }

    #[test]
    fn bytes_that_are_not_a_png_are_not_placed() {
        assert!(from_png(b"GIF89a".to_vec(), SUPPORT, 80, 100).is_none());
        assert!(from_png(png(200, 100), SUPPORT, 0, 100).is_none());
    }

    #[test]
    fn an_image_beside_the_document_resolves() {
        let snapshot = MemorySnapshot::new("");
        snapshot.write("docs/.attachments/a.png", png(200, 100));
        let source = snapshot.source("docs/page.md");
        let placement = resolve(".attachments/a.png", Some(&source), SUPPORT, 80)
            .expect("the image should resolve");
        assert_eq!((placement.cols, placement.rows), (20, 5));
        assert!(placement.escape().starts_with("\x1b_G"));
    }

    #[test]
    fn an_image_above_the_document_resolves_within_the_content_root() {
        let source = wiki().source("wiki/design/page.md");
        assert!(
            resolve(
                "../.attachments/architecture.png",
                Some(&source),
                SUPPORT,
                80
            )
            .is_some()
        );
    }

    #[test]
    fn a_root_relative_path_resolves_from_the_content_root() {
        let source = wiki().source("wiki/design/page.md");
        assert!(resolve("/.attachments/architecture.png", Some(&source), SUPPORT, 80).is_some());
    }

    #[test]
    fn a_path_escaping_the_content_root_is_rejected() {
        let source = wiki().source("wiki/design/page.md");
        assert!(resolve("../../outside.png", Some(&source), SUPPORT, 80).is_none());
        assert!(resolve("/../outside.png", Some(&source), SUPPORT, 80).is_none());
        assert!(resolve("../../../outside.png", Some(&source), SUPPORT, 80).is_none());
    }

    #[test]
    fn remote_and_data_destinations_are_never_fetched() {
        let source = MemorySnapshot::new("").source("a.md");
        assert!(resolve("https://example.com/a.png", Some(&source), SUPPORT, 80).is_none());
        assert!(resolve("data:image/png;base64,AAAA", Some(&source), SUPPORT, 80).is_none());
    }

    #[test]
    fn a_document_from_stdin_has_nothing_to_resolve_against() {
        assert!(resolve("a.png", None, SUPPORT, 80).is_none());
    }

    #[test]
    fn an_unsupported_extension_is_rejected() {
        let snapshot = MemorySnapshot::new("");
        snapshot.write("a.svg", b"<svg></svg>".to_vec());
        assert!(resolve("a.svg", Some(&snapshot.source("a.md")), SUPPORT, 80).is_none());
    }

    #[test]
    fn contents_disagreeing_with_the_extension_are_rejected() {
        let snapshot = MemorySnapshot::new("");
        snapshot.write("a.png", b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec());
        assert!(resolve("a.png", Some(&snapshot.source("a.md")), SUPPORT, 80).is_none());
    }

    #[test]
    fn an_oversized_file_is_rejected() {
        let snapshot = MemorySnapshot::new("");
        let mut bytes = png(10, 10);
        bytes.resize(MAX_BYTES as usize + 1, 0);
        snapshot.write("big.png", bytes);
        assert!(resolve("big.png", Some(&snapshot.source("a.md")), SUPPORT, 80).is_none());
    }

    #[test]
    fn a_percent_encoded_name_resolves() {
        let snapshot = MemorySnapshot::new("");
        snapshot.write("a b.png", png(10, 10));
        assert!(resolve("a%20b.png", Some(&snapshot.source("a.md")), SUPPORT, 80).is_some());
    }

    #[test]
    fn a_missing_file_is_rejected() {
        let source = MemorySnapshot::new("").source("a.md");
        assert!(resolve("nope.png", Some(&source), SUPPORT, 80).is_none());
    }

    #[test]
    fn an_image_wider_than_the_column_is_scaled_down() {
        let cell = CellSize {
            width: 10,
            height: 20,
        };
        // 100x40 cells naturally, capped to 40 columns.
        assert_eq!(
            fit(
                Pixels {
                    width: 1000,
                    height: 800
                },
                cell,
                40
            ),
            (40, 16)
        );
    }

    #[test]
    fn a_tall_image_is_capped_in_rows_and_keeps_its_ratio() {
        let cell = CellSize {
            width: 10,
            height: 20,
        };
        let (cols, rows) = fit(
            Pixels {
                width: 200,
                height: 2000,
            },
            cell,
            80,
        );
        assert_eq!(rows, MAX_ROWS);
        assert_eq!(cols, 4);
    }

    #[test]
    fn a_small_image_keeps_its_natural_size() {
        let cell = CellSize {
            width: 10,
            height: 20,
        };
        assert_eq!(
            fit(
                Pixels {
                    width: 50,
                    height: 40
                },
                cell,
                80
            ),
            (5, 2)
        );
    }

    #[test]
    fn a_terminal_reporting_no_pixel_size_gets_the_assumed_cell() {
        assert_eq!(cell_size(80, 24, 0, 0), ASSUMED_CELL);
        assert_eq!(
            cell_size(80, 24, 800, 480),
            CellSize {
                width: 10,
                height: 20
            }
        );
    }

    #[test]
    fn detect_measures_the_cell_from_the_window_the_caller_passes() {
        let window = WindowSize {
            columns: 100,
            rows: 50,
            width: 1600,
            height: 1800,
        };
        assert_eq!(
            ImageSupport::detect(Protocol::Kitty, Some(window)).cell,
            CellSize {
                width: 16,
                height: 36
            }
        );
        assert_eq!(
            ImageSupport::detect(Protocol::Kitty, None).cell,
            ASSUMED_CELL
        );
    }

    #[test]
    fn protocols_are_recognised_from_the_environment() {
        let kitty = Env {
            term: Some("xterm-kitty".into()),
            ..Env::default()
        };
        assert_eq!(protocol_from_env(&kitty), Some(Protocol::Kitty));

        let ghostty = Env {
            term_program: Some("ghostty".into()),
            ..Env::default()
        };
        assert_eq!(protocol_from_env(&ghostty), Some(Protocol::Kitty));

        let iterm = Env {
            term_program: Some("iTerm.app".into()),
            ..Env::default()
        };
        assert_eq!(protocol_from_env(&iterm), Some(Protocol::Iterm2));

        let apple = Env {
            term_program: Some("Apple_Terminal".into()),
            term: Some("xterm-256color".into()),
            ..Env::default()
        };
        assert_eq!(protocol_from_env(&apple), None);
        assert_eq!(protocol_from_env(&Env::default()), None);
    }

    /// Passthrough depends on the outer terminal and the tmux version, neither
    /// of which is visible from the environment.
    #[test]
    fn a_multiplexer_turns_detection_off() {
        let env = Env {
            term: Some("xterm-kitty".into()),
            tmux: true,
            ..Env::default()
        };
        assert_eq!(protocol_from_env(&env), None);
    }
}
