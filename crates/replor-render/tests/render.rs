//! Golden rendering tests. The stdout path of the render core is driven end to
//! end so the layout engine and output backends are all covered.
//!
//! The helpers take mdvu's flags so the snapshots stay comparable with mdvu's.
//! Values the flags do not set are fixed to what mdvu's CLI picks under test:
//! stdout is not a tty, `--paging never`, and no colour or terminal variables.
//! See `docs/design/p1-core.md`.

use std::path::Path;

use clap::ValueEnum;
use replor_render::diagram::{MermaidMode, MermaidWidth};
use replor_render::flavor::Flavor;
use replor_render::layout::icons::IconSet;
use replor_render::layout::theme::Variant;
use replor_render::output::ColorChoice;
use replor_render::{RenderOptions, render_to_string};

// The fixture is the test's input, read by the test rather than the render
// core.
#[allow(clippy::disallowed_methods)]
fn render(fixture: &str, width: &str, color: ColorChoice, extra: &[&str]) -> String {
    let mut options = RenderOptions {
        base_dir: Path::new(fixture).parent().map(Path::to_path_buf),
        width: width.parse().expect("width should be a number"),
        flavor: Flavor::AzureDevops,
        mermaid: MermaidMode::Unicode,
        mermaid_width: MermaidWidth::Unicode,
        icons: IconSet::Unicode,
        color,
        variant: Variant::Dark,
        // mdvu highlights only when the output carries ANSI escapes, and emits
        // hyperlinks only to a tty.
        highlight: color == ColorChoice::Ansi,
        hyperlinks: false,
    };
    for pair in extra.chunks(2) {
        let [flag, value] = pair else {
            panic!("flag without a value: {pair:?}");
        };
        match *flag {
            "--flavor" => options.flavor = from_flag(value),
            "--mermaid" => options.mermaid = from_flag(value),
            "--icons" => options.icons = from_flag(value),
            // mdvu has no such flag; it stands for replor's `mermaid_width` key.
            "--mermaid-width" => options.mermaid_width = from_flag(value),
            other => panic!("unsupported flag: {other}"),
        }
    }
    let text = std::fs::read_to_string(fixture).expect("fixture should be readable");
    render_to_string(text, &options).expect("rendering should succeed")
}

fn from_flag<T: ValueEnum>(value: &str) -> T {
    T::from_str(value, false).unwrap_or_else(|err| panic!("{value}: {err}"))
}

fn plain(fixture: &str, width: &str) -> String {
    render(fixture, width, ColorChoice::Plain, &[])
}

macro_rules! golden {
    ($name:ident, $fixture:expr, $width:expr) => {
        #[test]
        fn $name() {
            insta::assert_snapshot!(plain($fixture, $width));
        }
    };
}

/// Styled output, with the escapes made visible so a snapshot stays readable.
fn styled(fixture: &str, width: &str) -> String {
    render(fixture, width, ColorChoice::Ansi, &[]).replace('\x1b', "ESC")
}

golden!(gfm_showcase_40, "tests/fixtures/gfm/showcase.md", "40");
golden!(gfm_showcase_80, "tests/fixtures/gfm/showcase.md", "80");
golden!(gfm_showcase_120, "tests/fixtures/gfm/showcase.md", "120");
golden!(gfm_tables_40, "tests/fixtures/gfm/tables.md", "40");
golden!(gfm_tables_80, "tests/fixtures/gfm/tables.md", "80");
golden!(gfm_nested_40, "tests/fixtures/gfm/nested.md", "40");
golden!(gfm_nested_80, "tests/fixtures/gfm/nested.md", "80");
golden!(gfm_footnotes_40, "tests/fixtures/gfm/footnotes.md", "40");
golden!(gfm_alerts_40, "tests/fixtures/gfm/alerts.md", "40");
golden!(gfm_alerts_80, "tests/fixtures/gfm/alerts.md", "80");
golden!(gfm_alerts_120, "tests/fixtures/gfm/alerts.md", "120");
golden!(gfm_wide_table_40, "tests/fixtures/gfm/wide-table.md", "40");
golden!(
    gfm_wide_table_120,
    "tests/fixtures/gfm/wide-table.md",
    "120"
);
golden!(
    japanese_showcase_40,
    "tests/fixtures/japanese/showcase.md",
    "40"
);
golden!(
    japanese_showcase_80,
    "tests/fixtures/japanese/showcase.md",
    "80"
);
golden!(
    japanese_punctuation_40,
    "tests/fixtures/japanese/punctuation.md",
    "40"
);
golden!(
    japanese_punctuation_80,
    "tests/fixtures/japanese/punctuation.md",
    "80"
);
golden!(
    japanese_tables_40,
    "tests/fixtures/japanese/tables.md",
    "40"
);
golden!(
    japanese_tables_80,
    "tests/fixtures/japanese/tables.md",
    "80"
);

fn plain_with(fixture: &str, width: &str, extra: &[&str]) -> String {
    render(fixture, width, ColorChoice::Plain, extra)
}

macro_rules! golden_with {
    ($name:ident, $fixture:expr, $width:expr, $extra:expr) => {
        #[test]
        fn $name() {
            insta::assert_snapshot!(plain_with($fixture, $width, &$extra));
        }
    };
}

golden_with!(
    azure_showcase_80,
    "tests/fixtures/azure-devops/showcase.md",
    "80",
    ["--flavor", "azure-devops"]
);
golden_with!(
    azure_showcase_as_gfm_80,
    "tests/fixtures/azure-devops/showcase.md",
    "80",
    ["--flavor", "gfm"]
);
golden_with!(
    azure_toc_80,
    "tests/fixtures/azure-devops/toc.md",
    "80",
    ["--flavor", "azure-devops"]
);
golden_with!(
    azure_details_80,
    "tests/fixtures/azure-devops/details.md",
    "80",
    ["--flavor", "azure-devops"]
);
golden_with!(
    azure_attachments_80,
    "tests/fixtures/azure-devops/attachments.md",
    "80",
    ["--flavor", "azure-devops"]
);
golden_with!(
    azure_containers_80,
    "tests/fixtures/azure-devops/containers.md",
    "80",
    ["--flavor", "azure-devops"]
);
golden_with!(
    azure_malformed_80,
    "tests/fixtures/azure-devops/malformed.md",
    "80",
    ["--flavor", "azure-devops"]
);

golden!(
    mermaid_azure_graph_80,
    "tests/fixtures/mermaid/azure-graph.md",
    "80"
);
golden!(
    mermaid_flowchart_warning_80,
    "tests/fixtures/mermaid/flowchart-warning.md",
    "80"
);
golden!(
    mermaid_sequence_80,
    "tests/fixtures/mermaid/sequence.md",
    "80"
);
golden!(mermaid_er_80, "tests/fixtures/mermaid/er.md", "80");
golden!(mermaid_class_80, "tests/fixtures/mermaid/class.md", "80");
golden!(mermaid_state_80, "tests/fixtures/mermaid/state.md", "80");
golden!(
    mermaid_er_japanese_80,
    "tests/fixtures/mermaid/er-japanese.md",
    "80"
);
golden_with!(
    mermaid_no_text_layout_80,
    "tests/fixtures/mermaid/no-text-layout.md",
    "80",
    ["--flavor", "gfm"]
);
golden!(
    mermaid_structured_text_80,
    "tests/fixtures/mermaid/structured-text.md",
    "80"
);
golden!(
    mermaid_japanese_80,
    "tests/fixtures/mermaid/japanese.md",
    "80"
);
golden_with!(
    mermaid_japanese_cjk_80,
    "tests/fixtures/mermaid/japanese.md",
    "80",
    ["--mermaid-width", "cjk"]
);
golden!(
    mermaid_invalid_80,
    "tests/fixtures/mermaid/invalid.md",
    "80"
);
golden!(
    mermaid_unsupported_80,
    "tests/fixtures/mermaid/unsupported.md",
    "80"
);
golden!(
    mermaid_azure_graph_40,
    "tests/fixtures/mermaid/azure-graph.md",
    "40"
);
golden!(
    mermaid_azure_graph_120,
    "tests/fixtures/mermaid/azure-graph.md",
    "120"
);

golden_with!(
    mermaid_mode_ascii,
    "tests/fixtures/mermaid/sequence.md",
    "80",
    ["--mermaid", "ascii"]
);
golden_with!(
    mermaid_mode_source,
    "tests/fixtures/mermaid/sequence.md",
    "80",
    ["--mermaid", "source"]
);
golden_with!(
    mermaid_mode_off,
    "tests/fixtures/mermaid/sequence.md",
    "80",
    ["--mermaid", "off"]
);

// Glyphs are characters rather than escapes, so `--color never` still shows them and
// the golden files record them. Every other snapshot stays on the default set,
// which is what proves the default output is unchanged.
golden_with!(
    gfm_alerts_nerd_80,
    "tests/fixtures/gfm/alerts.md",
    "80",
    ["--icons", "nerd"]
);
golden_with!(
    gfm_showcase_nerd_80,
    "tests/fixtures/gfm/showcase.md",
    "80",
    ["--icons", "nerd"]
);
golden_with!(
    azure_attachments_nerd_80,
    "tests/fixtures/azure-devops/attachments.md",
    "80",
    ["--flavor", "azure-devops", "--icons", "nerd"]
);

/// mdvu resolves `--mermaid image` to `unicode` when no image can be drawn,
/// and this output never draws images.
#[test]
fn image_mode_falls_back_to_unicode_without_images() {
    let fixture = "tests/fixtures/mermaid/sequence.md";
    assert_eq!(
        plain_with(fixture, "80", &["--mermaid", "image"]),
        plain(fixture, "80")
    );
}

#[test]
fn ascii_mode_emits_no_wide_drawing_characters() {
    let out = plain_with(
        "tests/fixtures/mermaid/azure-graph.md",
        "80",
        &["--mermaid", "ascii"],
    );
    let diagram: String = out
        .lines()
        .filter(|l| l.starts_with("│ "))
        .map(|l| l.trim_start_matches("│ "))
        .collect();
    assert!(diagram.is_ascii(), "ASCII mode leaked: {diagram}");
}

#[test]
fn a_failed_diagram_keeps_its_source_and_the_rest_of_the_document() {
    let out = plain("tests/fixtures/mermaid/invalid.md", "80");
    assert!(out.contains("graph LR"), "source was dropped");
    assert!(out.contains("Text after the diagram still renders."));
}

#[test]
fn gfm_flavor_leaves_azure_syntax_alone() {
    let out = plain_with(
        "tests/fixtures/azure-devops/showcase.md",
        "80",
        &["--flavor", "gfm"],
    );
    // `_TOC_` is ordinary CommonMark emphasis here, so the macro survives as
    // text rather than being expanded into a table of contents.
    assert!(out.contains("[[TOC]]"), "the macro should not be expanded");
    assert!(!out.contains("Contents"));
    assert!(!out.contains("Child pages unavailable"));
    assert!(!out.contains("[video]"));
}

#[test]
fn plain_output_contains_no_ansi_escapes() {
    for fixture in [
        "tests/fixtures/gfm/showcase.md",
        "tests/fixtures/japanese/showcase.md",
    ] {
        for width in ["40", "80", "120"] {
            assert!(
                !plain(fixture, width).contains('\x1b'),
                "{fixture} at width {width} leaked an escape sequence"
            );
        }
    }
}

/// Mirrors the JIS X 4051 sets in `src/layout/wrap.rs`. `tests/` cannot import
/// them because they are private to the crate.
fn is_no_break_start(c: char) -> bool {
    matches!(c,
        '）' | '〕' | '］' | '｝' | '〉' | '》' | '」' | '』' | '】' | '｠' | '〙' | '〗' | '»'
        | '‐' | '〜' | '！' | '？' | '‼' | '⁇' | '⁈' | '⁉' | '・' | '：' | '；'
        | '。' | '．' | '、' | '，' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ' | '々' | '〻' | 'ー'
        | 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'っ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ' | 'ゕ' | 'ゖ'
        | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ッ' | 'ャ' | 'ュ' | 'ョ' | 'ヮ' | 'ヵ' | 'ヶ'
        | '\u{31F0}'..='\u{31FF}'
        | '｡' | '､' | '｣' | '\u{FF67}'..='\u{FF6F}' | 'ｰ'
    )
}

fn is_no_break_end(c: char) -> bool {
    matches!(
        c,
        '（' | '〔'
            | '［'
            | '｛'
            | '〈'
            | '《'
            | '「'
            | '『'
            | '【'
            | '｟'
            | '〘'
            | '〖'
            | '«'
            | '｢'
    )
}

/// A line may only exceed the requested width when kinsoku left nowhere legal
/// to break (追い込み). In that case the position where the line first runs over
/// budget must itself be a forbidden break point.
fn kinsoku_forbids_breaking_at_the_width(line: &str, width: usize) -> bool {
    use unicode_width::UnicodeWidthChar;
    let mut used = 0usize;
    let mut previous = None;
    for c in line.chars() {
        let next = used + UnicodeWidthChar::width(c).unwrap_or(0);
        if next > width {
            return is_no_break_start(c) || previous.is_some_and(is_no_break_end);
        }
        used = next;
        previous = Some(c);
    }
    false
}

#[test]
fn every_rendered_line_fits_the_requested_width_unless_kinsoku_forbids_it() {
    use unicode_width::UnicodeWidthStr;
    for fixture in [
        "tests/fixtures/gfm/showcase.md",
        "tests/fixtures/gfm/tables.md",
        "tests/fixtures/gfm/nested.md",
        // Reaches the vertical table fallback at narrow widths.
        "tests/fixtures/gfm/wide-table.md",
        "tests/fixtures/japanese/showcase.md",
        "tests/fixtures/japanese/punctuation.md",
        "tests/fixtures/japanese/tables.md",
    ] {
        for width in [40usize, 80, 120] {
            for line in plain(fixture, &width.to_string()).lines() {
                // Code and diagram lines are deliberately not wrapped; the pager
                // scrolls them horizontally instead.
                if line.starts_with('│') && !line.ends_with('│') {
                    continue;
                }
                let measured = UnicodeWidthStr::width(line);
                assert!(
                    measured <= width || kinsoku_forbids_breaking_at_the_width(line, width),
                    "{fixture} at width {width}: {line:?} is {measured} columns \
                     and could have been broken within budget"
                );
            }
        }
    }
}

golden!(gfm_code_80, "tests/fixtures/gfm/code.md", "80");

/// Highlighting must classify tokens without changing a single character, so
/// the plain snapshot above and this one carry the same text.
#[test]
fn gfm_code_highlighted_80() {
    insta::assert_snapshot!(styled("tests/fixtures/gfm/code.md", "80"));
}
