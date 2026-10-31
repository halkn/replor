# ADR-0001: P0 Spike の結論

- Status: Accepted（2026-10-31。WSL の 4 環境と S4 の Azure DevOps は、検証できる環境が無いため未実施。残るリスクは Consequences に記す）
- 根拠: `docs/spike/p0-results.md`、計画: `docs/design/spike-p0.md`

## Context

P1 以降の設計判断のうち、次の4つは検証してから決める必要があった。

1. 画像の層の方式（mdvu 方式か ratatui-image か）
2. merman の版
3. Diff の描き方
4. Git からの読み取りの方式

`docs/design/spike-p0.md` の判定基準に照らして、ここに結論を記録する。

## Decision

### S1: 画像の層は mdvu の方式をそのまま使う

判定基準の「mdvu 方式が普段の環境すべてで表示できる（`--images` の強制や設定で出せる場合を含む）」に、検証できた macOS の 2 環境で当たる。ratatui-image は比べていない（比べる条件の「mdvu 方式で出せない環境」がなかった）。

- macOS ＋ Ghostty 1.3.1: `auto` が kitty を選び、3 図とも表示でき、スクロールと終了で残骸が出なかった。
- その中の herdr 0.9.5（`experimental.kitty_graphics = true`）: herdr が `TERM_PROGRAM=herdr`・`TERM=xterm-256color` を設定して外側の端末を隠すので、`auto` は off になる。`--images kitty` で強制すれば、Ghostty と同じく表示でき、残骸も出なかった。
- 強制が要った環境は、判定基準どおり設定の既定値として扱う。herdr の `experimental.kitty_graphics` が有効かは環境変数から分からず、`TERM_PROGRAM=herdr` を kitty と判定すると、無効のときに escape sequence が流れうる。そのため `auto` の判定には足さず、設定ファイルで `images = "kitty"` を既定にできるようにする（herdr の利用者は設定する）。
- WSL の 4 環境（Noctty・WezTerm とそれぞれの中の herdr）は検証できなかった。macOS の結果だけで決め、WSL での表示は未検証のリスクとして残す（Consequences）。

あわせて次を決める。

- **描画コアは merman の呼び出しを `catch_unwind` で囲み、1 つの図の panic を、その図だけの描画エラーとして扱う。** mdvu 0.10.2（merman 0.7.0）は、erDiagram の属性名が非 ASCII だと panic し、文書全体を表示できなかった。版を上げても、merman が panic しないとは保証されていない。

### S2: merman は 0.9 系に上げ、テキスト図は `Unicode` profile を既定にする

判定基準の「日本語の枠が揃う」に当たる。曖昧幅を 1 列で描く端末では、0.9.3 の `Unicode` profile で日本語の枠が揃った。0.7.0 はどの図種でもずれた。

- P1 で mdvu の描画コードを取り込むときに、merman を 0.9 系に上げる。snapshot の差分（6 件。日本語の枠、sequence の幅、state 図のテキスト化、エラー文言）は一度で受け入れる。mdvu 本体は 0.7.0 のまま据え置く。
- API の変更は `diagram/mermaid.rs` の中で閉じる（`Renderer::render(RenderRequest::{ascii, svg})`）。SVG は `SvgPipeline::resvg_safe()` を明示する。図種は `diagram-*` の feature で選ぶ。
- width profile の既定は `Unicode` とし、曖昧幅を 2 列で描く端末向けに設定で `Cjk` を選べるようにする。`Cjk` では罫線が ASCII になる。Ghostty は曖昧幅を 1 列で描き、`Unicode` profile で揃うことを目視で確かめた。Noctty・WezTerm の曖昧幅は未確認のまま残る。
- 0.9.3 で新しく出る制約は次のとおりで、取り込み時にどれも扱う。
  - **戻りの辺を持つ flowchart は、テキスト図にできない。** エラーの種類（`UnsupportedFeature`）で判定して「テキストでは描けない図」としてソースを出す。mdvu の文字列判定（`is_unsupported`）は使わない。画像の経路では描ける。
  - erDiagram の属性は `type(bytes=N)="…"` の形式で出る（merman の仕様）。読みにくさは既知の制約として扱う。
  - PNG で、erDiagram の属性名の左端が欠ける（0.7.0 では欠けない）。upstream の不具合として報告し、取り込み時に直っていなければ既知の制約とする。
  - PNG で、絵文字を含むラベル全体が豆腐になる。0.7.0 でも同じなので、版の判断には影響しない。mdvu の raster 側のフォントの選び方の問題として、P1 で扱う。

### S3: Diff は自前で描き、見えている行だけを組み立てる。ハイライトは同期で全行にかけない

判定基準の「数千行規模の PR で固まる」には当たらない。PR #39773（80 ファイル・15,720 変更行）では、全行を組み立てる方式でも 1 フレーム 1 ms 未満だった。それでも P3 では次を前提にする。

- 見えている行だけを組み立てる（window 方式）。全行方式の費用はファイルの行数に比例し、5 万行で 1 フレーム 23 ms になった。window 方式は 0.25 ms で、行数に依らない。herdr-reviewr も同じ方式を取っている。
- 構文ハイライトは、ファイルを開いたときに全行を同期でかけない。syntect は約 0.1 ms／行で、5 万行で 4.9 s 止まった。見えている範囲から順にかけるか、別スレッドでかけて終わった行から差し替える。あわせて、herdr-reviewr と同じく行数・バイト数の上限を置き、超えたファイルは通知に替える。
- Diff の行モデルは、git の unified diff の文字列からではなく、旧新の blob から作る（herdr-reviewr の `diff.rs` の方式）。blob は S4 の Snapshot から読めるので、`git diff` を呼ばずに済み、Git 実行契約の対象を減らせる。

### S4: リビジョンの読み取りは gix、fetch は契約付きの git CLI

判定基準の「gix が不変条件を保てない」には当たらない。gix の object の読み取りは filter・textconv を通さず、index も書き換えなかった。

- Snapshot（リビジョン）のファイル一覧と内容は gix で読む。git CLI より速かった。一覧は 0.8 ms と 20.5 ms、1 ファイルあたりは約 0.02 ms と常駐 `cat-file --batch` の約 0.12 ms だった。また、プロセスの寿命を管理しなくて済む。
- gix で diff と status を使う場合の制約:
  - diff（`gix-diff` の blob pipeline）を使うなら、`Mode::ToGit` を明示する。既定では filter と textconv の driver を起動する。
  - P4 で作業ツリーの status に gix を使うなら、filter を無効にして開く。この組み合わせは未検証なので、P4 で確かめる。
- fetch と `ls-remote` は、旧 replor の DESIGN.md 7.3 の契約を付けた git CLI で行う。認証・プロキシ・ssh はユーザーの Git 設定と環境に任せる。
- fetch の書き先と書き方は次のとおり。
  - 書き先は `refs/replor/` の下に限る。GitHub は `+refs/pull/<N>/head:refs/replor/pr/<N>/head`。
  - FETCH_HEAD・remote-tracking ref・tag・commit-graph を書かず、auto gc と maintenance を起こさない（`--no-write-fetch-head --no-tags --no-auto-gc --no-auto-maintenance --no-write-commit-graph`、`-c gc.auto=0 -c maintenance.auto=false -c fetch.writeCommitGraph=false`）。
  - 前後で比べると、status・HEAD・symbolic-ref・index のハッシュは一致し、ref の差分は `refs/replor/` だけだった。
- **Git 実行契約の環境変数の許可リストに、fetch のときだけ `HTTPS_PROXY`・`HTTP_PROXY`・`ALL_PROXY`・`NO_PROXY`（小文字も含む）と `SSH_AUTH_SOCK` を足す。** 許可リストのままでは、プロキシ配下の fetch が失敗した。
- Azure DevOps の PR の ref 名と、OID で直接 fetch できるか（概念設計の Open Questions Q5）は未検証のまま残す。git CLI で fetch する方式は refspec を変えるだけで対応できるので、この ADR の決定には影響しない。P3 で Azure DevOps のリポジトリが使えるようになった時点で、`docs/spike/p0/scripts/check-azure-devops.sh` で確かめる。

## Consequences

- P1 で mdvu の `image/`・`pager/images.rs` をそのまま取り込み、`--images` の既定値を設定ファイルで変えられるようにする（herdr 向け）。
- P1 で `diagram/mermaid.rs` を書き換え、snapshot の差分を受け入れ、`catch_unwind` と、エラーの種類による未対応の判定を足す。width profile の設定項目を足す。
- P3 の Diff は、window 方式、非同期または範囲限定のハイライト、行数とバイト数の上限、blob から作る行モデルを前提にする。
- Snapshot の trait のリビジョン側の実装は gix に依存する（gix 0.90、features `revision`・`max-performance`・`sha1`）。git CLI は fetch・`ls-remote` と、gix で代えられない操作に限る。
- 旧 replor の DESIGN.md 7.3 の契約は、fetch 用の環境変数を足した形で replor の設計ドキュメントに引き継ぐ。
- 未検証のまま P1 に進むリスクが2つある。
  - WSL（Noctty・WezTerm とその中の herdr）での画像表示: mdvu の環境変数による判定が働かない場合は、`--images` の指定か設定の既定値で補う想定。Noctty の Direct3D 11 renderer では kitty の画像が描かれない（Open Questions Q1）。
  - Azure DevOps の PR の ref 名（Q5）。
- `spike/` は Accepted の後に削除した（コードはコミット `151d461` に残る）。上の未検証のリスクを確かめるためのスクリプトと fixture は `docs/spike/p0/scripts/`・`docs/spike/p0/fixtures/` に移した。
