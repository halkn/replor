# ADR-0001: P0 技術 Spike の採否

- Status: Accepted（macOS / Ghostty の実機確認済み。Linux は CI で確認済み）
- Date: 2026-10-12
- Scope: [ROADMAP](../plans/initial/ROADMAP.md) P0-01〜P0-09
- 計測環境: macOS（Apple Silicon）、Go 1.27.1、Apple Git 2.50.1、go.mod の依存版（bubbletea v2.1.0 / glamour v2.0.1 / lipgloss v2.0.6 / goldmark v1.8.6 / go-mermaid v0.1.6）

本書は P0 の結論と、その根拠となった検証を記録する。設計の基準そのものは [DESIGN](../plans/initial/DESIGN.md) に反映済みで、ここでは「なぜそう決めたか」と「何で確かめたか」を残す。

## 1. 結論の要約

| 領域 | 決定 | 状態 |
|---|---|---|
| Markdown 表示 | Glamour を継続。トップレベルブロック単位で描画し、goldmark の AST 索引と行範囲で対応付ける | 採用 |
| GFM 差異 | Alert・脚注は Glamour が扱えないため、自前の前処理／ブロック差し替えで表示する | P1-04 で対応 |
| 端末出力 | 入力由来の制御文字を描画前に可視化。Glamour が出す OSC 8 は http/https 以外を除去 | 採用 |
| Workspace 境界 | `os.Root` で全ファイルアクセスを行う | 採用 |
| Git 実行契約 | 3.3 の引数・環境変数・filter 無効化。最低 Git 2.31、partial clone は 2.45 以上 | 採用 |
| 非同期 | 表示先ごとに最新要求だけを受理する `app.Latest`（generation + 連番 + cancel） | 採用 |
| 上限 | 3.5 の数値で P1 を開始する | 採用（Linux CI で再計測済み） |
| Mermaid | go-mermaid を条件付きで採用。PNG 化は日本語非対応のため、フォールバックフォントを入れた raster の内製版が前提 | 条件付き |
| 画像表示 | Kitty Unicode placeholder／halfblocks／off の 3 段。Sixel・iTerm2 は v1 対象外 | 採用（Ghostty で確認済み） |
| 画像ライブラリ | go-termimg は不採用。placeholder と転送は `internal/preview` に約 100 行で持つ | 不採用 |
| 配布 | CGo 不要。darwin/linux × amd64/arm64 を `CGO_ENABLED=0` でビルド可能 | 採用（Linux 実行は CI で確認済み） |
| 対象環境 | macOS は保証、Linux は CI で動作確認、Windows Terminal + WSL はベストエフォート（検証環境がないため） | 採用 |

## 2. Spike の成果物

| パス | 内容 | P1 での扱い |
|---|---|---|
| `internal/process` | 環境変数の許可リスト、timeout、出力上限付きの外部コマンド実行 | そのまま使う |
| `internal/git` | Git 実行契約、版検出、Workspace ⇔ Git root のパス変換 | 拡張して使う |
| `internal/workspace` | `os.Root` による境界、読み取り上限、パス列挙 | 拡張して使う |
| `internal/document` | 制御文字の無害化、AST 索引（ブロック・見出し・リンク）、GitHub 互換アンカー、リンク解決 | 拡張して使う |
| `internal/preview` | ブロック単位の Markdown 描画、位置変換、検索、画像（Kitty / halfblocks） | 拡張して使う |
| `internal/app` | `Latest`（古い結果の棄却） | そのまま使う |
| `spike/tui` | 2 ペイン TUI の試作とヘッドレス操作テスト | P1 で削除済み。実装は `internal/app`・`cmd/replor`、テストは `internal/app/app_test.go` に移した |
| `spike/mermaid` | Mermaid の SVG/PNG 生成と失敗モードの確認 | P2-04 着手時に削除 |
| `testdata/` | GFM・Azure Wiki・Mermaid・画像の合成 fixture | そのまま使う |

負荷計測は `REPLOR_LOAD=1 go test ./internal/... -run Load -v` で再現できる。

## 3. 項目別の結果

### 3.1 P0-01 Bubble Tea v2 の最小 TUI

- `spike/tui` のヘッドレステスト（`Update`/`View` を直接駆動）で確認した範囲: 40×10〜200×60 のリサイズ、Tab によるフォーカス切替、ツリーのカーソル移動、狭幅時のツリー非表示。全行が画面幅に収まることを、wcwidth と grapheme の両方式で検査している。
- 問題: `lipgloss.JoinHorizontal` は最も広い行に全行を揃える。肌色修飾付き絵文字（👍🏽）などは wcwidth と grapheme で幅が異なるため、1 行の誤差が全行に広がって画面幅を超えた。Bubble Tea v2 は端末が mode 2027 に対応する場合だけ grapheme 方式になるので、実際の幅は端末に依存する。
- 決定: 本文を含むペインの合成は行ごとの切り詰め（`ansi.Truncate`）で行い、`JoinHorizontal` は幅が確定した部品だけに使う。
- 実端末: Ghostty で 5 章の手順をすべて実施し、問題がなかった（2026-10-12）。リサイズ・フォーカス・表と絵文字の行のはみ出し・リンク・戻る/進む・TOC・検索を含む。sandbox では pty を開けないため、この確認は手動で行った。
- 読み込みエラーで描画結果がない状態で同じ文書の履歴へ戻ると、spike が nil 参照で落ちていた。エラー時は描画結果に付随する状態（選択中のリンク・検索一致・TOC）も破棄し、描画結果がなければ開き直すよう修正した（`TestHistoryAfterLoadError`）。

### 3.2 P0-02 GFM と日本語

fixture は `testdata/markdown/gfm.md`。

| 対象 | 結果 | 回避策 |
|---|---|---|
| 表（日本語・右寄せ・コード） | 読める。長いセルは折り返す | なし |
| Task List | `[✓]` / `[ ]` で表示 | なし |
| 日本語長文 | 指定幅で折り返す | なし |
| コードブロック | chroma でハイライト | なし |
| Alert（`> [!NOTE]`） | `[!NOTE]` がそのまま引用として出る | P1-04: AST で検出し、ラベル付き引用に置き換える |
| 脚注 | 参照は `[^1]` のまま。定義は参照定義として解釈され、本文が**消える** | P1-04: 自前パーサー（Footnote 拡張あり）で定義ブロックを検出し、ブロック差し替えで描画する |
| 絵文字（修飾・ZWJ） | 端末の幅方式により表の罫線がずれ得る | 3.1 の行単位切り詰めではみ出しだけ防ぐ。完全な整列は保証しない |
| 相対リンクの URL 表示 | `./other.md` が `./other.md/other.md` と表示される（BaseURL 未設定時の Glamour の解決） | P1-04: ローカルリンクの URL 併記をやめる |
| 入力由来の制御文字 | 描画前に `␛` などへ可視化され、実行されない | 採用済み |

### 3.3 P0-08 境界と Git 実行契約

**Workspace 境界**

- `workspace.Open` で起点を `EvalSymlinks` 済みの絶対パスに固定し、以後のアクセスは `os.Root` を通す。`..`、絶対パス、ルート外を指す相対 symlink、ルート外への symlink ディレクトリ経由のアクセスを拒否する（`internal/workspace` のテスト）。
- `os.Root` は、ルート内を指していても**絶対パスの symlink を辿らない**。v1 はこの挙動を受け入れ、リンク切れと区別できる表示にする。
- 単一ファイル指定では親ディレクトリが root になり、`../README.md` は拒否される。

**Git root との対応**

- `rev-parse --show-toplevel --show-prefix` で Git root と prefix を取得し、`FromGit` / `ToGit` で変換する。
- 境界をまたぐ rename は、内側だけが Workspace のパスに変換され、外側は範囲外として扱われる。pathspec で絞ると rename の対応が失われるため、status はリポジトリ全体を取得してから変換する。

**Wiki**

- `testdata/azure-wiki` に、`.order`（重複・欠損・未記載）、本文と同名のサブフォルダ、本文のないフォルダ、`.attachments`、日本語、`%3A` を含むファイル名を用意した。
- リンクのパスは一度だけパーセントデコードしてから境界を検証する。`%2E%2E` は `..` として拒否し、`%252E` はリテラルとして扱う。
- 拡張子なしのリンクは `Page.md` を優先し、同名ファイルとの曖昧さはエラーにする。
- スキームは http/https 以外を拒否し、制御文字を含む URL とプロトコル相対 URL も拒否する。
- P2 に残す課題: Azure がファイル名に適用する変換（空白 → `-`、`:` → `%3A`）とリンクとの対応。現状は推測せず NotFound を返す。

**Git 実行契約**（Apple Git 2.50.1 で実測）

素の Git で観測した副作用:

- `git status`: `core.fsmonitor` に設定された hook を実行し、`.git/index` を書き換える。サイズが同じで mtime が新しいファイルには clean filter を実行する。
- `git diff`: clean / smudge filter と `diff.external` を実行する。
- partial clone: `diff` で不足している blob を取得する（オブジェクト数 4 → 6）。

これらを止める契約（`internal/git`）:

| 経路 | 対策 |
|---|---|
| index の書き換え | `--no-optional-locks` と `GIT_OPTIONAL_LOCKS=0` |
| fsmonitor hook | `-c core.fsmonitor=false` |
| clean / smudge / process filter | 設定済みの filter driver 名を Git 呼び出しのたびに `git config --get-regexp '^filter\.'` で列挙し（起動後に追加された driver にも効かせるため、キャッシュしない）、`GIT_CONFIG_COUNT` 経由で clean/smudge/process を空に、required を false にする。`-c` はサブセクション名に `=` を含むと表現できないため使わない |
| 外部 diff・textconv | diff 系の呼び出しに `--no-ext-diff --no-textconv` を必須とする |
| gpg | `-c log.showSignature=false` |
| pager | `--no-pager`、`GIT_PAGER=cat`、`PAGER=cat` |
| lazy fetch | Git 2.45 以上では `--no-lazy-fetch`。それ未満で partial clone（`remote.*.promisor` / `extensions.partialclone`）なら Git 機能を Unavailable にする |
| 対話認証 | `GIT_TERMINAL_PROMPT=0`。親プロセスの `*_ASKPASS` は引き継がない |
| 呼び出し元の環境変数による向き先変更 | 許可リスト（PATH, HOME, USER, LOGNAME, XDG_CONFIG_HOME, TMPDIR, SYSTEMROOT）以外を引き継がない。`GIT_DIR`・`GIT_INDEX_FILE`・`GIT_CONFIG_PARAMETERS`・`GIT_EXTERNAL_DIFF` を設定しても効かないことを確認した |
| パス | `--literal-pathspecs`。機械処理は `-z`。改行・空白・先頭ハイフン・`*`・`:(glob)` を含む名前を fixture 化 |
| submodule | status は `--ignore-submodules=dirty`。submodule 内の status は、その submodule 固有の filter 設定で実行されるため、dirty 判定は v1 で行わない（gitlink の OID 変更だけを表示） |
| 所有者の異なるリポジトリ | `safe.directory` による拒否を Unavailable（理由付き）として扱う |

- 最低対応 Git は **2.31**。`GIT_CONFIG_COUNT`（2.31、RelNotes 2.31.0）が必須のため。partial clone のみ **2.45**（`--no-lazy-fetch`、RelNotes 2.45.0）。
- 実行して確認したのは 2.50.1 だけ。版の判定と、古い版で partial clone を拒否する経路は、版を差し替えたテストで確認した。CI の `ubuntu:24.04` コンテナ（同梱 Git 2.43.0）でも全テストが通った。2.45 未満なので partial clone は Unavailable の経路になる。
- 契約の各要素を外すと対応するテストが失敗することを確認した（fsmonitor・index・filter・lazy fetch）。

### 3.4 P0-03 AST・リンク・TOC・検索・履歴

- 方式: goldmark（GFM + Footnote）で `document.Doc` を作る。トップレベルブロックはソース位置順に並べ、各ブロックは次のブロックの開始行までを担当する。Footnote 拡張は定義を AST 末尾へ移すため、AST 順では並べない。Glamour は各ブロックのソースに全参照定義を足して個別に描画し、`Rendered.BlockLine` で行とブロックを対応付ける。
- 閲覧位置は `Position{Block, Offset}` で表す。幅が変わったときは同じブロックの同じ行オフセットへ戻す（ブロックが短くなった場合は末尾に丸める）。
- 見出し ID: github-slugger 互換の生成器で、見出しの描画後テキストから作る（日本語・記号・重複の `-1`・リンクやコードを含む見出しを検証）。goldmark の自動 ID は見出しのソースから作られ、非 ASCII も落とすため使わない。
- ヘッドレステストで成立を確認した操作: リンク選択 → 遷移 → 戻る → 進む、別文書へのリンクと戻る、境界外リンクの拒否、TOC での重複見出しへのジャンプ、検索 → 次/前 → リサイズ後も一致が画面内、A→B の完了順逆転で A を棄却。
- 制約: リンクの選択表示はブロック単位（ガターの印）で、行内のハイライトはしない。検索は描画後のプレーンテキストに対するリテラル一致。
- 性能: ブロック単位描画は一括描画より約 10% 遅い（100KiB: 80ms 対 78ms、1MiB: 596ms 対 540ms）。Glamour は内部で再パースするため、パースは 2 回走る。
- P1 への申し送り: 現在の復元は「画面先頭の位置」を基準にしている。検索一致・選択リンクなど注目対象が画面外へ出ないことを保証するには、注目対象を別に保持する。

### 3.5 P0-09 非同期と上限

**計測**（macOS / APFS と、GitHub Actions の Ubuntu ランナー上の `ubuntu:24.04` コンテナ。WSL の `/mnt/c` は検証環境がなく未計測）

| 対象 | macOS | Linux（CI ランナー） |
|---|---|---|
| パス列挙（`os.Root` 経由の WalkDir） | 1万件 163ms、10万件 1.58s、30万件 5.23s。最初の 512 件は約 10ms。ヒープ増分は 30万件で約 29MiB | 1万件 61ms、10万件 566ms、30万件 1.57s。最初の 512 件は約 4ms。ヒープ増分は約 22MiB |
| Markdown 解析と描画（幅 100） | 100KiB 80ms、1MiB 596ms、4MiB 2.37s。描画結果は元文書の約 9 倍（1MiB → 9.4MiB） | 100KiB 250ms、1MiB 2.44s、4MiB 6.66s |
| Git status（2万ファイル、契約込み） | Open（rev-parse と config）を含めて約 100ms | 約 65ms |
| Mermaid | SVG 0.1〜0.3ms、PNG（scale 2）40〜60ms | SVG 0.1〜0.2ms、PNG 105〜173ms |

- Linux ランナーでは描画が約 4 倍遅く、Preview の上限（1MiB）で 2.4 秒かかる。描画は非同期なので操作は止まらないため、上限は据え置く。待ち時間の短縮は P1-04 で扱う（画面に見えている範囲のブロックから先に描画する）。

**P1 の初期値**

| 上限 | 値 | 根拠 |
|---|---|---|
| Markdown Preview | 1 MiB。超えたら Source 表示に切り替え、理由を出す | 描画 0.6s 以内 |
| ファイル読み取り（Source） | 8 MiB。超えたら「大きすぎる」と表示する | 全文をメモリに置く上限 |
| パス列挙 | 20万件、深さ 64。到達時は部分結果であることを表示 | 約 3.5s・約 20MiB |
| 描画キャッシュ | 合計 64 MiB かつ 16 件、LRU | 1MiB 文書の描画結果が約 9.4MiB |
| 同時実行 | 描画は表示先ごとに 1 件（新しい要求が古い要求をキャンセル）。Git はリポジトリごとに直列。列挙は 1 件 | `Latest` で古い結果を棄却 |
| Git 1 呼び出し | timeout 10s、出力 64 MiB | 異常時でも UI を止めない |

- `app.Latest` は、A→B の完了順逆転、表示先の独立性、generation 切替、200 件の同時完了で、最新 1 件だけが受理されることを `-race` 付きで確認した。
- `process.Run` の出力上限は当初効いていなかった。`bytes.Buffer` を埋め込んだため、昇格した `ReadFrom` を `io.Copy` が使い、`Write` を迂回していた。テストで検出して修正済み。

### 3.6 P0-04 Mermaid

- go-mermaid v0.1.6（MIT、pre-1.0）。PNG 化は oksvg / rasterx（BSD、2022 年以降更新なし）と `golang.org/x/image` に依存する。CGo・外部プロセス・ネットワークは不要。
- graph・sequence・ER の 3 図種とも、日本語ラベルを含む SVG を生成できた。
- 失敗モード: 構文エラーは `ErrParse`（行・列付き）、未対応の図種は `ErrUnsupported`。どちらも図ブロック単位でソース表示に戻し、文書全体の表示は止まらない（`spike/tui` のテスト）。
- 問題 1（重大）: PNG 化は Go フォント固定で、**日本語ラベルが豆腐（□）になる**。raster パッケージ（約 400 行）を複製し、日本語を含むラベルだけシステムフォント（ヒラギノ角ゴシック W3）へフォールバックさせる試作で解消できた。
- 問題 2: 楕円弧（`A` コマンド）の rasterize が崩れ、円柱ノード `[(…)]` に余計な線が出る。SVG 側は正しい。
- 問題 3: ラベル幅を Helvetica の文字幅で見積もるため、日本語では sequence の alt 条件などが重なる。
- 決定: Mermaid 画像は v1 に残すが、条件付きとする。P2-04 では raster を内製化（MIT の複製と改変）し、CJK フォールバックを入れる。フォントは同梱せず、OS のフォントを探索する（macOS: ヒラギノ、Linux: Noto CJK）。見つからない場合と、未対応の構文・生成エラーの場合はソース表示に戻す。問題 2・3 は upstream へ報告し、v1 の既知制約として記載する。

### 3.7 P0-05 ターミナル画像

- Bubble Tea v2 のセル描画（ultraviolet）は APC/DCS をセルに付けて素通しするが、セルが再描画されるたびに再送される。画像データの転送は `tea.Raw` で 1 回だけ行い、本文には Kitty の Unicode placeholder（U+10EEEE と行・列の diacritics）を文字として置く。こうすると、スクロール・クリップ・差分描画はテキストと同じ仕組みで扱える。
- 確認済み（ヘッドレス）: placeholder は wcwidth・grapheme のどちらの幅方式でも 1 セルに 1 つずつ保持される。スクロールしても画面幅を超えない。リサイズ時は新しい ID で再転送し、古い ID を削除する。終了時にも削除する。halfblocks は指定したセル数ちょうどで描画される。
- 表示方式の判定は環境変数だけで行う。端末への問い合わせは Bubble Tea の入力読み取りと競合するため。

| 条件 | 方式 |
|---|---|
| `KITTY_WINDOW_ID`、`TERM=xterm-kitty` / `xterm-ghostty`、`TERM_PROGRAM=ghostty` | Kitty |
| `TMUX` | halfblocks |
| `COLORTERM=truecolor` / `24bit`、`WT_SESSION`（Windows Terminal） | halfblocks |
| それ以外 | off |

- Sixel は v1 の対象外。セル単位の差分描画の外で書き込む必要があり、スクロール時のクリップと消去を自前で管理することになるため。Windows Terminal は halfblocks で表示する。
- go-termimg は不採用。端末判定で stdin へ問い合わせ、描画も stdout へ直接書き込む設計で、Bubble Tea に組み込めない。必要だったのは placeholder の diacritics 表と転送形式だけで、diacritics 表は termimg の値と照合済み。
- 制約: セルの縦横比を 1:2 と仮定しており、Kitty では画像が歪み得る。セルのピクセル寸法の取得は P2-04 で扱う。
- 実機: Ghostty で、Kitty 画像と halfblocks の表示、スクロールへの追従、リサイズ時の再描画、終了時の消去、Mermaid の構文エラー時のソース表示、`-images off` での閲覧を確認し、問題がなかった（2026-10-12）。Windows Terminal は検証環境がなく、任意の P3-02 まで未確認。

### 3.8 P0-06 配布

- `CGO_ENABLED=0 go build -trimpath -ldflags="-s -w"` で darwin/arm64・darwin/amd64・linux/amd64・linux/arm64 をビルドできた（spike/tui で約 15MB）。Linux 版は静的リンク。darwin 版がリンクするのは libSystem と libresolv だけ。
- 実行時に必要なのは `git`（任意）だけで、Node・Chromium・Go toolchain は不要。Mermaid の日本語表示には OS のフォントを使う。
- go.mod の go directive は依存側の要求で 1.26.0。
- **未確認**: Linux 版の実行。

## 4. 既知の制限とフォールバック

| 制限 | フォールバック |
|---|---|
| Alert・脚注は Glamour 非対応 | P1-04 で自前表示。それまではソースのまま表示 |
| 絵文字の幅は端末依存 | 行単位の切り詰めではみ出しを防ぐ |
| リンク選択はブロック単位 | ガターの印とステータス行に、リンク文字列と遷移先を表示 |
| 絶対パスの symlink は辿らない | 理由を表示 |
| Mermaid PNG の日本語・円弧・ラベル幅 | ソース表示。日本語は P2-04 のフォント探索で対応 |
| Sixel / iTerm2 / tmux での画像 | halfblocks または off |
| Git 2.31 未満、または 2.45 未満の partial clone | Git 機能を理由付きで Unavailable にし、ファイル閲覧は継続 |
| submodule の dirty 判定 | 表示しない（gitlink の OID 変更だけを表示） |

## 5. 手動確認の手順（Ghostty で実施済み、2026-10-12）

`spike/tui` は P1 で削除した。同じ手順は `go build ./cmd/replor` のバイナリで再現できる。違いはツリーの表示切替が `b` から `e` になったことと、Mermaid が P2-04 まで画像ではなくコードブロックとして表示されることだけ。

ビルド:

```sh
CGO_ENABLED=0 go build -o replor-spike ./spike/tui
```

Ghostty（macOS）で次を確認する。P1 着手の条件はこの確認だけとする。Windows Terminal + WSL2 はベストエフォート（任意の P3-02）とし、Linux 固有の確認（Git 版・負荷・Linux バイナリの起動）は P1-00 の CI で行う。

1. `./replor-spike testdata/markdown/gfm.md` を起動し、表・日本語・絵文字の行で罫線と右端が崩れないことを確認する。
2. ウィンドウを細くしたり広げたりして、ツリーが 70 桁未満で消え、本文が折り返し直されることを確認する。
3. `Tab` で本文へ移り、`>` で `data` リンクを選んで `Enter` を押し、`Backspace` で戻り、`]` で進む。
4. `t` で TOC を開き、2 つ目の「重複見出し」に `Enter` で移動する。
5. `/` で `書生` と入力して `Enter` を押し、`n`/`N` で移動したあとウィンドウ幅を変えても、一致した行が画面内に残ることを確認する。
6. `./replor-spike testdata/markdown/media.md` を起動し、Kitty 画像で表示されることを確認する。続けて次を確認する。
   - `j`/`k` でスクロールすると、図が本文と一緒に動いて端で欠けること。
   - リサイズで図が描き直されること。
   - `q` で終了したあと画面に図が残らないこと。
   - 2 つ目の Mermaid（構文エラー）がソース表示になり、ステータス行にエラーが出ること。
7. `-images off` と `-images halfblocks` でも同じファイルを開き、文書全体が読めることを確認する。
結果は本 ADR の 3.1・3.7 に追記する。
