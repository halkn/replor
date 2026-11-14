# P2 Wiki の設計

P2 の Wiki lens・Wiki root・`.order`・`[[_TOSP_]]`・Wiki の中のリンク解決を決める。Wiki lens・`crates/replor-render/src/wiki/`・Wiki の fixture・azure-devops の flavor でのリンク解決を触る前に読む。Snapshot・content root・`SnapshotPath` の規則は `docs/design/p1-core.md` にあり、この文書はそれを前提に、Wiki で変える点だけを書く。

規則の出どころは、旧 replor（halkn/replor-go）の `docs/plans/initial/DESIGN.md` 8.2 と、Microsoft Learn の次の 3 ページ。

| ページ | ms.date |
|---|---|
| Wiki files, folder structure, Git repo conventions（`project/wiki/wiki-file-structure`） | 2026-02-18 |
| Markdown syntax for files, widgets, and wikis（`project/wiki/markdown-guidance`） | 2026-06-03 |
| Publish a Git repository to a wiki（`project/wiki/publish-repo-to-wiki`） | 2026-02-18 |

## Wiki root

- **Wiki root は、起動したパスから content root まで上にたどり、`.order`（通常のファイル）か `.attachments`（ディレクトリ）を持つ一番上のディレクトリとする。** 見つからなければ Wiki は無く、Wiki lens を出さない。
  - 一番上を採るのは、Wiki の中のどのフォルダで起動しても同じ Wiki root になるようにするため。子ページのフォルダも `.order` を持つ（fixture の `Design/.order`）。
  - content root より上には出ない。入れ子のクローンの中で起動したら、そのクローンの中だけを見る。
  - 探索はしない。起動したパスの下にある Wiki は見つけない。
    - 大きなリポジトリで全体を歩く費用を払わず、`.attachments` だけを持つフォルダを Wiki と取り違えることもない。
    - 代わりに、code wiki をリポジトリの root で起動すると Wiki lens は出ない。code wiki のフォルダ（例: `docs/wiki/`）で起動する。起動の引数が無いときは `docs/` から始まるので（`start.rs`）、`docs/` が Wiki なら引数なしで見つかる。
    - `.git` の無いディレクトリに Wiki のクローンを並べた場合は、親のディレクトリで起動すると Wiki lens は出ない。各クローンの中で起動する。
- 判定は Snapshot の `list` と `resolve` で行う。作業ツリーとリビジョン（P3 の PR head）で同じ結果になる。
  - P3 で PR head を開くときは、作業ツリーで起動したパスと同じ相対パスを起点にする。PR head でそのパスが無ければ、tree の root から判定する。

## flavor

- **Wiki root の下の文書は、設定の `flavor` に関わらず `azure-devops` で描く。** Wiki の外の文書は、設定の `flavor`（既定は `azure-devops`）で描く。
  - Wiki root の下で `gfm` を選ぶ理由が無い。`[[_TOSP_]]` や `::: mermaid` が読めなくなるだけである。
  - 設定の `flavor` の既定は変えない。Azure Repos の README も `[[_TOC_]]` を使うので、Wiki の外でも既定は `azure-devops` のままでよい。
- 文書の中身（`[[_TOC_]]` など）からは判定しない（旧 replor の 8.2 と同じ）。

## 描画コアの `wiki` module

Wiki のページの並びと `[[_TOSP_]]` の中身は、描画コア（`replor-render`）の `wiki` module に置く。

- Snapshot の trait だけで読むので、描画コアの禁止（p1-core.md）に当たらない。
- `[[_TOSP_]]` を描くのは描画コアであり、子ページの一覧を描画コアの外で作ると、P5 の単一ファイルモードでも同じ処理を別に持つことになる。
- ツリーの行の状態（展開・選択）は持たない。それは `replor` の Wiki lens が持つ。

### ページ

```rust
pub struct WikiPage {
    /// 表示名（下の「ページ名」）。
    pub name: String,
    /// `.md` を除いたパス。Wiki の中で一意。
    pub key: SnapshotPath,
    /// 本文 `Page.md`。子ページのフォルダだけのときは無い。
    pub body: Option<SnapshotPath>,
    /// 子ページのフォルダ `Page/`。
    pub children: Option<SnapshotPath>,
    /// `.order` にあるのに、本文もフォルダも無い。
    pub missing: bool,
}

pub struct WikiLevel {
    pub pages: Vec<WikiPage>,
    /// `.order` で 2 度目以降に出た名前の数。
    pub duplicates: usize,
    /// `.order` を読めなかった理由。そのときは `.order` が無いときと同じ並びにする。
    pub order_error: Option<SnapshotError>,
    /// 一覧の上限で切った。
    pub partial: bool,
}

pub fn list_level(snapshot: &dyn Snapshot, folder: &SnapshotPath) -> Result<WikiLevel, SnapshotError>;
```

形を示すもので、名前の細部は実装で変えてよい。次の性質は変えない。

- **`Page.md` と `Page/` を 1 つのページにする。** Microsoft Learn（publish-repo-to-wiki）の「子ページを持つページは、同じ名前の Markdown ファイルとフォルダを同じフォルダに置く」に従う。
- ページになるのは `.md` のファイルとディレクトリだけで、`.attachments` と、それ以外のファイル（画像・`.order` など）はページにしない。
  - 本文の無いフォルダもページにする。Microsoft Learn（publish-repo-to-wiki）は、Markdown を持たないサブフォルダにも親ページを作るとしている。
  - symlink は、Files lens と同じく展開しない。`.md` の symlink は、`resolve` で通常のファイルになるときだけ本文にする。
- 一覧は Snapshot の `list` を 1 階層ずつ使う。上限と `partial` は Files lens と同じ値と規則にする。
  - `partial` のときは、`missing` のページを出さない。切った先にあるだけかもしれないため（旧 replor と同じ）。

### `.order`

- `.order` は 1 行に 1 つのページ名を書く。Microsoft Learn（wiki-file-structure）は「ページ名を書き、`.md` は付けない」「大文字小文字を区別する」としている。
- 読み方:
  - 先頭の BOM を除く。各行の前後の空白と CR を除き、空の行は飛ばす。
  - 名前は、ファイル名から `.md` を除いたもの（`key` の最後の要素）と、大文字小文字を区別して比べる。
  - 同じ名前の 2 度目以降は数えるだけにして（`duplicates`）、最初の位置を採る。
  - 注釈の書き方は無い。docsail は `#` で始まる行を注釈として飛ばしていたが、Microsoft Learn に記述が無いので採らない。ページ名に `#` は使えないので、そうした行は `missing` のページになる。
- 並び:
  1. `.order` に書かれた順。
  1. `.order` に無いページを、名前の UTF-8 のバイト順で後ろに足す。`.order` が無いフォルダは、すべてこの順にする。
  - Microsoft Learn は「アルファベット順」とだけ書き、大文字小文字の扱いを書いていない。Files lens と同じバイト順にする。
- `.order` は 1 MiB まで読む。超えたら `order_error` にする。

### ページ名

- ファイル名から表示名を作る: `.md` を除き、`-` を空白にしてから、`%XX` を 1 度だけ復号する。復号できない名前は、空白にしたままで出す。
  - Microsoft Learn（wiki-file-structure）の規則の逆変換である。ページ名の空白はファイル名で `-` になり、特殊文字は URI 文字列（`:` は `%3A`、`-` は `%2D`）になる。
  - 例: `API-%3A-v2.md` → `API : v2`、`Set%2Dup-Guide.md` → `Set-up Guide`。
- 復号した名前に制御文字があれば、置き換えてから出す。ツリーの行と `[[_TOSP_]]` の項目を壊さないため。

## Wiki lens

- **左ペインの Lens を `w` で Files と Wiki に切り替える。** Wiki が無いときは、`w` は何もせず、ステータスバーにその理由を出す。
  - `w` は今どのキーにも割り当てていない（`app.rs` と `pager/event.rs` で確認）。旧 replor の `w` と同じ。
- **Wiki があるときは、Wiki lens で始める。** 主要ユースケース 1（Wiki のクローンで起動する）では、最初に `.order` の順を見たい。
  - 起動の引数でファイルを指定したときも Wiki lens で始め、そのページを選ぶ。
  - 引数でファイルを指定しないときは、Wiki root の一番目のページを開く。Microsoft Learn（wiki-file-structure）は、root の `.order` の先頭をホームページにするとしている。
- 行はページの 1 つずつで、Files lens と同じく展開したフォルダだけを読む。
  - 子ページを持つページは、展開と折りたたみができる。本文があれば、`Enter` で本文を開く。
  - 本文の無いページは、`Enter` で展開と折りたたみだけをする。
  - `missing` のページは、薄い色で出し、開けない。`duplicates` と `order_error` は、そのフォルダを展開したときにステータスバーに出す。
- **Lens を切り替えても、開いている文書と履歴は変えない。** 切り替えた先の Lens で、開いている文書の行を選ぶ。行が無ければ（Wiki の外の文書、本文以外のファイル）、選択を動かさない。
- リンクをたどって文書を開いたときも、今の Lens で開いた文書の行を選ぶ。そのために、閉じたフォルダは開く。
- `r` は、Files lens と同じく、開いているフォルダと文書を読み直す。`.order` もこのときに読み直す。ファイルの監視はしない。
- Files lens の並びと構造は、Wiki があっても変えない（旧 replor の 8.2）。docsail は Files ツリーを `.order` で並べ替えていたが、ファイルの実際の構造を見る手段が無くなる。

## Wiki の中のリンクと画像

ここで変えるのは、Wiki root の下の文書だけである。Wiki の外の文書は、p1-core.md の規則のままにする。

- **先頭が `/` の参照は、Wiki root を起点にする。** p1-core.md では content root を起点にしていた。
  - リンクと画像の両方に適用する。`/.attachments/x.png` は Wiki root の `.attachments` を指す。
  - project wiki（Wiki 専用のリポジトリ）では Wiki root と content root が一致するので、結果は変わらない。
  - code wiki では、Wiki root は公開したフォルダで、content root はリポジトリの root になる。Microsoft Learn は、code wiki で `/` がどちらを起点にするかを書いていない。公開したフォルダが Wiki の root として見えることから、Wiki root を起点にする（未確認。下の Open Questions の Q1）。
  - 画像を読める範囲（content root の境界）は変えない。Wiki root は content root の下にあるので、Wiki root の起点で作った参照は必ず content root の中にある。
  - そのため、`DocumentSource` に参照の起点（Wiki root か content root）を、content root とは別に持たせる。`SnapshotPath::from_reference` は、content root ではなくこの起点を受ける。
- **リンクのパスは、URI のパスと fragment に分けてから、パスを 1 度だけ復号する。** 境界の判定は、復号した後に行う。
  - `%2E%2E` は `..` として扱い、root の外に出るなら拒否する。`%252E` は `%2E` という文字のまま残る。
  - 今の `navigate::resolve` と同じ順序である。fixture の `escape`・`encoded escape`・`double-encoded` で確かめる。
- **拡張子の無いリンクは、`Page.md` を先に探す。**
  - `Page.md` があれば、それを開く。`Page/` が同時にあっても、`Page.md` を採る（子ページのフォルダは本文ではない）。
  - `Page.md` と、拡張子の無い同じ名前の通常のファイルが両方あれば、曖昧なので開かない。推測で開かない（旧 replor の 8.2）。
  - `Page/` だけがあれば、フォルダなので開けない。理由をステータスバーに出す。
- **パスがどのファイルも指さないとき、Azure がページ名から作るファイル名を 1 度だけ試す。**
  - 復号する前のパスを、次のように変える。
    - 空白（`%20` を含む）は `-` にする。
    - `:` `<` `>` `*` `?` `|` `"` は URI 文字列（`%3A` など）にする。リンクの中にすでにエスケープ（`%3A`）で書いてあれば、ファイル名の表記なのでそのまま残す。
    - `-` は `-` のまま残す。リンクの中の `-` は、空白の表記として扱う。ページ名に `-` を含むページへは、`%2D` で書いたリンクだけが届く。
    - それ以外のエスケープは復号する。
  - 変えた後のパスも、同じ境界の判定と拡張子の規則を通す。無ければ「見つからない」とする。
  - `<` と `>` の URI 文字列は `%3C`・`%3E` とする。Microsoft Learn の表は、この 2 つの列に `&lt;`・`&gt;` を載せているが、ほかの行が URI 文字列であることから、表の書き間違いか描画の崩れと判断した（未確認。Q2）。
- 見出しのアンカーは、Wiki でも GFM の規則（今の `navigation::slug`）で作る。
  - Microsoft Learn（markdown-guidance）は「多くの特殊文字を `-` にする」と書く一方、その例（`Team #1 : Release Wiki!` → `team-1--release-wiki`）は GFM の規則（除去）と一致する。規則として確定できないため、GFM のままにする（旧 replor の 8.2 と同じ）。
- 外部の URL・`javascript:` などの扱いは、今の `navigate::resolve` のまま変えない。

## `[[_TOSP_]]`

- **最初の 1 つを、その位置で、文書の直接の子ページの一覧（題は Child Pages）に置き換える。** 2 つ目以降は、`[[_TOC_]]` と同じく、無視したことを診断として出す。
  - 文書 `Page.md` の子ページは、同じフォルダの `Page/` の `list_level` で取る。並びは Wiki lens と同じにする。
  - `missing` のページは出さない。本文の無いページは、リンクを付けずに名前だけを出す。
  - 各項目のリンクは、子ページの本文への相対リンクにする。ページ名に `:` を含むとリンクが URL の scheme に読まれるので、`./` を付け、`%`・`#`・`?` と制御文字はエスケープする（旧 replor と同じ）。
  - Microsoft Learn（markdown-guidance）は、並び順と深さを書いていない。直接の子だけを、Wiki の順で出す。
- Wiki の外の文書では、今のプレースホルダ（`TOSP_MESSAGE`）のままにする。子ページの規則は Wiki の中にしか無い。
- 子ページの一覧を取れなかったとき（一覧の失敗、上限で切った）は、取れた分を出し、取れなかった理由を同じブロックに書く。文書全体の表示は止めない。
- `document::build` は今、文字列だけから文書を作っている。子ページの一覧は、`[[_TOC_]]` の見出しと同じく、組み立てた後に `wiki` module で埋める。`build` の引数に Snapshot を足さない。

## fixture と検証

- 旧 replor の `testdata/azure-wiki` を、`crates/replor/tests/fixtures/azure-wiki/` にそのまま取り込む。取り込み元は halkn/replor-go の `cd8da066c73c953d8aacf81ae2a127b05a8da0d7` とする。
  - fixture の内容: `.order` の重複（`Home`）・欠損（`Missing-Page`）・記載の無いページ（`Unlisted`・`Folder-Only`）、`Design.md` と `Design/`、本文の無い `Folder-Only/`、`%3A`・`%3F`・`%2D` を含むファイル名、日本語のページ名、`Home.md` の各種のリンク。
  - code wiki 用に、Wiki root と content root が違う fixture を足す（リポジトリの下の `docs/wiki/` に Wiki がある形）。
- テストで確かめること:
  - `list_level` の並び・`missing`・`duplicates`・ページ名（作業ツリーとリビジョンの両方で、`replor-snapshot` の `tests/contract.rs` と同じ組み立てで）。
  - `Home.md` の各リンクの行き先。外部・拒否・見つからないものの理由も含める。
  - `[[_TOSP_]]` の描画。`replor-render` の `tests/render.rs` に snapshot を足す。
  - Wiki root の判定: Wiki の中のフォルダで起動、Wiki root で起動、code wiki のフォルダで起動、Wiki の無いところで起動。
  - Wiki lens の操作: `replor` の `tests/` に、Files lens と同じ形で足す。

## 完了の目安

- 主要ユースケース 1 を、fixture と手元の Azure DevOps Wiki のクローンで行える。
- concept.md の Acceptance Criteria の「旧 replor の `testdata/azure-wiki` の fixture で、`.order` の順序とリンク解決が旧 replor の DESIGN.md 8.2 の規則どおりになる」を満たす。
- docsail をアーカイブできる。docsail の機能（README の「目標」）との対応:

| docsail | replor |
|---|---|
| `.order` の順のツリー、子ページの折りたたみ | Wiki lens（P2） |
| 相対リンク・見出しアンカー・Back / Forward | P1 で実装済み（`f` / `F` / `Enter`、`[` / `]`） |
| `t` の TOC、`f` のファイル検索、`/` の本文検索 | P1 で実装済み（`t`、`Ctrl-p`、`/`） |
| `m` で Mermaid の描画とソースを切り替える | 持たない。文書全体の Source（`s`）と、設定の `mermaid` で代える |
| `g` で Hunk の差分を開く | P4（concept.md の Q8） |
| ファイルの更新に追従する | 持たない。`r` で読み直す。監視は P5 の `--watch` で単一ファイルモードにだけ入れる |

## Open Questions

- Q1: code wiki で、先頭が `/` のリンクと画像は、公開したフォルダと、リポジトリの root のどちらを起点にするか。上では公開したフォルダ（Wiki root）とした。Azure DevOps の code wiki で確かめるスクリプトを `docs/spike/` に足し、結果で決める。
- Q2: ページ名の `<` と `>` が、ファイル名で `%3C`・`%3E` になるか。Q1 と同じスクリプトで確かめる。
- Q3: Wiki の外の文書の flavor の既定を `gfm` に変えるか。Wiki を自動で判定できるようになったので、`azure-devops` を既定にする理由は Azure Repos の README だけになった。
