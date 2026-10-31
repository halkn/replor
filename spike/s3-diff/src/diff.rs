//! unified diff の解析と、unified / split の行モデル。端末に依存しない。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Hunk,
    Context,
    Deletion,
    Insertion,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub kind: Kind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub rows: Vec<Row>,
}

impl FileDiff {
    pub fn changed_lines(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| matches!(r.kind, Kind::Deletion | Kind::Insertion))
            .count()
    }
}

/// split 表示の 1 行。片側が空のときは `None`。
#[derive(Debug, Clone, Copy)]
pub struct Pair {
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub hunk: Option<usize>,
}

pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files = Vec::new();
    let mut current: Option<FileDiff> = None;
    let (mut old_no, mut new_no) = (0u32, 0u32);
    let mut in_hunk = false;
    for line in text.split('\n') {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(f) = current.take() {
                files.push(f);
            }
            let path = rest.rsplit_once(" b/").map_or(rest, |(_, b)| b).to_string();
            current = Some(FileDiff {
                path,
                rows: Vec::new(),
            });
            in_hunk = false;
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("@@ ") {
            let (o, n) = hunk_starts(rest);
            old_no = o;
            new_no = n;
            in_hunk = true;
            file.rows.push(Row {
                kind: Kind::Hunk,
                old_no: None,
                new_no: None,
                text: sanitize(line),
            });
            continue;
        }
        if !in_hunk {
            continue;
        }
        let (kind, body) = match line.as_bytes().first() {
            Some(b' ') => (Kind::Context, &line[1..]),
            Some(b'-') => (Kind::Deletion, &line[1..]),
            Some(b'+') => (Kind::Insertion, &line[1..]),
            // `\ No newline at end of file` と、末尾の空行。
            _ => continue,
        };
        let row = match kind {
            Kind::Context => {
                old_no += 1;
                new_no += 1;
                Row {
                    kind,
                    old_no: Some(old_no - 1),
                    new_no: Some(new_no - 1),
                    text: sanitize(body),
                }
            }
            Kind::Deletion => {
                old_no += 1;
                Row {
                    kind,
                    old_no: Some(old_no - 1),
                    new_no: None,
                    text: sanitize(body),
                }
            }
            Kind::Insertion => {
                new_no += 1;
                Row {
                    kind,
                    old_no: None,
                    new_no: Some(new_no - 1),
                    text: sanitize(body),
                }
            }
            Kind::Hunk => unreachable!(),
        };
        file.rows.push(row);
    }
    if let Some(f) = current {
        files.push(f);
    }
    files
}

fn hunk_starts(rest: &str) -> (u32, u32) {
    let mut parts = rest.split(' ');
    let num = |s: Option<&str>| -> u32 {
        s.and_then(|s| s[1..].split(',').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(1)
    };
    let old = num(parts.next());
    let new = num(parts.next());
    (old, new)
}

/// タブを空白に展開し、端末制御文字を可視化する。diff の本文は入力由来なので、
/// そのまま端末に流さない。
fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut col = 0;
    for c in text.chars() {
        match c {
            '\t' => {
                let n = 4 - col % 4;
                out.extend(std::iter::repeat_n(' ', n));
                col += n;
            }
            '\r' => {}
            c if c.is_control() => {
                out.push('\u{FFFD}');
                col += 1;
            }
            c => {
                out.push(c);
                col += 1;
            }
        }
    }
    out
}

/// 連続する削除と追加を、先頭から順に向かい合わせる。
pub fn pairs(rows: &[Row]) -> Vec<Pair> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        match rows[i].kind {
            Kind::Hunk => {
                out.push(Pair {
                    old: None,
                    new: None,
                    hunk: Some(i),
                });
                i += 1;
            }
            Kind::Context => {
                out.push(Pair {
                    old: Some(i),
                    new: Some(i),
                    hunk: None,
                });
                i += 1;
            }
            Kind::Deletion | Kind::Insertion => {
                let mut dels = Vec::new();
                let mut inss = Vec::new();
                while i < rows.len() && rows[i].kind == Kind::Deletion {
                    dels.push(i);
                    i += 1;
                }
                while i < rows.len() && rows[i].kind == Kind::Insertion {
                    inss.push(i);
                    i += 1;
                }
                for k in 0..dels.len().max(inss.len()) {
                    out.push(Pair {
                        old: dels.get(k).copied(),
                        new: inss.get(k).copied(),
                        hunk: None,
                    });
                }
            }
        }
    }
    out
}
