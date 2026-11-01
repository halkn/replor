# P1 Core の設計

P1 の crate の構成、Snapshot の trait、描画コアの snapshot を比べる入口、設定ファイルの形を決める。crate の境界・Snapshot・mdvu の取り込み・`tests/render.rs`・設定ファイルを触る前に読む。制約の出どころは `docs/design/concept.md` の Constraints と `docs/adr/0001-p0-spike.md`。

## crate の構成

Cargo の virtual workspace に、次の 3 つの crate を置く（`crates/<name>/`）。

| crate | 種類 | 持つもの | 依存してよいもの |
|---|---|---|---|
| `replor-render` | lib | 描画コア（mdvu から取り込む部分）と、Snapshot の trait | 下の「描画コアの禁止」に当たらないもの |
| `replor-snapshot` | lib | Snapshot の実装（作業ツリー、gix によるリビジョン） | `replor-render`、gix、ignore |
| `replor` | bin | TUI、pager、設定ファイル、起動の引数 | すべて |

- **trait は描画コアに置き、実装は別の crate に置く。** 描画コアは Snapshot から読む側なので、trait を知る必要がある。一方で、実装の依存（gix）を描画コアに入れてはならない。trait だけの crate を別に作る案は採らない。境界は明確になるが、型の置き場所が増えるだけで、守れる制約は変わらない。
- clap は描画コアに残す。
  - mdvu の `diagram/mod.rs`・`flavor/mod.rs`・`layout/icons.rs`・`layout/theme.rs` は、選択肢の enum に `clap::ValueEnum` を derive している。
  - 外すには取り込むコードを書き換える必要があり、clap は Constraints の禁止にも当たらない。
- 環境変数の読み取りは描画コアに残す（`image/mod.rs` の `Env::detect`、`layout/theme.rs` の `COLORFGBG`）。端末への問い合わせではなく、外部への作用もない。

### 描画コアの禁止

描画コアは、ファイルシステム・Git・TUI・サブプロセス・ネットワークを直接扱わない。ファイルは Snapshot の trait を経由して読む。CI で次の 2 つを検査する。

- **依存の crate の名前を照合する。**
  - 照合する出力は、`cargo tree -p replor-render -e normal --all-features --target all --prefix none`。feature と target で有効になる依存も含めるため、この形にする。
  - `gix`・`ratatui`・`crossterm`・`notify` は前方一致で照合する。`gix-hash` や `ratatui-core` のような分割された crate も拾うため。
  - `ignore`・`replor-snapshot` は完全一致で照合する。
- **呼び出しを clippy で禁止する。**
  - `crates/replor-render/clippy.toml` に `disallowed_methods` と `disallowed_types` を置く。
  - clippy は CI で `-D warnings` を付けて走らせているので、違反は CI で落ちる。clippy はパスを解決して照合するので、`use std::{fs, ..}` のような書き方でも、別名を付けても見逃さない。
  - 禁止するもの:
    - `std::fs` の関数と型
    - `std::path::Path` のファイルシステムに触れるメソッド（`canonicalize`・`exists`・`try_exists`・`metadata`・`symlink_metadata`・`read_dir`・`read_link`・`is_file`・`is_dir`・`is_symlink`）
    - `std::process::Command`
    - `std::net` と `std::os::unix::net` の型
  - テストの fixture を作るコードだけは、`#[cfg(test)]` の中で `#[allow]` を付けて使ってよい。
- 依存の crate が中で行う通信やサブプロセスは、この 2 つでは検出できない。描画コアに依存を足すときは、レビューでその crate が通信もサブプロセスもしないことを確かめる。

## mdvu の module の配置

取り込み元は halkn/mdvu の `ffc324c326a6c1e0139dca2705e55cfa2fabfd2e`（v0.10.2）。この SHA より後の mdvu の修正を取り込むかは、この SHA との差分で判断する。

| mdvu の module | 置き場所 | 理由 |
|---|---|---|
| `markdown/`・`flavor/`・`diagram/`・`layout/`・`source.rs`・`diagnostic.rs`・`document.rs`・`text.rs` | `replor-render` | 描画コアの本体。ratatui に依存しない（`markdown/model.rs`・`layout/mod.rs` の冒頭のコメントで明記されている） |
| `image/` | `replor-render` | 画像の読み取りと、kitty・iTerm2 の escape sequence の組み立て |
| `output/` | `replor-render` | 描画結果を ANSI または plain の文字列にする。snapshot の比較に要る。P5 の stdout 出力でもそのまま使う |
| `error.rs` | `replor-render` | `output/` が使っている。入力の読み取りのエラー（`InputIo` など）は、`input.rs` と一緒に `replor` へ移してよい |
| `pager/`（全体） | `replor` | `app.rs`・`view.rs`・`event.rs`・`mod.rs` が ratatui と crossterm に、`watch.rs` が notify に依存する。`images.rs`・`search.rs`・`search_session.rs`・`state.rs` は依存しないが、`pager/` の中で互いに使い合っているので分けない |
| `cli.rs`・`config.rs`・`input.rs`・`main.rs` | `replor` | 起動・設定・入力。stdin からの入力は P5 で使う |

mdvu の `tests/cli.rs` は CLI の振る舞いのテストなので、P1 では取り込まない。P5 の単一ファイルモードで扱う。

### 取り込むときに描画コアを変える箇所

mdvu のコードをそのまま移し、次の変更だけを加える。ほかの描画のふるまいは変えない。

1. crate 化に要る `pub` と `use` の変更。
1. `ImageSupport::detect` を、セルの大きさを引数で受ける形にする。今は中で `crossterm::terminal::window_size` を呼んでいる。セルの大きさは `replor` が crossterm で取って渡す。
1. snapshot を比べるための関数を足す（下の「snapshot の比較」）。

取り込んだ後の変更は、それぞれ別の PR で行う。

- 文書と画像の読み取りを Snapshot に置き換える（下の「描画コアとの接続」）。
- merman を 0.9 系に上げる。ADR-0001 S2 と Consequences にあるとおり、`diagram/mermaid.rs` の書き換え、`catch_unwind`、エラーの種類による未対応の判定、width profile の選択肢を足す。

## Snapshot の trait

### パス

- Snapshot の中のパスは、専用の型 `SnapshotPath` で表す。
  - root からの相対パスで、区切りは `/`、UTF-8 とする。
  - 空の要素、`.`、`..`、NUL を含まない。
- **Markdown の参照（リンク・画像のパス）から作るときだけ、字句の上で正規化する。**
  - `.` を除き、`..` を一つ前の要素と打ち消す。root より上に出るものは作れない（エラーにする）。
  - 文書からの相対の参照は、文書のあるディレクトリを起点にする。
  - 先頭が `/` の参照は、文書の content root（下の「content root」）を起点にする。mdvu の `confined` と同じで、GitHub と Azure DevOps の解決のしかたに合わせている。
- **ディレクトリのエントリ名から作るときは、正規化しない。** 名前が次のどれかに当たるエントリは、一覧に出さない。
  - 空、`.`、`..`
  - `/` か NUL を含む
  - UTF-8 でない
  - `.git`（大文字小文字を区別しない）
  - 理由: 他人の PR head の tree は信頼できない。細工した名前を正規化すると、別のパスに写ったり、ツリーが循環したりする。作業ツリーの `.git` も、同じ規則で出さない。
- 字句の上で root の中にあっても、読み取りが root の中で終わるとは限らない。途中の要素が symlink のことがあるからだ。境界の判定は、下の「symlink と境界」のとおり、実装が読み取りのたびに行う。

### root と content root

- **Snapshot の root** は、Snapshot が読める範囲の全体である。
  - 作業ツリーでは、起動したパスから上にたどって最初に `.git` があるディレクトリ。見つからなければ、起動したパスそのもの。
  - リビジョンでは、commit の tree の root。
- **content root** は、文書ごとの画像とリンクの境界であり、先頭が `/` の参照の起点でもある。
  - 文書のあるディレクトリから root までの間で、`.git` を持つ一番近いディレクトリとする。無ければ Snapshot の root とする。
  - mdvu の `content_root` と同じ規則で、文書ごとに決める。`.git` の無いディレクトリに Wiki のクローンを並べて起動しても、各 Wiki の `/.attachments/` を読める。入れ子のクローンの下の文書は、入れ子のクローンを境界にする。
  - リビジョンでは、入れ子のリポジトリは submodule なので読まない。content root は常に tree の root になる。
- 起動したパス（例: `docs/`）は、ツリーの初期位置であって境界ではない。
  - Git リポジトリの外では、mdvu は文書のあるディレクトリを境界にしていた。replor は起動したパスを境界にするので、起動したパスより下の別のディレクトリにある画像も読める。ユーザーが自分で開いた範囲なので、広げてよいと判断した。

### 操作

```rust
pub trait Snapshot: Send + Sync {
    /// 1 階層分のエントリ。下の「一覧」の順に並べる。
    fn list(&self, dir: &SnapshotPath) -> Result<Vec<Entry>, SnapshotError>;
    /// symlink をたどり、実体のある通常のファイルのパスを返す。
    fn resolve(&self, path: &SnapshotPath) -> Result<SnapshotPath, SnapshotError>;
    /// 通常のファイルの内容。`limit` を超えたら `TooLarge` を返し、超えた分は読まない。
    fn read(&self, path: &SnapshotPath, limit: u64) -> Result<Vec<u8>, SnapshotError>;
    /// 文書の content root。
    fn content_root(&self, document: &SnapshotPath) -> SnapshotPath;
    /// 表示に使う名前（ステータスバーなど）。作業ツリーならパス、リビジョンなら短い OID を含める。
    fn label(&self) -> String;
}

pub struct Entry {
    pub name: String,
    pub path: SnapshotPath,
    pub kind: EntryKind,
}

pub enum EntryKind { File, Directory, Symlink, Submodule, Other }
```

これは形を示すもので、名前や引数の細部は実装で変えてよい。次の性質は変えない。

### 一覧

- **1 階層ずつ取る。** ツリーは展開したディレクトリだけを読めばよいので、大きなリポジトリで全件の列挙を待たずに済む。gix の tree も 1 階層ずつ読める。
- **並び順は trait の契約とし、各実装が明示的に並べ替える。**
  - ディレクトリを先に、ファイル・symlink・submodule・その他を後に置く。それぞれの中では、名前の UTF-8 のバイト順に並べる。
  - gix は Git の順（ディレクトリを `name/` として比べる）で返す。作業ツリーの `read_dir` の順は OS に依存する。どちらもそのまま返すと、同じ内容でもツリーの並びが変わってしまう。
- **一覧の中身は実装が決め、ビューで絞らない。**
  - 作業ツリーは、ignore crate で `.gitignore` を考慮する。設定は docsail に揃える。隠しファイルは出し、従うのはリポジトリの `.gitignore` だけで、グローバルの ignore と `.git/info/exclude` には従わない。
  - リビジョンは、tree にあるものをすべて出す（ignore 済みのファイルは、そもそも tree に無い）。
  - こうすると、ビューに作業ツリー専用の分岐が要らない（concept.md の Acceptance Criteria）。
- `EntryKind` は lstat（作業ツリー）と tree のエントリの mode（リビジョン）で決め、symlink をたどらない。FIFO・socket・device は `Other` にする。
- `Submodule` を出すのはリビジョンだけで、tree のエントリが gitlink（commit を指すもの）のときに使う。
  - 作業ツリーでは、`.git` を持つディレクトリ（submodule や入れ子のクローン）も `Directory` として出し、中を読める。中のファイルは手元にあるので、読めない理由が無い。
  - 境界は content root の規則で、そのディレクトリに切り替わる。
  - そのため、同じ commit でも submodule の中身は作業ツリーでだけ見える。リビジョンで submodule の中を読むには、submodule の commit を別の Snapshot として開く必要があり、P1 では扱わない。

### symlink と境界

- **symlink は、たどった先が root の中にあり、通常のファイルであるときだけ読む。** パスの最後の要素でも、途中の要素でも同じ規則にする。
- 一覧では `Symlink` として出す。ディレクトリを指していても展開しない（docsail の `follow_links(false)` と同じ）。
- 作業ツリーの判定:
  - `resolve` は、root と結合したパスを毎回 `canonicalize` し、結果が root を `canonicalize` したパスの下にあるか確かめる（mdvu の `confined` と同じ）。
  - 字句の正規化だけでは判定しない。途中のディレクトリが root の外を指す symlink だと、OS がそれをたどって外を読んでしまう。
  - `canonicalize` の後に `metadata` で通常のファイルかを確かめ、そうでなければ `NotAFile` にする。FIFO を開くと、書き手を待って止まるため。
- リビジョンの判定:
  - `resolve` は、パスの要素を先頭から 1 つずつ tree でたどる。
  - 要素が symlink なら、その blob（リンク先のパスの文字列）を読み、symlink のあるディレクトリを起点にして `SnapshotPath` の規則で解決し、続きをたどる。
  - リンク先が絶対パス（先頭が `/`）のものは `OutsideRoot` にする。作業ツリーで `/etc/x.png` を指す symlink は root の外になるので、結果を揃える。Markdown の参照で先頭の `/` を content root の起点にする規則は、symlink のリンク先には使わない。
  - たどった先が blob でなければ（tree・submodule）、`NotAFile` にする。
- 両方の実装で、たどる回数に上限を置き、超えたら `OutsideRoot` と同じ扱いにする。ループする symlink に備えるため。
- **content root の境界は描画コアで確かめる。**
  - 画像は `resolve` の結果が文書の content root の下にあるときだけ読む。
  - Snapshot は root を、描画コアは content root を受け持つ。
- 作業ツリーでは、`resolve` してから `read` するまでの間にファイルが差し替えられうる。これは手元の作業ツリーを読む以上は避けられないので、許容する。リビジョンは不変なので、この問題が無い。
- 次のケースを共通の fixture にし、作業ツリーとリビジョンで同じ結果になることをテストで確かめる。
  - root の中を指す symlink（ファイル、途中のディレクトリ）
  - root の外を指す symlink（相対パス、絶対パス）
  - ループする symlink
  - `..` で root を出る参照
  - 入れ子の `.git`（作業ツリーだけ）
  - FIFO（作業ツリーだけ）

### その他の性質

- **リビジョンの submodule（gitlink）は読まない。** `read` と `resolve` は `Unsupported` を返す。
- **エラーの型は描画コアで定義し、実装の型を出さない。**
  - `NotFound`・`NotAFile`・`OutsideRoot`・`TooLarge { limit }`・`Unsupported`・`Backend(Box<dyn Error + Send + Sync>)` を持つ。
  - gix や `std::io` のエラーは `Backend` に包む。描画コアの型に gix の型を出すと、描画コアが gix に依存してしまう。
- **`Send + Sync` にする。** P3 で構文ハイライトを別スレッドでかける（ADR-0001 S3）ときに、Snapshot を共有できるようにするため。
  - gix の実装は、スレッド間で共有できる repository の handle を持ち、使うスレッドでスレッドローカルの handle に変える。
  - 変換の費用は P0 で測っていない。呼び出しごとに変えて遅ければ、スレッドごとに持ち回る。
- **書き込まない。** どちらの実装も、作業ツリー・index・ref・Git の設定を変えない（AGENTS.md）。
  - gix は object を読むだけにし、filter・textconv を通さない（ADR-0001 S4 で確かめた）。

### 実装

- `WorktreeSnapshot`: root のディレクトリを `std::fs` で読む。
- `RevisionSnapshot`: commit の OID を受け、gix の tree と blob を読む。
  - 版と feature は ADR-0001 S4 のとおり（gix 0.90、`revision`・`max-performance`・`sha1`）。
  - P1 では、OID を受けて読むところまでを作る。ブランチ名や PR からの解決と fetch は P3 で足す。UI からリビジョンを選ぶ手段も P1 では作らない。
  - P1 で作るのは、trait の形が作業ツリーに偏らないことを、P3 より前に確かめるため。

## 描画コアとの接続

- 文書の読み取りと画像の解決は、`Arc<dyn Snapshot>` と文書の `SnapshotPath` を経由させる。
  - 置き換える対象は、mdvu の `input.rs` の `LoadedInput { text, display_name, base_dir }` と、`layout::inline::InlineContext` の `base_dir`・`content_root`。
  - stdin から読んだ文書（P5）は文書のパスを持たないので、mdvu と同じく画像を読まない。
- **表示用の基準のパスは、読み取りの経路と分ける。**
  - mdvu の `layout::inline::display_target` は、相対リンクと画像の表示先に `base_dir` を文字列のまま連結して出している。開くためではなく、表示のためだけの連結である。
  - `InlineContext` には、表示用の基準のパス（文字列）を、`SnapshotPath` とは別に持たせる。表示用のパスは正規化しない。
  - 正規化すると、mdvu の snapshot にある `tests/fixtures/azure-devops/../.attachments/architecture.png` のような表示が変わってしまう。
- `image::load` は、`resolve` と `read` で読む。
  - 拡張子・内容の形式・大きさの検査（`MAX_BYTES`、`dimensions::format_of`）は、今のまま描画コアに残す。
  - 逸脱の拒否は、root について Snapshot の実装が、content root について描画コアが受け持つ。
- 描画コアは `Rc` を使っている（`image::ImageAsset` など）。描画の結果を別スレッドに渡す必要が出るまで、`Arc` には変えない。

## snapshot の比較

### 入口

描画コアに、次を受けて出力の文字列を返す関数を置く。中では、mdvu の `main.rs` の stdout の経路と同じく、`document::build` → `layout::PreparedDocument::new` → `layout::layout_document` → `output::write_document` を呼ぶ。

- 文書の文字列
- 表示用の基準のパス
- 幅・flavor・mermaid・icons・color
- theme の `Variant`
- highlight と hyperlinks の真偽

選択肢は環境から決めず、すべて引数で受ける。mdvu の `tests/common/mod.rs` は、子プロセスの環境から `COLORFGBG`・`NO_COLOR`・`TERM`・`TERM_PROGRAM`・`KITTY_WINDOW_ID`・`KONSOLE_VERSION`・`TMUX` を消して、テストを動かす人の環境から切り離していた。同じプロセスで関数を呼ぶとこれができない。Rust 2024 では `std::env::remove_var` が `unsafe` で、並列に走るテストの中では安全に使えない。

### `tests/render.rs`

`replor-render` の `tests/render.rs` に置き、mdvu の binary の起動（`assert_cmd`）を、上の関数の呼び出しに替える。

- テストの名前、helper の名前（`plain`・`plain_with`・`styled`）、macro の名前、fixture のパス（`tests/fixtures/...`）は変えない。引数の形も変えない。
- `plain_with` が受ける mdvu のフラグ（`--flavor`・`--mermaid`・`--icons`）は、helper の中で `ValueEnum::from_str` で値に写す。
- 引数で受けない値は、mdvu の CLI がテストの環境で決める値に固定する。テストの環境とは、stdout が tty でなく、`--paging never` で、上の環境変数が無い状態。

| 値 | 固定する値 | mdvu で決まる根拠 |
|---|---|---|
| flavor | `azure-devops` | `cli.rs` の `--flavor` の既定。`--flavor` を渡さない golden（`gfm_showcase_80` など）もこれで描かれている |
| mermaid | `unicode` | `--mermaid` の既定 |
| icons | `unicode` | `--icons` の既定 |
| theme | `Variant::Dark` | `COLORFGBG` が無いときの `Variant::resolve` |
| highlight | color が ANSI のときだけ真 | `cli.rs` の `highlight`。`plain` では偽、`styled` では真 |
| hyperlinks | 偽 | color が plain なら偽。ANSI でも、stdout が tty でなければ偽 |
| 画像 | 無し（`inline.images` が `None`） | color が plain なら無し。ANSI でも、`auto` は stdout が tty でなければ無し |
| 表示用の基準のパス | fixture のパスの親（例: `tests/fixtures/azure-devops`） | mdvu の `input.rs` の `path.parent()` |

- 次の 2 つは CLI の色の方針のテストなので、関数を呼ぶ形には移さず、P5 で `tests/cli.rs` と一緒に扱う。
  - `no_color_env_disables_ansi_under_auto`
  - `color_always_emits_ansi_even_when_captured`
- replor の binary に、mdvu と同じ引数を受ける入口を先に作る案は採らない。P5 の CLI の設計（concept.md の Q9）を先取りすることになるため。CLI の配線は比較の対象から外れるが、P5 で `tests/cli.rs` と一緒に扱う。

### 一致の判定

- **比べるのは、`.snap` の header（先頭の `---` で囲まれた部分）の後の本文に限る。**
  - insta は header の `source:` を workspace の root からの相対パスで書く（insta 1.48.0 の `runtime.rs` で確認）。`crates/replor-render/tests/render.rs` に置くと、すべての `.snap` の `source:` が `crates/replor-render/tests/render.rs` に変わる。
  - insta が比べるのも本文だけである。
  - 取り込むときは、mdvu の `.snap` の本文と replor の `.snap` の本文を比べるスクリプトか手順で確かめる。header の差は受け入れる。
- **取り込みと merman の更新は分ける。**
  1. merman 0.7.0 のまま取り込み、本文がすべて一致することを確かめてマージする。
  1. その後の別の PR で 0.9 系に上げ、ADR-0001 S2 の 6 件の差分（`docs/spike/p0/s2-mdvu-snapshot.diff`）だけを受け入れる。
  - 同時に行うと、移植で入った誤りが mermaid の差分に紛れて、そのまま受け入れられてしまう。

## 設定ファイル

### 場所と形式

- 場所は `$XDG_CONFIG_HOME/replor/config.toml`。`XDG_CONFIG_HOME` が無いか空なら、どの OS でも `~/.config/replor/config.toml` とする。dotfiles で 1 つのファイルを持ち回れるようにするため（mdvu と同じ）。
- 環境変数 `REPLOR_CONFIG` で場所を変えられる。空文字列なら設定ファイルを読まない。テストを、動かす機械の設定に依存させないため（mdvu の `MDVU_CONFIG` と同じ）。
  - `REPLOR_CONFIG` で明示したファイルが無ければエラーにする。既定の場所に無いだけならエラーにしない。
- 形式は TOML とする。
- 知らないキーは、警告を出して無視する。
  - 値の誤り（知っているキーに知らない値）はエラーにする。
  - mdvu は知らないキーもエラーにしていた（`deny_unknown_fields`）。replor では、dotfiles で 1 つのファイルを版の違う replor と共有するので、新しい版で足したキーが古い版の起動を止めてしまう。綴りの誤りには警告で気付ける。

### キー

値の綴りは、それぞれの enum の `ValueEnum` から取る（大文字小文字は区別しない）。mdvu の `config.rs` は CLI のフラグの既定値を変える方式だが、P1 の replor にはフラグがほぼ無いので採らない。設定ファイルの値を、そのまま描画の選択肢にする。

| キー | 値 | 既定 | enum の置き場所 | 理由 |
|---|---|---|---|---|
| `images` | `auto`・`kitty`・`iterm2`・`never` | `auto` | `replor`（mdvu の `cli.rs` の `ImagesWhen` を移す） | herdr の中では `auto` が off になるので、`kitty` を選べるようにする（ADR-0001 S1） |
| `mermaid` | `unicode`・`ascii`・`image`・`source`・`off` | `unicode` | `replor-render`（`diagram::MermaidMode`） | mdvu と同じ |
| `mermaid_width` | `unicode`・`cjk` | `unicode` | `replor-render`（新しく足す。merman の width profile に写す） | 曖昧幅を 2 列で描く端末向け（ADR-0001 S2）。merman 0.9 に上げる PR で足す |
| `theme` | `auto`・`dark`・`light` | `auto` | `replor-render`（`layout::theme::ThemeChoice`） | `COLORFGBG` で判定できない端末向け |
| `highlight` | `auto`・`never` | `auto` | `replor`（mdvu の `cli.rs` の `HighlightWhen` を移す） | mdvu と同じ |
| `icons` | `unicode`・`nerd` | `unicode` | `replor-render`（`layout::icons::IconSet`） | mdvu と同じ |
| `flavor` | `gfm`・`azure-devops` | `azure-devops` | `replor-render`（`flavor::Flavor`） | mdvu の既定と同じ。Azure DevOps の方言は GFM に `[[_TOC_]]` などを足したもの。Wiki lens（P2）でワークスペースごとに決められるようにするまでの、全体の既定 |

- mdvu の `color`・`hyperlinks`・`width`・`watch` は、P1 では持たない。どれも stdout への出力か、`--watch`（P5）のための値である。

### 環境変数による上書き

- 設定ファイルのどのキーも、環境変数 `REPLOR_<キーの大文字>`（例: `REPLOR_IMAGES=kitty`）で上書きできる。環境変数の値の誤りはエラーにする。
- 理由: dotfiles で共有する設定ファイルに `images = "kitty"` と書くと、kitty の画像を描けない端末（macOS Terminal、VS Code の端末、ssh 先）でも強制されてしまう。そうなると、kitty の escape sequence が本文に流れる。ADR-0001 S1 が、`TERM_PROGRAM=herdr` を kitty と判定する案を退けた理由と同じ失敗である。
- herdr の利用者は、herdr の中だけで設定される環境変数（herdr の設定や、shell の起動ファイルでの `TERM_PROGRAM=herdr` の判定）で、`REPLOR_IMAGES=kitty` を設定する。設定ファイルの `images` は、すべての端末で同じでよい値にだけ使う。
