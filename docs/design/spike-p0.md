# P0 Spike: Rust で残っている技術リスクを検証する

## Why

概念設計（`docs/design/concept.md`）の実装基盤は Rust / Ratatui とし、描画は mdvu、ナビゲーションは docsail の資産を引き継ぐ。Markdown の描画、Azure Wiki 方言、Mermaid の画像化、kitty での画像表示は mdvu で動いている。残っているのは、mdvu と docsail が扱っていない、または両者で方式が分かれている次の点である。

- 画像表示の方式が2つに分かれている。mdvu は kitty と iTerm2 を自前で描き、環境変数だけで判定し、sixel を持たない。docsail は ratatui-image で端末に問い合わせ、sixel も扱う。普段の端末（macOS の Ghostty、WSL＋Noctty、WSL＋WezTerm、それぞれの中の herdr）はどれも kitty か iTerm2 の protocol を持つので、mdvu の方式が第一候補になる。ただし、WSL の中と herdr の中で mdvu の環境変数判定が働くか、WezTerm で kitty が不具合なく動くかは確認していない。
- 画像を出せない環境で頼るテキスト図は、merman 0.7 では日本語ラベルで枠がずれる（mdvu の Known limitations）。merman 0.8.0 で terminal-width profile（`Unicode` / `Cjk`）が入ったが、日本語で揃うかは確認していない。
- Diff は Ratatui に既製の部品がなく、自前で描く。PR 規模での速度を確認していない。
- リビジョン（PR head）からの読み取りと、専用 ref への fetch は、どのツールでも実装していない。

本 Spike はこれらを検証し、P1 以降の設計判断（画像の層をどう作るか、merman の版、Diff と Git 読み取りの方式）を決める。成果物はコードではなく判定と根拠である。Spike のコードは捨てる前提で書き、本実装へ流用しない。

却下した進め方:

- 検証せずに P1 から始める: 画像の層の方式は描画コアと pager の境界を決める。P1 で mdvu のコードを replor に取り込んだ後に方式を変えると、`image/` と `pager/images.rs` を作り直すことになる。
- TS / Bun / OpenTUI 側の Spike（旧版の本文書）: 実装基盤を Rust にしたため不要。

## What

次の S1〜S4 を検証し、項目ごとに結果（成立／条件付き成立／不成立）と根拠を記録する。

### 実施の分担

| 項目 | エージェント | ユーザー |
|---|---|---|
| S1 | fixture と、確認用のスクリプト（環境変数 `TERM`・`TERM_PROGRAM`・`KITTY_WINDOW_ID` 等を表示し、mdvu を `--images auto` → `kitty` → `iterm2` の順に起動する）、結果の記入表を用意する | 6つの環境でスクリプトを実行し、表示の可否を記入し、スクリーンショットを撮る |
| S2 | 捨てるブランチで mdvu の merman を上げ、出力例と snapshot の差分を記録する。Ghostty 以外の端末での表示幅の確認手順を書く | Noctty・WezTerm で出力例を開き、枠が揃うかを記入する |
| S3 | 最小の Ratatui アプリを作り、PR の規模と計測値を記録する。体感速度の確認手順を書く | 体感速度を記入する |
| S4 | GitHub の部分をすべて行う。Azure DevOps の部分は手順とスクリプトを用意する | Azure DevOps のリポジトリにアクセスできる環境で、Azure DevOps の部分を実行する |

順序は S2・S3・S4（GitHub）と S1 の準備を先に進め、ユーザーの確認が要る項目はまとめて渡す。結果は `docs/spike/p0-results.md` に、結論は `docs/adr/0001-p0-spike.md` に記録する。

### 前提

- 参照するリポジトリ（halkn/mdvu、halkn/docsail、halkn/replor-go、persiyanov/herdr-reviewr）は手元に clone してある。パスは依頼時に伝える。見つからないときは、読み取り用に別のディレクトリへ clone してよい。

### S1: 普段の環境で画像を表示できるか（最重要）

日本語ラベルを含む flowchart・sequenceDiagram・erDiagram を、mdvu の `--mermaid image` で表示する（merman で SVG、resvg で PNG）。まず `--images auto` で試し、出なければ `--images kitty`、次に `--images iterm2` で強制する。

| 環境 | 記録すること |
|---|---|
| macOS ＋ Ghostty | `auto` が選んだ protocol、表示の可否 |
| 上記の中の herdr（`experimental.kitty_graphics = true`） | 同上。herdr の中で `TERM`・`TERM_PROGRAM` が何になり、`auto` が kitty を選ぶか |
| WSL2 ＋ Noctty | 同上。WSL 側の `TERM_PROGRAM` が `ghostty` か。Noctty が OpenGL renderer で動いているか（Direct3D 11 renderer では Kitty の画像が描かれない） |
| 上記の中の herdr | 同上 |
| WSL2 ＋ WezTerm | 同上。WSL 側に `TERM_PROGRAM` が渡るか。kitty と iTerm2 のどちらが崩れずに描けるか |
| 上記の中の herdr | 同上 |

あわせて、スクロールで画像が上下端にかかるとき、別の文書へ切り替えたときに、画像の残骸が出ないかを確認する。

mdvu の方式で表示できない環境があったときだけ、docsail と同じく ratatui-image の `Picker::from_query_stdio` で protocol を選ぶ最小の Ratatui アプリを作り、その環境で同じ PNG を表示して比べる。起動時の問い合わせにかかる時間も記録する。

### S2: merman 0.9 のテキスト図で日本語の枠が揃うか

mdvu の merman を 0.7.0 から最新（2026-10-31 時点で 0.9.3）に上げ、`Cjk` と `Unicode` の width profile で、日本語ラベルを含む flowchart・sequenceDiagram のテキスト図を描く。

- 枠が揃うか。profile ごとの出力例を記録する。
- 端末（Ghostty、Noctty、WezTerm）での全角文字の表示幅と、profile の想定が合うか。
- 上げたことで mdvu の snapshot（`tests/render.rs`）と `--mermaid image` の経路に出る差分。API の変更で直す必要があった箇所。

### S3: Diff を自前で描いたときの速度

実在する PR の unified diff（数十ファイル・数千行規模）を、最小の Ratatui アプリで unified と split の両方に描く。

- 日本語を含む行の表示幅、構文ハイライト（mdvu の `layout/highlight.rs` の syntect の流用）
- ファイル切り替えとスクロールの体感速度（固まるかどうか）。見えている範囲だけを描く方式が要るか
- herdr-reviewr の Diff 描画の構成（参考にできる単位）

### S4: リビジョンからの読み取り

作業ツリーを変更せずに、任意のコミット（PR head を想定）のファイル一覧と内容を読む。

- `git ls-tree` と `git cat-file --batch` による読み取りと、gix による読み取りの速度。作業ツリー直読みとの差。対象は手元で最も大きいリポジトリとする。
- 旧 replor の DESIGN.md 7.3 の Git 実行契約（環境変数の許可リスト、`-c` での設定上書き等）を `std::process::Command` で満たせるか。gix を使う場合に同じ不変条件（index を書かない、filter driver・textconv を起動しない）が保てるか。
- PR head を専用の ref 名前空間へ fetch できるか。GitHub と Azure DevOps のそれぞれで、`git ls-remote origin 'refs/pull/*'` 等で PR の ref 名を確認し、記録する。

## Scope

### In scope

- S1〜S4 の検証と、結果の記録
- 検証用の fixture（日本語ラベルの Mermaid 3図種、PNG 画像、長い Markdown 文書、Azure Wiki 形式の `::: mermaid` を含む文書）。mdvu の `tests/fixtures`、旧 replor の `testdata/azure-wiki`、docsail の `docs/gfm-preview-test.md` を流用してよい

### Out of scope

- 描画コアの取り込みと crate 化、Snapshot の trait（P1）。ただし S1〜S4 の結果で取り込み方が変わる点は記録する
- mdvu 本体への変更。S2 で mdvu の merman を上げる作業は捨てるブランチで行い、mdvu の main には入れない
- ファイルツリー、Wiki の `.order`、リンク遷移、履歴などのナビゲーション（docsail で動作実績がある）
- `gh` / `az` による PR メタデータの取得（実績のある CLI で、方式の判断に影響しない）
- 配布（mdvu と docsail の release workflow で単一バイナリの配布が動いている）
- 本実装で使える品質のコード、テスト、エラー処理

## Acceptance Criteria

- [ ] S1〜S4 の各項目について、結果（成立／条件付き成立／不成立／未実施）と根拠が表で記録されている。未実施の項目には理由がある。
- [ ] S1 で、検証した環境と方式ごとに、選ばれた protocol と表示の可否が記録されている。表示できた組み合わせには、日本語ラベルを含む flowchart・sequenceDiagram・erDiagram の端末のスクリーンショットがある。
- [ ] S2 で、merman の版と width profile ごとの、日本語ラベルを含むテキスト図の出力例と、mdvu の snapshot の差分の要約が記録されている。
- [ ] S3 で、使った PR の規模（ファイル数・行数）と、unified / split それぞれの体感速度が記録されている。
- [ ] S4 で、検証の前後の `git status --porcelain=v2`、`git rev-parse HEAD`、`.git/index` のハッシュが一致し、`git for-each-ref` の差分が専用の名前空間の ref だけである。GitHub と Azure DevOps の PR の ref 名が記録されている。
- [ ] 判定基準に照らした結論と理由が ADR として記録されている。

## 判定基準

| 結果 | 判定 |
|---|---|
| S1 で mdvu 方式が普段の環境すべてで表示できる（`--images` の強制や設定で出せる場合を含む） | mdvu の画像の層をそのまま使う。強制が要った環境は、判定に足す環境変数か設定の既定値として記録する |
| S1 で mdvu 方式が出せず、ratatui-image 方式だけが表示できる環境がある | 画像の層を ratatui-image に寄せるか、mdvu の層に足りない判定や protocol を足すかを、kitty の切り出し・格納の挙動（mdvu `docs/mermaid-image.md`）を保てるかで決める |
| S1 で普段の環境（herdr 内を含む）にどちらの方式でも画像が出ない | S2 の結果を見る。S2 で日本語のテキスト図が揃えばテキスト図を主経路にする。揃わなければ、概念設計の却下案「Rust ＋ htmx 4 の Web UI」に切り替えるかを再判断する |
| S2 で日本語の枠が揃う | P1 で mdvu の描画コードを取り込むときに merman を上げ、snapshot の差分を一度で受け入れる。mdvu 本体は 0.7.0 のまま据え置く |
| S3 で数千行規模の PR で固まる | 見えている範囲だけを描く方式を P3 の前提にする |
| S4 で gix が Git 実行契約と同じ不変条件を保てない | git CLI を `Command` で呼ぶ |

S1 は半日〜1日を目安とし、超えたら打ち切って状況を記録する（目安は推測）。

## 停止条件

- エージェントだけで進められる作業が終わり、ユーザーの確認（S1 の表示、S2・S3 の目視、S4 の Azure DevOps）が要る状態になったとき。確認手順をまとめて渡して止まる
- 検証に使うリポジトリで、作業ツリー・index・ユーザーのブランチが変わった、または変わりそうな操作が必要になったとき
- 判定基準の「Web UI に切り替えるかを再判断する」に当たったとき

## Constraints

- 実装は Rust、Ratatui、Crossterm。
- Spike のコードは新規の halkn/replor の `spike/` ディレクトリに置き、S ごとに独立した crate とする。本実装の crate から参照しない。ADR を記録したら削除する。
- S4 で、作業ツリー・index・ユーザーのブランチを変更しない。fetch で書く ref は専用の名前空間に限る。検証に使うリポジトリは、壊れても困らない clone とする。

## Open Questions

- Q1: 会社 PC の Noctty が OpenGL renderer で動くか。リモートデスクトップや VM で使うと Direct3D 11 renderer に切り替わり、Kitty の画像が描かれない。

## References

- 概念設計: `docs/design/concept.md`
- mdvu: README.md（Mermaid、Images、Known limitations）、docs/mermaid-image.md、docs/runtime.md（画像プロトコル）、src/image/、src/pager/images.rs、src/diagram/
- docsail: src/main.rs・src/diagram.rs（ratatui-image の Picker）、docs/adr/0004
- merman: <https://github.com/Latias94/merman（CHANGELOG.md> の 0.8.0、crates/merman-ascii の `TerminalWidthProfile`）
- ratatui-image: <https://crates.io/crates/ratatui-image>
- Noctty: <https://github.com/amanthanvi/noctty（README.md> の Requirements、docs/status.md、docs/windows.md の WSL）
- 旧 replor（halkn/replor-go）: docs/plans/initial/DESIGN.md 7.3（Git 実行契約）、testdata/azure-wiki
