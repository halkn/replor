//! 旧 replor の DESIGN.md 7.3 の Git 実行契約を `std::process::Command` で組む。
//!
//! `demo` は、filter driver・textconv・external diff・fsmonitor を仕込んだ使い捨ての
//! リポジトリで、素の git と契約付きの git を同じ操作で比べ、外部プログラムの起動と
//! index の書き換えが起きるかを記録する。gix で同じ読み取りをした場合も並べる。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 親プロセスから引き継ぐ環境変数（DESIGN.md 7.3 の許可リスト）。
const ALLOWED_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "XDG_CONFIG_HOME",
    "TMPDIR",
    "SYSTEMROOT",
];

/// fetch だけが追加で引き継ぐ環境変数。許可リストだけではプロキシ配下と ssh-agent
/// 経由の fetch ができない（P0 Spike S4 で確認）。
const FETCH_ENV: &[&str] = &[
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "SSH_AUTH_SOCK",
];

const OVERRIDES: &[&str] = &[
    "-c",
    "diff.autoRefreshIndex=false",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "log.showSignature=false",
    "-c",
    "core.quotePath=false",
    "-c",
    "color.ui=false",
];

pub struct Git {
    repo: PathBuf,
}

impl Git {
    pub fn new(repo: &str) -> Self {
        Self {
            repo: PathBuf::from(repo),
        }
    }

    /// 契約を満たす `git` の Command。filter driver の名前は呼び出しのたびに列挙し直す。
    pub fn command(&self, args: &[&str]) -> Command {
        self.command_with_env(args, &[])
    }

    fn command_with_env(&self, args: &[&str], extra_env: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.repo).env_clear();
        for name in ALLOWED_ENV.iter().chain(extra_env) {
            if let Ok(value) = std::env::var(name) {
                cmd.env(name, value);
            }
        }
        cmd.env("LC_ALL", "C")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat");
        let drivers = self.filter_drivers();
        let mut count = 0;
        for name in &drivers {
            for (key, value) in [
                ("clean", ""),
                ("smudge", ""),
                ("process", ""),
                ("required", "false"),
            ] {
                cmd.env(
                    format!("GIT_CONFIG_KEY_{count}"),
                    format!("filter.{name}.{key}"),
                )
                .env(format!("GIT_CONFIG_VALUE_{count}"), value);
                count += 1;
            }
        }
        cmd.env("GIT_CONFIG_COUNT", count.to_string());
        cmd.args([
            "--no-pager",
            "--no-optional-locks",
            "--literal-pathspecs",
            "--no-lazy-fetch",
        ])
        .args(OVERRIDES)
        .args(args);
        cmd
    }

    /// 設定済みの filter driver 名。この呼び出し自体は driver を起動しない。
    fn filter_drivers(&self) -> Vec<String> {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.repo).env_clear();
        for name in ALLOWED_ENV {
            if let Ok(value) = std::env::var(name) {
                cmd.env(name, value);
            }
        }
        let out = cmd
            .args(["config", "--name-only", "-z", "--get-regexp", r"^filter\."])
            .output()
            .expect("spawn git config");
        let mut names: Vec<String> = out
            .stdout
            .split(|b| *b == 0)
            .filter_map(|key| std::str::from_utf8(key).ok())
            .filter_map(|key| {
                let rest = key.strip_prefix("filter.")?;
                let (name, _) = rest.rsplit_once('.')?;
                Some(name.to_string())
            })
            .collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn run(&self, args: &[&str]) -> Vec<u8> {
        let out = self.command(args).output().expect("spawn git");
        if !out.status.success() {
            panic!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        out.stdout
    }

    pub fn run_allow_fail(&self, args: &[&str]) -> Vec<u8> {
        let out = self.command(args).output().expect("spawn git");
        let mut v = out.stdout;
        if !out.status.success() {
            v.extend_from_slice(format!("(exit {})\n", out.status).as_bytes());
        }
        v
    }

    /// PR head を専用の名前空間へ fetch する。FETCH_HEAD・remote-tracking ref・tag を
    /// 書かず、auto gc・maintenance・commit-graph の書き込みも起こさない。
    pub fn fetch(&self, url: &str, refspecs: &[String]) -> String {
        for spec in refspecs {
            let dst = spec.rsplit_once(':').map(|(_, d)| d).unwrap_or("");
            assert!(
                dst.starts_with("refs/replor/"),
                "refspec の書き先は refs/replor/ に限る: {spec}"
            );
        }
        let mut args = vec![
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "-c",
            "fetch.writeCommitGraph=false",
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            "--no-auto-gc",
            "--no-auto-maintenance",
            "--no-write-commit-graph",
            url,
        ];
        args.extend(refspecs.iter().map(String::as_str));
        let out = self
            .command_with_env(&args, FETCH_ENV)
            .output()
            .expect("spawn git fetch");
        format!(
            "exit={}\n--- stdout\n{}--- stderr\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }
}

/// 契約なしの git。比較のための対照で、本実装では使わない。
fn naive(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("spawn git")
}

pub fn demo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let dir = std::fs::canonicalize(dir).unwrap();
    let repo = dir.join("repo");
    let marks = dir.join("marks");
    assert!(!repo.exists(), "{} は空であること", dir.display());
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&marks).unwrap();

    let script = |name: &str, body: &str| -> String {
        let path = dir.join(format!("{name}.sh"));
        std::fs::write(
            &path,
            format!("#!/bin/sh\ntouch '{}/{name}'\n{body}\n", marks.display()),
        )
        .unwrap();
        let mut perm = std::fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
        std::fs::set_permissions(&path, perm).unwrap();
        path.display().to_string()
    };
    let clean = script("clean", "cat");
    let smudge = script("smudge", "cat");
    let textconv = script("textconv", "cat \"$1\"");
    let extdiff = script("extdiff", "true");
    let fsmonitor = script("fsmonitor", "exit 1");

    let setup = |args: &[&str]| {
        let out = naive(&repo, args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let store = dir.join("gitdir");
    setup(&[
        "init",
        "-q",
        "--template=",
        "-b",
        "main",
        &format!("--separate-git-dir={}", store.display()),
    ]);
    setup(&["config", "user.name", "spike"]);
    setup(&["config", "user.email", "spike@example.invalid"]);
    setup(&["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join(".gitattributes"), "*.txt filter=evil diff=evil\n").unwrap();
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    setup(&["add", "."]);
    setup(&["commit", "-q", "-m", "one"]);
    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    setup(&["commit", "-q", "-am", "two"]);
    // ここから仕込む。上の commit で起動した分は数えない。
    setup(&["config", "filter.evil.clean", &clean]);
    setup(&["config", "filter.evil.smudge", &smudge]);
    setup(&["config", "diff.evil.textconv", &textconv]);
    setup(&["config", "diff.external", &extdiff]);
    setup(&["config", "core.fsmonitor", &fsmonitor]);
    // 内容は同じで stat だけ変わったファイル。素の status / diff は index を書き戻す。
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(repo.join("a.txt"), "two\n").unwrap();
    // 内容も変わったファイル。diff が textconv と external diff を呼ぶ。
    std::fs::write(repo.join("b.txt"), "new\n").unwrap();
    setup(&["add", "--intent-to-add", "b.txt"]);
    std::fs::remove_dir_all(&marks).unwrap();
    std::fs::create_dir_all(&marks).unwrap();

    let ops: &[&[&str]] = &[
        &["status", "--porcelain=v2"],
        &["diff"],
        &["diff", "HEAD~1", "HEAD"],
        &["log", "-p", "-1"],
        &["show", "HEAD:a.txt"],
        &["ls-tree", "-r", "HEAD"],
    ];
    let index = dir.join("gitdir/index");
    let contracted = Git::new(repo.to_str().unwrap());
    let contract_args = |op: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = op.iter().map(|s| s.to_string()).collect();
        if matches!(op[0], "diff" | "log" | "show") {
            v.insert(1, "--no-ext-diff".into());
            v.insert(2, "--no-textconv".into());
        }
        v
    };

    println!("| 操作 | 方式 | 起動した外部プログラム | index の書き換え |");
    println!("|---|---|---|---|");
    for op in ops {
        for mode in ["素の git", "契約付き"] {
            let before = std::fs::read(&index).unwrap();
            let before_mtime = std::fs::metadata(&index).unwrap().modified().unwrap();
            match mode {
                "素の git" => {
                    naive(&repo, op);
                }
                _ => {
                    let args = contract_args(op);
                    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                    contracted.command(&refs).output().unwrap();
                }
            }
            let after = std::fs::read(&index).unwrap();
            let after_mtime = std::fs::metadata(&index).unwrap().modified().unwrap();
            let fired = drain(&marks);
            println!(
                "| `git {}` | {mode} | {} | {} |",
                op.join(" "),
                if fired.is_empty() {
                    "なし".to_string()
                } else {
                    fired.join("・")
                },
                if before != after || before_mtime != after_mtime {
                    "あり"
                } else {
                    "なし"
                }
            );
            // 素の git が書き戻した index を、次の比較のために stat だけ変わった状態へ戻す。
            std::fs::write(&index, &before).unwrap();
        }
    }

    // gix: 同じリポジトリを開き、一覧・blob・index を読む。
    let before = std::fs::read(&index).unwrap();
    let repo_gix = gix::open(&repo).unwrap();
    let tree = repo_gix
        .rev_parse_single("HEAD")
        .unwrap()
        .object()
        .unwrap()
        .peel_to_tree()
        .unwrap();
    let mut recorder = gix::traverse::tree::Recorder::default();
    tree.traverse().breadthfirst(&mut recorder).unwrap();
    let mut read = 0;
    for entry in recorder.records.iter().filter(|e| e.mode.is_blob()) {
        read += repo_gix.find_object(entry.oid).unwrap().data.len();
    }
    let index_entries = repo_gix.index_or_empty().unwrap().entries().len();
    let after = std::fs::read(&index).unwrap();
    let fired = drain(&marks);
    println!(
        "| gix: open・rev_parse・tree traverse・find_object（{read} bytes）・index 読み取り（{index_entries} 件） | gix | {} | {} |",
        if fired.is_empty() {
            "なし".to_string()
        } else {
            fired.join("・")
        },
        if before != after { "あり" } else { "なし" }
    );
}

fn drain(marks: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(marks)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in &names {
        std::fs::remove_file(marks.join(name)).unwrap();
    }
    names
}
