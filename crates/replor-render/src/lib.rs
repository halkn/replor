//! replor の描画コア。取り込み元と、mdvu から変えた箇所は
//! `docs/design/p1-core.md` の「mdvu の module の配置」にある。

pub mod diagnostic;
pub mod diagram;
pub mod document;
pub mod error;
pub mod flavor;
pub mod image;
pub mod layout;
pub mod markdown;
pub mod output;
pub mod source;
pub mod text;

use std::path::PathBuf;

use diagram::MermaidMode;
use flavor::Flavor;
use layout::LayoutOptions;
use layout::icons::IconSet;
use layout::inline::InlineContext;
use layout::theme::{Theme, Variant};
use output::ColorChoice;

/// `render_to_string` の選択肢。テストを動かす環境に出力が左右されないよう、
/// 環境から決める値も含めてすべて呼び出し側が渡す。
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// 相対リンクと画像の表示先に連結する基準のパス。表示のためだけに使い、
    /// ファイルは開かない。
    pub base_dir: Option<PathBuf>,
    pub width: usize,
    pub flavor: Flavor,
    pub mermaid: MermaidMode,
    pub icons: IconSet,
    pub color: ColorChoice,
    pub variant: Variant,
    pub highlight: bool,
    pub hyperlinks: bool,
}

/// 文書を stdout と同じ経路で描き、出力の文字列を返す。mdvu の `main.rs` の
/// stdout の経路を、画像を出さない場合に限って同じ順で呼ぶ。`text` は decode
/// 済みの文字列で、BOM は呼び出し側が外す（mdvu の `input::decode`）。
pub fn render_to_string(text: String, options: &RenderOptions) -> error::Result<String> {
    let theme = Theme::new(options.variant).with_color(options.color);
    // 画像を出さないので、mdvu の `Cli::mermaid_mode` と同じく `Image` は
    // `Unicode` に落とす。落とさないと PNG 化してからソースを出してしまう。
    let mermaid = match options.mermaid {
        MermaidMode::Image => MermaidMode::Unicode,
        mode => mode,
    };
    let document = document::build(text, options.flavor, mermaid);
    let inline = InlineContext {
        mermaid,
        content_root: None,
        base_dir: options.base_dir.clone(),
        highlight: options.highlight,
        images: None,
        icons: options.icons,
    };
    let prepared = layout::PreparedDocument::new(document, inline);
    let rendered = layout::layout_document(&prepared, LayoutOptions::new(options.width));
    let mut out = Vec::new();
    output::write_document(
        &mut out,
        &rendered,
        options.color,
        &theme,
        options.hyperlinks,
    )?;
    Ok(String::from_utf8(out).expect("the output backends write UTF-8"))
}
