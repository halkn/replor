//! Inline images: which terminals get them, which files may be read, and how
//! much of the grid an image occupies.
//!
//! Every rule that can turn an image back into a text placeholder lives here,
//! so `layout` and the backends only ever see a resolved `Placement`.

pub mod dimensions;
pub mod protocol;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use dimensions::{Format, Pixels};
pub use protocol::Protocol;

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
pub fn load(
    dest: &str,
    base_dir: Option<&Path>,
    content_root: Option<&Path>,
) -> Option<Rc<ImageAsset>> {
    // A document read from stdin has no directory to confine reads to.
    let base = base_dir?;
    let path = local_path(dest)?;
    let expected = extension_format(&path)?;
    let path = confined(base, content_root, &path)?;

    let metadata = std::fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return None;
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .ok()?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
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
fn local_path(dest: &str) -> Option<PathBuf> {
    if dest.is_empty() || has_scheme(dest) {
        return None;
    }
    Some(PathBuf::from(percent_decoded(dest)))
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

/// The content root for a document, or `None` outside a repository.
///
/// A wiki keeps its attachments at the root rather than beside each page, so
/// the document's own directory is too narrow a boundary. The root is the
/// nearest ancestor holding a `.git` entry — a directory in a plain clone, a
/// file in a worktree or submodule, so the kind is not checked. `.attachments`
/// is not used as the marker: one repository can hold several of them, and the
/// nearest one is not necessarily the one a page points at.
pub fn content_root(base_dir: &Path) -> Option<PathBuf> {
    let base = directory(base_dir).canonicalize().ok()?;
    base.ancestors()
        .find(|dir| dir.join(".git").try_exists().unwrap_or(false))
        .map(Path::to_path_buf)
}

/// The absolute path, if and only if it stays inside the boundary: the content
/// root when there is one, the document's own directory otherwise. Every side
/// is canonicalised, so `../` and a symlink pointing outside are caught the
/// same way. The reader opened one document; a document cannot make `mdvu` read
/// files elsewhere on the machine.
///
/// A leading `/` is read as a path from the boundary, which is how Azure DevOps
/// and GitHub both resolve it. It is never a path from the filesystem root.
fn confined(base: &Path, content_root: Option<&Path>, path: &Path) -> Option<PathBuf> {
    let base = directory(base).canonicalize().ok()?;
    let boundary = match content_root {
        Some(root) => root.canonicalize().ok()?,
        None => base.clone(),
    };
    let joined = match path.strip_prefix("/") {
        Ok(rest) => boundary.join(rest),
        Err(_) => base.join(path),
    };
    let resolved = joined.canonicalize().ok()?;
    resolved.starts_with(&boundary).then_some(resolved)
}

/// `mdvu a.md` leaves the document's parent as an empty path, which names the
/// working directory but does not canonicalise.
fn directory(base_dir: &Path) -> &Path {
    match base_dir.as_os_str().is_empty() {
        true => Path::new("."),
        false => base_dir,
    }
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
    use std::io::Write;

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
        base_dir: Option<&Path>,
        content_root: Option<&Path>,
        support: ImageSupport,
        max_cols: usize,
    ) -> Option<Rc<Placement>> {
        load(dest, base_dir, content_root)?.place(support, max_cols)
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

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("creating the fixture directory");
        }
        let mut file = std::fs::File::create(&path).expect("creating the fixture");
        file.write_all(bytes).expect("writing the fixture");
        path
    }

    #[test]
    fn an_image_below_the_base_directory_resolves() {
        let dir = tempfile::tempdir().expect("temporary directory");
        write(dir.path(), ".attachments/a.png", &png(200, 100));
        let placement = resolve(".attachments/a.png", Some(dir.path()), None, SUPPORT, 80)
            .expect("the image should resolve");
        assert_eq!((placement.cols, placement.rows), (20, 5));
        assert!(placement.escape().starts_with("\x1b_G"));
    }

    #[test]
    fn a_path_escaping_the_base_directory_is_rejected() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let outside = write(dir.path(), "outside.png", &png(10, 10));
        let base = dir.path().join("doc");
        std::fs::create_dir_all(&base).expect("creating the base directory");
        assert!(resolve("../outside.png", Some(&base), None, SUPPORT, 80).is_none());
        assert!(resolve(outside.to_str().unwrap(), Some(&base), None, SUPPORT, 80).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_base_directory_is_rejected() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let outside = write(dir.path(), "outside.png", &png(10, 10));
        let base = dir.path().join("doc");
        std::fs::create_dir_all(&base).expect("creating the base directory");
        std::os::unix::fs::symlink(&outside, base.join("link.png")).expect("creating the symlink");
        assert!(resolve("link.png", Some(&base), None, SUPPORT, 80).is_none());
    }

    /// A wiki keeps its attachments beside the pages rather than under them, so
    /// a page one directory down reaches them with `../`. `outside.png` sits
    /// above the wiki, where nothing in the document may reach.
    fn wiki() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temporary directory");
        std::fs::create_dir_all(dir.path().join("wiki/.git"))
            .expect("creating the repository marker");
        std::fs::create_dir_all(dir.path().join("wiki/design")).expect("creating the page dir");
        write(
            dir.path(),
            "wiki/.attachments/architecture.png",
            &png(200, 100),
        );
        write(dir.path(), "outside.png", &png(10, 10));
        dir
    }

    #[test]
    fn an_image_above_the_document_resolves_within_the_content_root() {
        let dir = wiki();
        let root = dir.path().join("wiki");
        assert!(
            resolve(
                "../.attachments/architecture.png",
                Some(&root.join("design")),
                Some(&root),
                SUPPORT,
                80
            )
            .is_some()
        );
    }

    #[test]
    fn a_root_relative_path_resolves_from_the_content_root() {
        let dir = wiki();
        let root = dir.path().join("wiki");
        assert!(
            resolve(
                "/.attachments/architecture.png",
                Some(&root.join("design")),
                Some(&root),
                SUPPORT,
                80
            )
            .is_some()
        );
    }

    #[test]
    fn a_path_escaping_the_content_root_is_rejected() {
        let dir = wiki();
        let root = dir.path().join("wiki");
        let base = root.join("design");
        assert!(resolve("../../outside.png", Some(&base), Some(&root), SUPPORT, 80).is_none());
        assert!(resolve("/../outside.png", Some(&base), Some(&root), SUPPORT, 80).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_content_root_is_rejected() {
        let dir = wiki();
        let root = dir.path().join("wiki");
        std::os::unix::fs::symlink(dir.path().join("outside.png"), root.join("link.png"))
            .expect("creating the symlink");
        assert!(
            resolve(
                "../link.png",
                Some(&root.join("design")),
                Some(&root),
                SUPPORT,
                80
            )
            .is_none()
        );
    }

    /// Without a content root the document's own directory is still the
    /// boundary, and a root-relative path is read from there.
    #[test]
    fn a_root_relative_path_without_a_content_root_stays_inside_the_document_directory() {
        let dir = tempfile::tempdir().expect("temporary directory");
        write(dir.path(), ".attachments/a.png", &png(10, 10));
        assert!(resolve("/.attachments/a.png", Some(dir.path()), None, SUPPORT, 80).is_some());
        assert!(resolve("/../a.png", Some(dir.path()), None, SUPPORT, 80).is_none());
    }

    #[test]
    fn the_content_root_is_the_nearest_ancestor_holding_a_git_entry() {
        let dir = wiki();
        let root = dir
            .path()
            .join("wiki")
            .canonicalize()
            .expect("canonical root");
        assert_eq!(content_root(&root.join("design")).as_deref(), Some(&*root));

        // A nested repository owns the pages below it.
        let nested = root.join("design/vendored");
        std::fs::create_dir_all(nested.join(".git")).expect("creating the nested marker");
        assert_eq!(content_root(&nested).as_deref(), Some(&*nested));
    }

    /// A worktree and a submodule record `.git` as a file rather than a
    /// directory, and both are ordinary checkouts to read from.
    #[test]
    fn a_git_file_marks_the_content_root() {
        let dir = tempfile::tempdir().expect("temporary directory");
        write(dir.path(), ".git", b"gitdir: /elsewhere\n");
        let root = dir.path().canonicalize().expect("canonical root");
        assert_eq!(content_root(&root).as_deref(), Some(&*root));
    }

    #[test]
    fn a_directory_outside_a_repository_has_no_content_root() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert_eq!(content_root(dir.path()), None);
    }

    /// `mdvu a.md` leaves the parent as an empty path, which still names the
    /// working directory.
    #[test]
    fn an_empty_base_directory_means_the_working_directory() {
        assert_eq!(directory(Path::new("")), Path::new("."));
        assert_eq!(directory(Path::new("docs")), Path::new("docs"));
    }

    #[test]
    fn remote_and_data_destinations_are_never_fetched() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert!(
            resolve(
                "https://example.com/a.png",
                Some(dir.path()),
                None,
                SUPPORT,
                80
            )
            .is_none()
        );
        assert!(
            resolve(
                "data:image/png;base64,AAAA",
                Some(dir.path()),
                None,
                SUPPORT,
                80
            )
            .is_none()
        );
    }

    #[test]
    fn a_document_from_stdin_has_nothing_to_resolve_against() {
        assert!(resolve("a.png", None, None, SUPPORT, 80).is_none());
    }

    #[test]
    fn an_unsupported_extension_is_rejected() {
        let dir = tempfile::tempdir().expect("temporary directory");
        write(dir.path(), "a.svg", b"<svg></svg>");
        assert!(resolve("a.svg", Some(dir.path()), None, SUPPORT, 80).is_none());
    }

    #[test]
    fn contents_disagreeing_with_the_extension_are_rejected() {
        let dir = tempfile::tempdir().expect("temporary directory");
        write(dir.path(), "a.png", b"GIF89a\x01\x00\x01\x00\x00\x00\x00");
        assert!(resolve("a.png", Some(dir.path()), None, SUPPORT, 80).is_none());
    }

    #[test]
    fn an_oversized_file_is_rejected() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let mut bytes = png(10, 10);
        bytes.resize(MAX_BYTES as usize + 1, 0);
        write(dir.path(), "big.png", &bytes);
        assert!(resolve("big.png", Some(dir.path()), None, SUPPORT, 80).is_none());
    }

    #[test]
    fn a_percent_encoded_name_resolves() {
        let dir = tempfile::tempdir().expect("temporary directory");
        write(dir.path(), "a b.png", &png(10, 10));
        assert!(resolve("a%20b.png", Some(dir.path()), None, SUPPORT, 80).is_some());
    }

    #[test]
    fn a_missing_file_is_rejected() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert!(resolve("nope.png", Some(dir.path()), None, SUPPORT, 80).is_none());
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
