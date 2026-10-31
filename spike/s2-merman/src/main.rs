//! S2: merman 0.7.0 と 0.9.3 のテキスト図を、同じ日本語ラベルの入力で並べる。
//!
//! 出力は Markdown で、図ごとに版と width profile の出力例と、端末の幅の想定
//! （曖昧幅を 1 とする端末／2 とする端末）ごとの、つながらない縦の罫線の数を並べる。

use std::fmt::Write as _;

use unicode_width::UnicodeWidthChar;

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let path = std::env::args()
        .nth(1)
        .expect("usage: s2-merman <fixture.md>");
    let text = std::fs::read_to_string(&path).expect("read fixture");
    let mut out = String::new();
    for (title, source) in blocks(&text) {
        writeln!(out, "### {title}\n").unwrap();
        writeln!(out, "```text\n{}```\n", source).unwrap();
        for (label, rendered) in [
            (
                "merman 0.7.0 / unicode charset",
                guarded(|| render07(&source)),
            ),
            (
                "merman 0.9.3 / unicode charset / Unicode profile",
                guarded(|| render09(&source, merman09::ascii::TerminalWidthProfile::Unicode)),
            ),
            (
                "merman 0.9.3 / unicode charset / Cjk profile",
                guarded(|| render09(&source, merman09::ascii::TerminalWidthProfile::Cjk)),
            ),
        ] {
            writeln!(out, "#### {label}\n").unwrap();
            match rendered {
                Ok(text) => {
                    let narrow = broken_verticals(&text, false);
                    let wide = broken_verticals(&text, true);
                    writeln!(
                        out,
                        "つながらない縦の罫線: 曖昧幅=1 の端末で {narrow}、曖昧幅=2 の端末で {wide}\n"
                    )
                    .unwrap();
                    writeln!(out, "```text\n{text}\n```\n").unwrap();
                }
                Err(err) => writeln!(out, "エラー: `{err}`\n").unwrap(),
            }
        }
    }
    print!("{out}");
}

/// Mermaid のブロックを見出しと組で取り出す。GFM の fence と Azure Wiki の `::: mermaid` の両方。
fn blocks(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut heading = String::new();
    let mut current: Option<(String, &str)> = None;
    for line in text.lines() {
        if let Some((body, close)) = current.as_mut() {
            if line.trim() == *close {
                found.push((heading.clone(), std::mem::take(body)));
                current = None;
            } else {
                body.push_str(line);
                body.push('\n');
            }
            continue;
        }
        if let Some(h) = line.strip_prefix("## ") {
            heading = h.to_string();
        } else if line.trim() == "```mermaid" {
            current = Some((String::new(), "```"));
        } else if line.trim() == "::: mermaid" {
            current = Some((String::new(), ":::"));
        }
    }
    found
}

/// 0.7.0 は日本語の ER 属性名で panic するので、panic も結果として記録する。
fn guarded(
    render: impl FnOnce() -> Result<String, String> + std::panic::UnwindSafe,
) -> Result<String, String> {
    std::panic::catch_unwind(render).unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        Err(format!("panic: {message}"))
    })
}

fn render07(source: &str) -> Result<String, String> {
    use merman07::ascii::{AsciiRenderOptions, HeadlessAsciiRenderer};
    HeadlessAsciiRenderer::new()
        .with_ascii_options(AsciiRenderOptions::unicode())
        .render_ascii_sync(source)
        .map_err(|e| e.to_string())
        .and_then(|t| t.ok_or_else(|| "no diagram".to_string()))
}

fn render09(
    source: &str,
    profile: merman09::ascii::TerminalWidthProfile,
) -> Result<String, String> {
    use merman09::ascii::AsciiRenderOptions;
    use merman09::{AsciiRequest, OperationControl, RenderOutput, RenderRequest, Renderer};
    let request = AsciiRequest {
        options: AsciiRenderOptions::unicode().with_terminal_width_profile(profile),
        ..AsciiRequest::default()
    };
    match Renderer::new().render(RenderRequest::ascii(
        source,
        OperationControl::new(),
        request,
    )) {
        Ok(RenderOutput::Ascii(Some(output))) => Ok(output.text),
        Ok(_) => Err("no diagram".to_string()),
        Err(err) => Err(err.to_string()),
    }
}

/// 下へ伸びる罫線のうち、真下の表示列に上からつながる罫線が無いものの数。
///
/// 日本語ラベルの幅を見誤った行では、その行の右側の罫線だけが横にずれ、上下の行の
/// 罫線とつながらなくなる。図種ごとの測り方に依らず、結果の文字列と端末の幅表だけで判定する。
fn broken_verticals(text: &str, ambiguous_wide: bool) -> usize {
    let grid: Vec<Vec<Option<char>>> = text
        .lines()
        .map(|line| columns(line, ambiguous_wide))
        .collect();
    let down = |c: char| {
        matches!(
            c,
            '│' | '┌' | '┐' | '┬' | '├' | '┤' | '┼' | '╭' | '╮' | '|' | '+'
        )
    };
    let up = |c: char| {
        matches!(
            c,
            '│' | '└'
                | '┘'
                | '┴'
                | '├'
                | '┤'
                | '┼'
                | '╰'
                | '╯'
                | '|'
                | '+'
                | '▼'
                | 'v'
                | '▽'
                | '╱'
                | '╲'
                | '/'
                | '\\'
        )
    };
    let mut broken = 0;
    for row in 0..grid.len().saturating_sub(1) {
        let below = &grid[row + 1];
        if below.iter().all(|c| matches!(c, None | Some(' '))) {
            continue;
        }
        for (col, cell) in grid[row].iter().enumerate() {
            let Some(c) = *cell else { continue };
            if !down(c) || cardinality(&grid[row], col) {
                continue;
            }
            let beneath = below.get(col).copied().flatten();
            // ASCII の `+` は上の角にも下の角にも使うので、下が空白なら下の角とみなす。
            if c == '+' && matches!(beneath, None | Some(' ')) {
                continue;
            }
            let joined = beneath.is_some_and(|b| up(b) || matches!(b, 'o' | '{' | '}'));
            if !joined {
                broken += 1;
            }
        }
    }
    broken
}

/// erDiagram の多重度（`||`・`o{`・`|{`）は罫線と同じ文字を使うので数えない。
fn cardinality(row: &[Option<char>], col: usize) -> bool {
    let at = |i: Option<usize>| i.and_then(|i| row.get(i).copied().flatten());
    let (left, here, right) = (at(col.checked_sub(1)), at(Some(col)), at(Some(col + 1)));
    let marker = |c: Option<char>| matches!(c, Some('o' | '{' | '}'));
    here == Some('|') && (marker(left) || marker(right) || left == Some('|') || right == Some('|'))
}

/// 行を表示列ごとのセルに展開する。幅 2 の文字の右半分は `None`。
fn columns(line: &str, ambiguous_wide: bool) -> Vec<Option<char>> {
    let mut cells = Vec::new();
    for c in line.chars() {
        let width = if ambiguous_wide {
            c.width_cjk()
        } else {
            c.width()
        }
        .unwrap_or(0);
        if width == 0 {
            continue;
        }
        cells.push(Some(c));
        for _ in 1..width {
            cells.push(None);
        }
    }
    cells
}
