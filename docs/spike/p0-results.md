# P0 Spike の結果

`docs/design/spike-p0.md` の S1〜S4 の検証結果と根拠。結論と P1 以降への影響は `docs/adr/0001-p0-spike.md` に置く。証跡の生データは `docs/spike/p0/` にある。

## 計測環境

- macOS（Darwin 27.0.0、arm64）、rustc 1.99.1、git 2.54.0（Apple Git-157）、release ビルド
- エージェントの計測は Claude Code の sandbox（macOS Seatbelt）の中で動かした。ファイルの open とプロセスの起動は sandbox の分だけ遅く出ている可能性がある。どれもキャッシュが温まった状態の値
- 依存の版: merman 0.7.0 / 0.9.3、ratatui 0.30.2、crossterm 0.29.0、syntect 5.3.0、gix 0.90.0、ratatui-image 11.1.0
- 検証に使ったコードは `spike/`（S ごとの crate）で、ADR-0001 の Accepted 後に削除した。コミット `151d461` に残っている。fixture は `docs/spike/p0/fixtures/`、再確認用のスクリプトは `docs/spike/p0/scripts/` に移した

## 一覧

| 項目 | 結果 | 根拠の要約 |
|---|---|---|
| S1 画像表示 | 成立（macOS の 2 環境）／未実施（WSL の 4 環境。環境が無い） | Ghostty は `auto` で、その中の herdr は `--images kitty` の強制で、3 図とも日本語ラベルまで表示でき、スクロールの残骸もない（[S1](#s1-普段の環境で画像を表示できるか)）。準備中に、mdvu 0.10.2 が日本語の属性名を持つ erDiagram で panic し、文書全体を表示できないことが分かった |
| S2 merman 0.9 のテキスト図 | 条件付き成立 | `Unicode` profile なら、曖昧幅を 1 列で描く端末で日本語の枠が揃う。ただし戻りの辺を持つ flowchart をテキスト図にできない。erDiagram の属性は `type(bytes=6)="string"` の形式で出る |
| S3 Diff の自前描画 | 成立 | PR #39773（80 ファイル・15,720 変更行）では、全行を組み立てる方式でも 1 フレーム 1 ms 未満。5 万行の単一ファイルでは全行方式が 23 ms／フレーム、ハイライトが 4.9 s かかる |
| S4 リビジョンからの読み取り | 成立（GitHub）／未実施（Azure DevOps。アクセスできる環境が無い） | 契約付きの git CLI は外部プログラムを起動せず、index を書かない。gix も読み取りだけなら同じで、git CLI より速い。fetch の前後の差分は `refs/replor/` だけ |

## S1: 普段の環境で画像を表示できるか

macOS ＋ Ghostty とその中の herdr は成立（herdr は `--images kitty` の強制が要る）。WSL2 ＋ Noctty・WSL2 ＋ WezTerm とそれぞれの中の herdr は、検証できる環境が手元に無いため未実施とする。WSL の環境が使えるようになった時点で、同じスクリプトで確認する。

### 準備したもの

- `docs/spike/p0/scripts/check-images.sh <環境名>`
  - 環境変数（`TERM`・`TERM_PROGRAM`・`KITTY_WINDOW_ID`・`TMUX`・`WSLENV`・`HERDR*` など）を記録する
  - mdvu 0.10.2 の `protocol_from_env` と同じ判定で、`auto` が選ぶ protocol を推定する
  - mdvu を `--mermaid image --images auto` → `kitty` → `iterm2` の順に起動し、表示の可否を対話で記録する
  - 結果は `s1-<環境名>.md` に書く
- `docs/spike/p0/fixtures/mermaid-ja.md`: 日本語ラベルの flowchart・sequenceDiagram・erDiagram、Azure Wiki 形式の `::: mermaid`、曖昧幅の文字のラベル
- `docs/spike/p0/fixtures/long-ja.md`: スクロールで図と画像（`img/gradient.png`）が上下端をまたぐ配置の長い文書
- `spike/s1-picker`（削除済み）: mdvu の方式で表示できない環境があったときの比較用。docsail と同じ `Picker::from_query_stdio` で protocol を選び、問い合わせにかかった時間を表示する

### 分かったこと

- herdr の中では、herdr が `TERM_PROGRAM=herdr`・`TERM=xterm-256color` を設定し、外側の端末の `TERM_PROGRAM=ghostty` を隠す。mdvu の `protocol_from_env` はどれにも当たらないので、`auto` は画像を出さない。`experimental.kitty_graphics = true` のとき、`--images kitty` で強制すれば kitty の配置・スクロール時の切り出し・終了時の解放まで問題なく動いた。
- herdr の `experimental.kitty_graphics` が有効かは環境変数から分からない（`HERDR_*` には出ない）。`TERM_PROGRAM=herdr` を kitty と判定すると、無効のときに escape sequence がそのまま流れうる（未確認）。

- mdvu 0.10.2（merman 0.7.0）は、erDiagram の属性名が非 ASCII（例: `string 氏名`）だと merman-core の `er.rs:851` で panic し、文書全体を表示できない（`--mermaid unicode` でも起きる）。エンティティ名と関係のラベルが日本語なだけなら起きない。S1 の fixture は属性名を ASCII にし、日本語は属性のコメントに置いた。panic する例は `docs/spike/p0/fixtures/mermaid-ja-extra.md` に残した。
- 描画コアは merman を `catch_unwind` の内側で呼び、1 つの図の失敗で文書全体を止めないようにする必要がある（概念設計の Acceptance Criteria「文書全体の表示は止まらない」）。

### 記入表

| 環境 | `auto` が選んだ protocol | `auto` | `kitty` | `iterm2` | スクロール・切替の残骸 | 備考 |
|---|---|---|---|---|---|---|
| macOS ＋ Ghostty（1.3.1） | kitty | 可 | 可 | 不可 | なし（auto・kitty） | `TERM=xterm-ghostty`・`TERM_PROGRAM=ghostty`。3 図とも日本語ラベルまで表示。記録は `docs/spike/p0/s1/s1-ghostty.md`。長い文書のスクロール確認は、スクリプトの不具合（bash 3.2 で `$best）` を別の変数と読む）で記録が落ちたため、`mdvu --mermaid image --images kitty` で別に確認した（残骸なし） |
| 上記の中の herdr（herdr 0.9.5、`experimental.kitty_graphics = true`） | off（判定できない） | 不可 | 可 | 不可 | なし（auto・kitty） | `TERM=xterm-256color`・`TERM_PROGRAM=herdr`・`HERDR_ENV=1`。mdvu の判定に該当せず `auto` は off。`--images kitty` で 3 図とも日本語ラベルまで表示。iterm2 は出ない。記録は `docs/spike/p0/s1/s1-ghostty-herdr.md`。長い文書のスクロールでも残骸なし（ユーザーの報告） |
| WSL2 ＋ Noctty | 未実施 | | | | | 環境が無い。確認すること: `TERM_PROGRAM` が `ghostty` か、renderer が OpenGL か（Open Questions Q1） |
| 上記の中の herdr | 未実施 | | | | | 環境が無い |
| WSL2 ＋ WezTerm | 未実施 | | | | | 環境が無い。確認すること: `TERM_PROGRAM` が WSL 側に渡るか |
| 上記の中の herdr | 未実施 | | | | | 環境が無い |

## S2: merman 0.9 のテキスト図で日本語の枠が揃うか

条件付き成立。

### 枠が揃うか

`spike/s2-merman` で、同じ入力を merman 0.7.0（unicode charset）と 0.9.3（unicode charset、`Unicode` / `Cjk` profile）に描かせた。出力例は `docs/spike/p0/s2-merman-output.md`。

枠が揃うかは目視に加えて機械的にも調べた。端末の幅表で各行を表示列に展開し、下へ伸びる罫線（`│` `┌` `┬` など）の真下に、上からつながる罫線が無いものを数えた。曖昧幅（East Asian Ambiguous）を 1 列で描く端末と 2 列で描く端末を、それぞれ `unicode-width` 0.2 の `width` と `width_cjk` で模した。0 なら揃っている。

| 図 | 0.7.0 | 0.9.3 `Unicode` | 0.9.3 `Cjk` |
|---|---|---|---|
| flowchart（戻りの辺あり） | 18 / 30 | テキスト図にできない | テキスト図にできない |
| flowchart（戻りの辺なし） | 17 / 30 | **0** / 27 | 1 / 1 |
| sequenceDiagram | 14 / 20 | **0** / 16 | 0 / **0** |
| erDiagram（属性名は ASCII、コメントが日本語） | 2 / 10 | **0** / 8 | 0 / **0** |
| erDiagram（日本語の属性名） | panic | **0** / 6 | 0 / **0** |
| Azure Wiki 形式（`::: mermaid`） | 9 / 20 | **0** / 18 | 0 / **0** |
| 曖昧幅の文字（`○` `①` `→`）のラベル | 6 / 12 | **0** / 12 | 6 / **0** |

表の値は「曖昧幅を 1 列で描く端末 / 2 列で描く端末」でのつながらない罫線の数。`Cjk` の flowchart の 1 は、辺のラベルが縦線に重なる箇所の誤検出で、目視では揃っている。

読み取れること:

- 0.9.3 の `Unicode` profile は、曖昧幅を 1 列で描く端末で日本語の枠が揃う。0.7.0 は同じ端末でもずれる。
- `Cjk` profile は、曖昧幅を 2 列で描く端末で揃う。罫線が曖昧幅の文字なので、`Cjk` では構造を ASCII（`+` `-` `|`）に落とす設計になっている（merman-ascii `options.rs` の `structural_charset`）。
- 曖昧幅の文字を 1 列と 2 列のどちらで描くかは端末の設定で決まる。どちらの profile を選ぶかは、端末ごとの設定に合わせる必要がある（[ユーザーによる確認](#ユーザーによる確認2026-10-31macos)）。

### 0.9.3 に上げて新しく出る問題

- 戻りの辺（ループ）を持つ flowchart をテキスト図にできない。エラーは「ASCII rendering does not support `routes crossing reserved graph geometry` for `flowchart` yet」。英語のラベルでも同じで、0.7.0 は（ずれながらも）描けていた。画像の経路（SVG）では描ける。
- erDiagram の属性が `type(bytes=6)="string" name(bytes=6)="氏名" keys=[] comment(bytes=0)=""` の形式で出る。merman のテスト（`merman-ascii/tests/er_model.rs`）がこの形式を期待しているので、不具合ではなく仕様。0.7.0 の `string name 氏名` より読みにくい。
- PNG の経路を、同じ fixture で 0.7.0（`docs/spike/p0/s2-png-0.7.0/`）と 0.9.3（`docs/spike/p0/s2-png-0.9.3/`）で比べた。どちらも mdvu の `diagram/raster.rs` を通している。
  - erDiagram の属性名の左端が欠ける（`name` が `ıame` に見える）。0.7.0 では欠けないので、0.9.3 での退行。
  - sequenceDiagram のラベル「承認 ✅」が、丸ごと豆腐（□）で描かれる。0.7.0 でも同じで、版を上げたことによる退行ではない。絵文字を含むテキストの run 全体に、CJK を持たないフォントが当たっているとみられる（未確認）。mdvu の raster 側のフォントの選び方の問題として、P1 で別に扱う。
  - 既定の配色が変わる（0.7.0 の紫の枠から、0.9.3 は黒の枠と、図種ごとに色の付いた entity・participant へ）。日本語ラベルの読みやすさはどちらも同等。

### mdvu を 0.9.3 に上げたときの差分

mdvu（`ffc324c`、v0.10.2）を scratchpad に複製し、merman を `=0.9.3`（`features = ["ascii", "svg", "all-diagrams"]`）に上げた。手元の mdvu の clone には変更を加えていない。

API の変更で直した箇所は `src/diagram/mermaid.rs` だけだった。

- `HeadlessAsciiRenderer::render_ascii_sync` を `Renderer::render(RenderRequest::ascii(..))` に置き換えた。`AsciiRenderOptions` に `with_terminal_width_profile(TerminalWidthProfile::Unicode)` を足した。
- `HeadlessRenderer::render_svg_resvg_safe_sync` を、`SvgRequest { pipeline: Some(SvgPipeline::resvg_safe()), .. }` を渡す `Renderer::render(RenderRequest::svg(..))` に置き換えた。`diagram/raster.rs`（href を解決しない resolver）はそのまま使える。
- 0.9 では、描く図種を `diagram-*` の feature で選ぶ（既定の feature は ELK を含む SVG 一式）。`render` feature は `svg` に改名された。

テストの結果:

- `tests/render.rs` の snapshot は 52 件中 6 件が変わった（差分は `docs/spike/p0/s2-mdvu-snapshot.diff`）。
  - `mermaid_japanese_80`: 日本語ラベルの枠が揃う。
  - `mermaid_sequence_80`・`mermaid_mode_ascii`: メッセージのラベルが枠の外にはみ出していたのが直り、図の幅が広がる。
  - `mermaid_state_80`・`mermaid_unsupported_80`: state 図がソース表示からテキスト図になる。
  - `mermaid_invalid_80`: パースエラーの文言が変わる。
- 単体テストは 383 件中 3 件が失敗した。
  - `an_unsupported_diagram_family_falls_back_to_source`: state 図が未対応でなくなったため。別の未対応の図種に替える必要がある。
  - `pager::watch` の 2 件: merman と関係のないコードで、sandbox の中でファイル監視が届かないためと見ている（未確認）。
- 戻りの辺を持つ flowchart は、「Mermaid diagram not supported as text」ではなく「Mermaid render error」として出る。mdvu の `is_unsupported` がエラー文字列の `does not support diagram type` で判定しているためで、0.9 のエラー型（`UnsupportedFeature`）で判定し直す必要がある。
- `--mermaid image` の経路は 0.9.3 でも動く。fixture の 5 図を PNG にでき、1 図 10〜91 ms だった（初回は system font の走査を含む）。

## S3: Diff を自前で描いたときの速度

成立。

### 構成

`spike/s3-diff` は、`git diff --merge-base` の出力を解析して unified と split の行モデルを作り、Ratatui で描く。

- ハイライトは mdvu の `layout/highlight.rs` と同じく、syntect を parser として使い、scope を役割の色に写す。
- 描き方は2方式ある。
  - full: 全行を組み立て、`Paragraph::scroll` で切り出す
  - window: 見えている行だけを組み立てる
- `bench` は `TestBackend`（200×50）での描画時間を測る。各ファイルを 1 行ずつ 100 回送り、続けて 1 画面ずつ末尾まで送る。端末への書き出しは含まない。

### 計測

| 入力 | 規模 | 方式 | フレーム p50 / p95 / max |
|---|---|---|---|
| neovim PR #39773（head `c08901e`、base は `origin/master` `cc917b9` との merge-base） | 80 ファイル、追加 2,655・削除 13,065、最大 3,959 行のファイル | unified full | 0.17 / 0.63 / 0.95 ms |
| 同上 | 同上 | unified window | 0.17 / 0.21 / 0.25 ms |
| 同上 | 同上 | split full | 0.19 / 0.56 / 0.86 ms |
| 同上 | 同上 | split window | 0.19 / 0.25 / 0.30 ms |
| 合成した 1 ファイル（C、5 万行の追加） | 50,001 行 | unified full | 22.8 / 23.7 / 28.0 ms |
| 同上 | 同上 | unified window | 0.24 / 0.26 / 0.28 ms |
| 同上 | 同上 | split full | 21.1 / 21.8 / 22.3 ms |
| 同上 | 同上 | split window | 0.25 / 0.26 / 0.29 ms |

ファイルを初めて開くときのハイライトの時間:

- PR #39773: p50 0.6 ms、p95 36 ms、最大 122 ms（最大は SyntaxSet の読み込みを含む最初のファイル）
- 5 万行のファイル: 4.9 s（約 0.1 ms／行）。この間、画面は固まる

生データは `docs/spike/p0/s3-bench-*.txt`。

読み取れること:

- 数千行規模の PR では、どちらの方式でも固まらない。判定基準の「固まる」には当たらない。
- full 方式の費用はファイルの行数に比例する（約 0.45 µs／行）。数万行のファイルでは 1 フレーム 20 ms を超える。window 方式は行数に依らない。
- 先に詰まるのは描画ではなくハイライトである。ファイルを開くたびに全行を同期でハイライトすると、1 万行で約 1 秒止まる（5 万行の実測からの推定）。herdr-reviewr は 50,000 行・2 MB を超えるファイルを Diff にせず、通知だけ出す（`diff.rs` の `MAX_LINES`・`MAX_BYTES`）。

### 日本語の表示幅

`.work/ja.diff`（日本語の Markdown の変更）を PR の diff の前に足した `s3-input.diff` を、`tui` で目視する（[ユーザーによる確認](#ユーザーによる確認2026-10-31macos)）。タブは 4 列に展開し、制御文字は U+FFFD に置き換えている。Ratatui は全角を 2 列として扱い、split の各半分で幅を超えた分を切り詰める。

### herdr-reviewr の Diff の構成（参考にできる単位）

| モジュール | 役割 | replor での扱い |
|---|---|---|
| `diff.rs` | 端末に依存しない行モデル（`Row::{Context, Deletion, Insertion, Fold, Rendered}`）。旧新の全文を `similar` で比べて行を作り、全文をハイライトしてから行に割り当てる。単語単位の強調（`compute_emphasis`）、変更のない区間の折りたたみ（`collapse_context`）、内容ハッシュをキーにした `DiffCache`、50,000 行・2 MB の上限 | 参考にする単位。git の unified diff ではなく旧新の blob から Diff を作るので、S4 の Snapshot（gix で blob を読む）と組み合わせられ、`git diff` の実行契約も要らない |
| `split.rs` | `Pair::{Same, Old, New, Both}` で旧新を向かい合わせる。変更のまとまりの中で似た行を最長増加部分列で対応させる | split の行の対応付けとして参考にする（spike は先頭から順に対応させただけ） |
| `highlight.rs` | syntect と two-face の埋め込み済み syntax・theme | ハイライトの役割の写像は mdvu の方式に寄せる |
| `ui.rs`・`app.rs` | 見えている行（`Visible`）だけを描き、スクロール位置はカーソル移動でだけ動かす | window 方式と Continuity 不変条件の参考 |

## S4: リビジョンからの読み取り

GitHub は成立。Azure DevOps は、アクセスできる環境が無いため未実施。

### 対象

neovim を GitHub から新しく clone した使い捨ての clone（`spike/.work/neovim`）を使った。手元の `~/repos/github.com/neovim/neovim` は読んでいない。対象の HEAD は `cc917b9`（master）で、blob は 3,943 個・59.9 MiB。

### 速度

`spike/s4-revision bench`。5 回の中央値で、生データは `docs/spike/p0/s4-bench-neovim-head.txt`。git CLI はどの呼び出しにも実行契約を付けた。filter driver を列挙するための `git config` の起動が 1 回ずつ含まれる。

| 操作 | git CLI | gix | 作業ツリー直読み |
|---|---|---|---|
| 一覧（3,943 件） | `ls-tree -r -z`: 20.5 ms | tree の走査: 0.8 ms | ディレクトリの走査: 9.4 ms |
| 全 blob の読み取り（59.9 MiB） | `cat-file --batch`（1 プロセス）: 106 ms | `find_object`: 78 ms | `std::fs::read`: 144 ms |
| 198 ファイルを 1 つずつ | `cat-file blob` を毎回起動: 3,390 ms／常駐 `cat-file --batch`: 24 ms | 3.6 ms | 6.9 ms |

- 1 ファイルずつプロセスを起動する方式は使えない（1 ファイル約 17 ms）。git CLI で読むなら `cat-file --batch` を常駐させる。
- gix は、一覧でも 1 ファイルの読み取りでも git CLI より速く、作業ツリー直読みと同等以上。

### Git 実行契約を満たせるか

旧 replor の DESIGN.md 7.3 の契約を `std::process::Command` で組んだ（`spike/s4-revision/src/contract.rs`）。

- 環境変数は許可リストに絞る。
- `GIT_OPTIONAL_LOCKS=0` と `--no-optional-locks` を付ける。
- `-c` で `diff.autoRefreshIndex`・`core.fsmonitor`・`log.showSignature`・`core.quotePath`・`color.ui` を上書きする。
- filter driver は呼び出しのたびに `GIT_CONFIG_COUNT` で無効にする。
- diff 系の呼び出しには `--no-ext-diff --no-textconv` を付ける。
- `--literal-pathspecs` と `--no-lazy-fetch` を付ける。

試験用のリポジトリを `spike/s4-revision contract-demo` で作り、外部プログラムを仕込んだ。filter driver（clean / smudge）、textconv、`diff.external`、`core.fsmonitor` に、起動されると印のファイルを残すスクリプトを登録した。そのうえで、内容は同じで stat だけ変わったファイルと、内容が変わったファイルを置き、同じ操作を素の git と契約付きの git で比べた（`docs/spike/p0/s4-contract-demo.md`）。

| 操作 | 素の git | 契約付き |
|---|---|---|
| `status --porcelain=v2` | clean・fsmonitor を起動、index を書き戻す | なし |
| `diff` | clean・external diff・fsmonitor を起動、index を書き戻す | なし |
| `diff HEAD~1 HEAD` | external diff・smudge を起動 | なし |
| `log -p -1` | smudge・textconv を起動 | なし |
| `show HEAD:a.txt`・`ls-tree -r HEAD` | なし | なし |
| gix: open・`rev_parse_single`・tree の走査・`find_object`・index の読み取り | — | なし（gix） |

gix を使う場合に、同じ不変条件を保てるかは次のとおり。

- object の読み取り（tree の走査、`find_object`）は生の object を返し、filter も textconv も通さない。index を読むだけなら書き換えない（上の表で確認）。
- gix の blob diff（`gix-diff` の `blob::Pipeline`）は、既定の `Mode::ToWorktreeAndBinaryToText` で filter と textconv の driver を起動する。使うなら `Mode::ToGit` を明示する必要がある（gix-diff 0.70.0 の `src/blob/pipeline.rs` で確認）。
- 作業ツリーと index の比較（status）は、gix でも内容を比べるために clean filter を通す（gix-filter の driver が起動する）。P4 の Local changes で gix の status を使うなら、filter を無効にした設定で開く必要がある。今回の spike の feature 構成（`revision` だけ）には含めておらず、実測していない。

### PR head の fetch

- GitHub の PR の ref は `refs/pull/<N>/head`（PR ごとにある）と `refs/pull/<N>/merge`（merge 可能な open の PR にだけある）。neovim の `git ls-remote` で確認した。
- 契約付きの fetch で、22 件の PR head を `+refs/pull/<N>/head:refs/replor/pr/<N>/head` へ取得した（`docs/spike/p0/s4-github-fetch.txt`）。次の設定で、FETCH_HEAD・remote-tracking ref・tag・commit-graph を書かず、auto gc と maintenance も起こさない。
  - 引数: `--no-tags --no-write-fetch-head --no-recurse-submodules --no-auto-gc --no-auto-maintenance --no-write-commit-graph`
  - 設定: `-c gc.auto=0 -c maintenance.auto=false -c fetch.writeCommitGraph=false`
  - fetch 先は remote 名ではなく URL を渡す
- fetch の前後を比べた（`docs/spike/p0/s4-github-state.diff`）。`git status --porcelain=v2 --branch --untracked-files=all`・`git rev-parse HEAD`・`git symbolic-ref HEAD`・`.git/index` のハッシュ（`git hash-object --no-filters`）は一致した。`git for-each-ref` の差分は `refs/replor/pr/*/head` の追加 22 件だけで、`.git/FETCH_HEAD` も作られていない。
- bench と contract-demo の前後でも、neovim の clone の状態は一致した（記録した状態の md5 が `2e17497d…` で同じ）。

DESIGN.md 7.3 の環境変数の許可リストには、足りないものがある。

- 許可リストのままでは、プロキシ配下の fetch ができない（今回の sandbox がその環境だった）。fetch のときだけ、`HTTPS_PROXY`・`HTTP_PROXY`・`ALL_PROXY`・`NO_PROXY`（小文字も含む）と、ssh-agent 用の `SSH_AUTH_SOCK` を渡す必要がある。
- 認証は、ユーザーの Git 設定の credential helper に任せる（`HOME` を渡しているので global の設定を読む）。

### Azure DevOps

未実施（アクセスできる環境が無い）。PR の ref 名と OID での fetch の可否（概念設計の Open Questions Q5）は、P3 で Azure DevOps のリポジトリが使えるようになった時点で確かめる。そのための `docs/spike/p0/scripts/check-azure-devops.sh` を用意してある。使い方は `<clone の URL> <PR 番号> [PR の source commit の OID]` で、次を順に行う。

1. 使い捨ての clone を新しく作る。
2. `git ls-remote origin 'refs/pull/<N>/*'` で PR の ref 名を記録する。
3. 見つかった ref と、PR の source commit の OID の直接 fetch を `refs/replor/pr/<N>/` へ試す。
4. 前後の状態を比べる。

git の呼び出しは `contract.rs` と同じ契約を shell で組んだもので、状態を記録する関数の出力は Rust 版と一致することを確認した。

## ユーザーによる確認（2026-10-31、macOS）

| 項目 | 確認したこと | 結果 |
|---|---|---|
| S1 | Ghostty と、その中の herdr で `docs/spike/p0/scripts/check-images.sh` を実行。長い文書（`docs/spike/p0/fixtures/long-ja.md`）を `--images kitty` でスクロール | 上の記入表のとおり。長い文書でも、上下端をまたぐ図・画像に残骸や欠けは出なかった |
| S2 | Ghostty で `docs/spike/p0/s2-merman-output.md` を表示 | `Unicode` profile で枠が揃う。Ghostty は曖昧幅を 1 列で描く |
| S3 | Ghostty で `s3-diff tui`（PR #39773 の diff の前に日本語の diff を足した入力） を操作（スクロール・ファイル切り替え・split） | 遅れや固まりは感じなかった |

S1 のスクリーンショットは Ghostty の `auto`（`docs/spike/p0/s1/s1-ghostty-auto.png`）と `kitty`（`s1-ghostty-kitty.png`）。3 図とも日本語ラベルが読める。`auto` の画像では flowchart が上端で切り出されており、kitty の source rectangle による切り出しが働いている。sequenceDiagram の「承認 ✅」の豆腐は端末上でも同じ（S2 の PNG の項）。herdr の中のスクリーンショットは撮っておらず、`--images kitty` での表示は Ghostty と同じだったとユーザーが報告した。

個別の報告がないもの: S3 の 5 万行の入力と、Diff の中の日本語の列の揃い。

WSL の環境で S1・S2 を確かめるときは、同じ手順で `docs/spike/p0/scripts/check-images.sh <環境名>` と `cat docs/spike/p0/s2-merman-output.md` を使う。Noctty では OpenGL renderer で動いているか（Open Questions Q1）も記録する。
