//! 構文ハイライト。mdvu の `layout/highlight.rs` と同じく syntect を parser として使い、
//! scope を少数の役割に写す。色は役割ごとに固定する（Spike なのでテーマは持たない）。

use std::sync::OnceLock;

use ratatui::style::Color;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

use crate::diff::{FileDiff, Kind};

const RULES: &[(&str, Color)] = &[
    ("comment", Color::DarkGray),
    ("string", Color::Green),
    ("constant.numeric", Color::Magenta),
    ("entity.name.function", Color::Blue),
    ("variable.function", Color::Blue),
    ("support.function", Color::Blue),
    ("entity.name.type", Color::Yellow),
    ("storage.type", Color::Yellow),
    ("support.type", Color::Yellow),
    ("keyword", Color::Red),
    ("storage", Color::Red),
    ("constant", Color::Magenta),
];

pub type Spans = Vec<(String, Option<Color>)>;

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_nonewlines)
}

fn matchers() -> &'static [(Scope, Color)] {
    static M: OnceLock<Vec<(Scope, Color)>> = OnceLock::new();
    M.get_or_init(|| {
        RULES
            .iter()
            .filter_map(|(p, c)| Scope::new(p).ok().map(|s| (s, *c)))
            .collect()
    })
}

/// 行ごとのハイライト。旧側（文脈＋削除）と新側（文脈＋追加）を別々の parse 状態で
/// 順に読む。hunk の間の行は無いので、複数行のコメントや文字列は hunk をまたぐと崩れうる。
pub fn highlight_file(file: &FileDiff) -> Vec<Spans> {
    let set = syntax_set();
    let ext = file.path.rsplit('.').next().unwrap_or("");
    let Some(syntax) = set.find_syntax_by_extension(ext) else {
        return file
            .rows
            .iter()
            .map(|r| vec![(r.text.clone(), None)])
            .collect();
    };
    let mut old = (ParseState::new(syntax), ScopeStack::new());
    let mut new = (ParseState::new(syntax), ScopeStack::new());
    let mut out = Vec::with_capacity(file.rows.len());
    for row in &file.rows {
        let spans = match row.kind {
            Kind::Hunk => vec![(row.text.clone(), Some(Color::Cyan))],
            Kind::Deletion => line(&mut old, &row.text, set),
            Kind::Insertion => line(&mut new, &row.text, set),
            Kind::Context => {
                let _ = line(&mut old, &row.text, set);
                line(&mut new, &row.text, set)
            }
        };
        out.push(spans);
    }
    out
}

fn line(state: &mut (ParseState, ScopeStack), text: &str, set: &SyntaxSet) -> Spans {
    let (parse, stack) = state;
    let Ok(ops) = parse.parse_line(text, set) else {
        return vec![(text.to_string(), None)];
    };
    let mut spans: Spans = Vec::new();
    let mut last = 0;
    for (offset, op) in ops {
        let offset = offset.min(text.len());
        if offset > last {
            push(&mut spans, &text[last..offset], color(stack));
            last = offset;
        }
        if stack.apply(&op).is_err() {
            break;
        }
    }
    if last < text.len() {
        push(&mut spans, &text[last..], color(stack));
    }
    spans
}

fn push(spans: &mut Spans, text: &str, color: Option<Color>) {
    match spans.last_mut() {
        Some((t, c)) if *c == color => t.push_str(text),
        _ => spans.push((text.to_string(), color)),
    }
}

fn color(stack: &ScopeStack) -> Option<Color> {
    for scope in stack.as_slice().iter().rev() {
        for (m, c) in matchers() {
            if m.is_prefix_of(*scope) {
                return Some(*c);
            }
        }
    }
    None
}
