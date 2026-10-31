//! S4: 作業ツリーを変えずに、任意のリビジョンのファイル一覧と内容を読む。
//!
//! git CLI は旧 replor の DESIGN.md 7.3 の実行契約で呼ぶ（`contract` module）。
//! 計測対象は git CLI（`ls-tree` ＋ `cat-file --batch`）、gix、作業ツリー直読みの3つ。

mod contract;

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use contract::Git;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| -> &str { args.get(i).map(String::as_str).unwrap_or_else(|| usage()) };
    match arg(0) {
        "state" => print!("{}", state(&Git::new(arg(1)))),
        "bench" => bench(
            Path::new(arg(1)),
            arg(2),
            args.get(3).map_or(5, |n| n.parse().unwrap()),
        ),
        "fetch" => {
            let git = Git::new(arg(1));
            let out = git.fetch(arg(2), &args[3..]);
            print!("{out}");
        }
        "diff" => {
            let git = Git::new(arg(1));
            let out = git.run(&[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "-M",
                "--merge-base",
                arg(2),
                arg(3),
            ]);
            std::io::stdout().write_all(&out).unwrap();
        }
        "shortstat" => {
            let git = Git::new(arg(1));
            let out = git.run(&[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--shortstat",
                "-M",
                "--merge-base",
                arg(2),
                arg(3),
            ]);
            std::io::stdout().write_all(&out).unwrap();
        }
        "contract-demo" => contract::demo(Path::new(arg(1))),
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: s4-revision state <repo>\n\
         \x20      s4-revision bench <repo> <rev> [runs]\n\
         \x20      s4-revision fetch <repo> <url> <refspec>...\n\
         \x20      s4-revision diff <repo> <base> <head>\n\
         \x20      s4-revision shortstat <repo> <base> <head>\n\
         \x20      s4-revision contract-demo <empty-dir>"
    );
    std::process::exit(2)
}

/// 受け入れ条件の比較に使う状態。検証の前後で取り、diff で比べる。
fn state(git: &Git) -> String {
    let text = |args: &[&str]| String::from_utf8_lossy(&git.run_allow_fail(args)).into_owned();
    let index_path = text(&["rev-parse", "--path-format=absolute", "--git-path", "index"]);
    let index_hash = text(&["hash-object", "--no-filters", "--", index_path.trim()]);
    format!(
        "## git status --porcelain=v2 --branch\n{}\
         ## git rev-parse HEAD\n{}\
         ## git symbolic-ref HEAD\n{}\
         ## index hash (git hash-object --no-filters)\n{}\
         ## git for-each-ref\n{}",
        text(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all"
        ]),
        text(&["rev-parse", "HEAD"]),
        text(&["symbolic-ref", "HEAD"]),
        index_hash,
        text(&["for-each-ref", "--format=%(objectname) %(refname)"]),
    )
}

#[derive(Debug)]
struct Entry {
    oid: String,
    path: String,
}

fn bench(repo: &Path, rev: &str, runs: usize) {
    let git = Git::new(repo.to_str().unwrap());
    let entries = ls_tree(&git, rev);
    let bytes_total: usize = cat_file_batch(&git, &entries).0;
    println!(
        "rev={rev} blobs={} bytes={} ({:.1} MiB)",
        entries.len(),
        bytes_total,
        bytes_total as f64 / 1024.0 / 1024.0
    );
    let sample: Vec<&Entry> = entries
        .iter()
        .step_by(entries.len().div_ceil(200))
        .collect();

    let report = |name: &str, f: &mut dyn FnMut() -> usize| {
        let mut times = Vec::new();
        let mut last = 0;
        for _ in 0..runs {
            let start = Instant::now();
            last = f();
            times.push(start.elapsed());
        }
        times.sort();
        println!(
            "{name:<44} median {:>9.1} ms  min {:>9.1} ms  (n={last})",
            ms(times[times.len() / 2]),
            ms(times[0])
        );
    };

    println!("\n# 一覧");
    report("git ls-tree -r -z", &mut || ls_tree(&git, rev).len());
    let gix_repo = gix::open(repo).expect("gix open");
    report("gix rev_parse + tree traverse", &mut || {
        gix_list(&gix_repo, rev).len()
    });
    report("worktree walk (std::fs, .git を除く)", &mut || {
        walk(repo)
    });

    println!("\n# 全 blob の読み取り");
    report("git cat-file --batch（1 プロセス）", &mut || {
        cat_file_batch(&git, &entries).1
    });
    report("gix find_object", &mut || {
        entries
            .iter()
            .map(|e| {
                let oid = gix::ObjectId::from_hex(e.oid.as_bytes()).unwrap();
                gix_repo.find_object(oid).unwrap().data.len()
            })
            .sum::<usize>()
    });
    report("worktree std::fs::read（同じパス）", &mut || {
        entries
            .iter()
            .map(|e| std::fs::read(repo.join(&e.path)).map_or(0, |b| b.len()))
            .sum::<usize>()
    });

    println!("\n# 1 ファイルずつ（{} 件の標本、合計時間）", sample.len());
    report(
        "git cat-file blob（1 ファイル 1 プロセス）",
        &mut || {
            sample
                .iter()
                .map(|e| git.run(&["cat-file", "blob", &e.oid]).len())
                .sum()
        },
    );
    report(
        "git cat-file --batch（常駐 1 プロセス）",
        &mut || {
            let mut batch = Batch::spawn(&git);
            sample.iter().map(|e| batch.read(&e.oid)).sum()
        },
    );
    report("gix find_object", &mut || {
        sample
            .iter()
            .map(|e| {
                let oid = gix::ObjectId::from_hex(e.oid.as_bytes()).unwrap();
                gix_repo.find_object(oid).unwrap().data.len()
            })
            .sum()
    });
    report("worktree std::fs::read", &mut || {
        sample
            .iter()
            .map(|e| std::fs::read(repo.join(&e.path)).map_or(0, |b| b.len()))
            .sum()
    });
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// blob だけを返す。submodule（commit）と symlink は内容を読む対象にしない。
fn ls_tree(git: &Git, rev: &str) -> Vec<Entry> {
    let out = git.run(&["ls-tree", "-r", "-z", "--full-tree", rev]);
    out.split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let record = std::str::from_utf8(record).ok()?;
            let (meta, path) = record.split_once('\t')?;
            let mut meta = meta.split(' ');
            let mode = meta.next()?;
            let kind = meta.next()?;
            let oid = meta.next()?;
            (kind == "blob" && mode != "120000").then(|| Entry {
                oid: oid.to_string(),
                path: path.to_string(),
            })
        })
        .collect()
}

fn gix_list(repo: &gix::Repository, rev: &str) -> Vec<(gix::ObjectId, gix::bstr::BString)> {
    let tree = repo
        .rev_parse_single(rev)
        .unwrap()
        .object()
        .unwrap()
        .peel_to_tree()
        .unwrap();
    let mut recorder = gix::traverse::tree::Recorder::default();
    tree.traverse().breadthfirst(&mut recorder).unwrap();
    recorder
        .records
        .into_iter()
        .filter(|e| e.mode.is_blob())
        .map(|e| (e.oid, e.filepath))
        .collect()
}

fn walk(dir: &Path) -> usize {
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap().flatten() {
            let ty = entry.file_type().unwrap();
            if ty.is_dir() {
                if entry.file_name() != ".git" {
                    stack.push(entry.path());
                }
            } else if ty.is_file() {
                count += 1;
            }
        }
    }
    count
}

/// 返り値は（合計バイト数、読んだ blob の数）。
fn cat_file_batch(git: &Git, entries: &[Entry]) -> (usize, usize) {
    let mut child = git
        .command(&["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let oids: Vec<String> = entries.iter().map(|e| e.oid.clone()).collect();
    // 書き込みと読み出しを別のスレッドにしないと、pipe が詰まって止まる。
    let writer = std::thread::spawn(move || {
        for oid in oids {
            writeln!(stdin, "{oid}").unwrap();
        }
    });
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let (mut bytes, mut count) = (0, 0);
    let mut header = String::new();
    let mut buf = Vec::new();
    for _ in entries {
        header.clear();
        reader.read_line(&mut header).unwrap();
        let size: usize = header
            .trim_end()
            .rsplit(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        buf.resize(size + 1, 0);
        reader.read_exact(&mut buf).unwrap();
        bytes += size;
        count += 1;
    }
    writer.join().unwrap();
    child.wait().unwrap();
    (bytes, count)
}

/// 常駐させた `git cat-file --batch`。UI がファイルを開くたびに 1 件ずつ問い合わせる想定。
struct Batch {
    child: std::process::Child,
    reader: BufReader<std::process::ChildStdout>,
}

impl Batch {
    fn spawn(git: &Git) -> Self {
        let mut child = git
            .command(&["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let reader = BufReader::new(child.stdout.take().unwrap());
        Self { child, reader }
    }

    fn read(&mut self, oid: &str) -> usize {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{oid}").unwrap();
        stdin.flush().unwrap();
        let mut header = String::new();
        self.reader.read_line(&mut header).unwrap();
        let size: usize = header
            .trim_end()
            .rsplit(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let mut buf = vec![0; size + 1];
        self.reader.read_exact(&mut buf).unwrap();
        size
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}
