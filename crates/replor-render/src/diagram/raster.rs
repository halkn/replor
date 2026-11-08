//! The only module that may reference `resvg`: SVG in, PNG out.
//!
//! `merman`'s own raster helper builds `usvg` options with the default href
//! resolver, which reads any local path an `<image>` names. A Mermaid image
//! shape (`A@{ img: "/any/path.png" }`) would then let a document make `mdvu`
//! read files outside its content root, so rasterising is owned here instead.

use std::sync::{Arc, OnceLock};

use resvg::tiny_skia::{Color, Pixmap, Transform};
use resvg::usvg::{self, ImageHrefResolver, fontdb};

/// Device pixels per CSS pixel. A diagram is shown at about its CSS size, so
/// on a high-density display a 1x raster would be stretched and blurred.
const SCALE: f32 = 2.0;

/// Most pixels in one picture; a larger diagram is drawn at a lower scale.
/// Terminals drop larger images without a word (`docs/mermaid-image.md`).
const MAX_PIXELS: u64 = 4_000_000;

/// Largest side of the pixmap, below the 10000 px both kitty and Ghostty
/// accept.
const MAX_SIDE: u32 = 8192;

/// Families tried, in order, for the generic `sans-serif` that Mermaid's CSS
/// ends on. `fontdb` maps it to Arial alone, which most Linux systems lack.
const SANS_SERIF: &[&str] = &[
    "Arial",
    "Helvetica",
    "Liberation Sans",
    "DejaVu Sans",
    "Noto Sans",
];

pub struct Raster {
    pub png: Vec<u8>,
    /// Width in CSS pixels, which decides the size on screen whatever the
    /// resolution the picture was drawn at.
    pub css_width: u32,
}

pub fn svg_to_png(svg: &str) -> Result<Raster, String> {
    let options = usvg::Options {
        fontdb: Arc::clone(fonts()),
        image_href_resolver: ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|err| err.to_string())?;
    let css = tree.size();
    let scale = SCALE
        .min((MAX_PIXELS as f32 / (css.width() * css.height())).sqrt())
        .min(MAX_SIDE as f32 / css.width().max(css.height()));
    // Rounded down so the budget holds; the lost fraction is blank margin.
    let width = ((css.width() * scale) as u32).max(1);
    let height = ((css.height() * scale) as u32).max(1);
    let mut pixmap =
        Pixmap::new(width, height).ok_or_else(|| "diagram has no area to draw".to_string())?;
    // Mermaid's default theme has no background, so its dark strokes vanish on a dark terminal.
    pixmap.fill(Color::WHITE);
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(Raster {
        png: pixmap.encode_png().map_err(|err| err.to_string())?,
        css_width: css.width().ceil() as u32,
    })
}

/// Scanning the system fonts dominates the cost of one diagram, so it is done
/// once per process.
fn fonts() -> &'static Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        // macOS's Last Resort font maps every character to a placeholder box.
        // `usvg` keeps a fallback face for a whole run once it has a glyph for
        // every character, so one emoji with no other face would turn a whole
        // label into boxes.
        let last_resort: Vec<_> = db
            .faces()
            .filter(|face| face.post_script_name == "LastResort")
            .map(|face| face.id)
            .collect();
        for id in last_resort {
            db.remove_face(id);
        }
        let installed = |name: &str| {
            db.faces()
                .any(|face| face.families.iter().any(|(family, _)| family == name))
        };
        if let Some(name) = SANS_SERIF.iter().find(|name| installed(name)) {
            let name = (*name).to_string();
            db.set_sans_serif_family(name);
        }
        Arc::new(db)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED_SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='40' height='40'><rect width='40' height='40' fill='red'/></svg>";

    fn has_red(png: &[u8]) -> bool {
        let pixmap = Pixmap::decode_png(png).expect("valid png");
        pixmap
            .pixels()
            .iter()
            .any(|p| p.red() > 200 && p.green() < 50 && p.blue() < 50)
    }

    fn svg_with_image(href: &str) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="40" height="40" viewBox="0 0 40 40"><image href="{href}" width="40" height="40"/><image xlink:href="{href}" width="40" height="40"/></svg>"#
        )
    }

    #[test]
    fn a_plain_svg_becomes_a_white_backed_png() {
        let raster = svg_to_png(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="20"><rect x="0" y="0" width="10" height="10" fill="black"/></svg>"#,
        )
        .expect("should rasterise");
        assert_eq!(raster.css_width, 30);
        let pixmap = Pixmap::decode_png(&raster.png).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (60, 40));
        let corner = pixmap.pixel(59, 39).unwrap();
        assert_eq!(
            (corner.red(), corner.green(), corner.blue()),
            (255, 255, 255)
        );
    }

    /// A document must not be able to make `mdvu` read files by naming them
    /// in a diagram.
    // The file is a fixture the rasteriser must not read.
    #[allow(clippy::disallowed_methods)]
    #[test]
    fn local_files_named_by_an_image_are_never_read() {
        let dir = tempfile::tempdir().expect("temporary directory");
        // Bitmap decoding is compiled out, so only an SVG file shows it was read.
        let path = dir.path().join("red.svg");
        std::fs::write(&path, RED_SVG).unwrap();
        let path = path.to_str().unwrap().replace('\\', "/");

        for href in [path.clone(), format!("file://{path}")] {
            let raster = svg_to_png(&svg_with_image(&href)).expect("should rasterise");
            assert!(!has_red(&raster.png), "{href} was read");
        }
    }

    #[test]
    fn embedded_data_images_are_not_decoded_either() {
        let href = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='40' height='40'%3E%3Crect width='40' height='40' fill='red'/%3E%3C/svg%3E";
        let raster = svg_to_png(&svg_with_image(href)).expect("should rasterise");
        assert!(!has_red(&raster.png));
    }

    /// A large diagram is drawn at a lower resolution rather than refused, and
    /// keeps its CSS width so it is shown at the same size.
    #[test]
    fn a_large_diagram_is_drawn_within_the_pixel_budget() {
        let raster = svg_to_png(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="1400"></svg>"#,
        )
        .expect("should rasterise");
        let pixmap = Pixmap::decode_png(&raster.png).unwrap();
        assert!(u64::from(pixmap.width()) * u64::from(pixmap.height()) <= MAX_PIXELS);
        assert!(pixmap.width() > 1600, "{}", pixmap.width());
        assert_eq!(raster.css_width, 1600);
    }

    #[test]
    fn a_long_diagram_is_drawn_within_the_side_limit() {
        let raster = svg_to_png(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="9000" height="10"></svg>"#,
        )
        .expect("should rasterise");
        let pixmap = Pixmap::decode_png(&raster.png).unwrap();
        assert!(pixmap.width() <= MAX_SIDE, "{}", pixmap.width());
        assert_eq!(raster.css_width, 9000);
    }

    /// The face each character of a label is drawn with, under the font list
    /// Mermaid's CSS gives.
    fn faces_of(label: &str) -> Vec<(String, fontdb::ID)> {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="40"><text x="0" y="20" style="font-family:&quot;Recursive Variable&quot;,arial,sans-serif;font-size:16px">{label}</text></svg>"#
        );
        let options = usvg::Options {
            fontdb: Arc::clone(fonts()),
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_str(&svg, &options).expect("valid svg");
        let mut faces = Vec::new();
        collect_faces(tree.root(), &mut faces);
        faces
    }

    fn collect_faces(group: &usvg::Group, faces: &mut Vec<(String, fontdb::ID)>) {
        for node in group.children() {
            match node {
                usvg::Node::Group(group) => collect_faces(group, faces),
                usvg::Node::Text(text) => faces.extend(text.layouted().iter().flat_map(|span| {
                    span.positioned_glyphs
                        .iter()
                        .map(|glyph| (glyph.text.clone(), glyph.font))
                })),
                _ => {}
            }
        }
    }

    /// An emoji must not change the face of the rest of the label. Which
    /// faces exist is up to the system, so this compares against the same
    /// label without the emoji rather than naming a face.
    #[test]
    fn an_emoji_leaves_the_faces_of_the_other_characters_alone() {
        let plain = faces_of("承認");
        let with_emoji = faces_of("承認 ✅");
        assert_eq!(plain.len(), 2, "{plain:?}");
        assert_eq!(with_emoji[..2], plain[..], "{with_emoji:?}");
    }
}
