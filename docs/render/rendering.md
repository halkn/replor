# パース〜レイアウトの決定事項

`crates/replor-render/` の `src/markdown/`・`src/flavor/`・`src/diagram/`・`src/layout/`・`tests/render.rs`・`tests/fixtures/`・`tests/snapshots/` を触る前に読む。文中の `--line`・`--watch`・`--mermaid` などは mdvu の CLI のフラグを、`main.rs`・`pager/` は mdvu の module（replor では `replor` crate に置く）を指す。content root の規則は、この文書ではなく `docs/design/p1-core.md` の「root と content root」に従う。

## IR とフレーバー

`markdown/model.rs` は renderer-neutral に保つ。`ratatui` や端末型への依存をここへ持ち込まない。フレーバー差は `flavor/` の内側に閉じ、`layout/` 以降へ持ち込まない。`azure_devops.rs` は Azure 固有の記法を汎用の block / inline へ変換する役目であって、描画の分岐を作る役目ではない。

- **`Block::Image` は作らない。** CommonMark では画像は inline 要素であり、段落と独立した block にすると source range と wrap 処理が二重化する。`Inline::Image` として保持し、layout 側で `[image: ...]` / `[attachment: ...]` の placeholder か、描画領域へ変換する。
- **`Block::Alert` も作らない。** GFM alert は「ラベルを持つ引用」であり、構造は `Block::Quote` と同一。`QuoteBlock.kind: Option<AlertKind>` として持てば、`Block::range()` や `Block::children()` を通る既存の走査がそのまま動く。variant を足すとその全部に分岐が増え、得られるのは layout の 1 箇所の分岐だけになる。
- alert の判定は `Options::ENABLE_GFM` に任せ、フレーバーで分けない。Azure DevOps Wiki も同じ `> [!NOTE]` 記法を持つため、`markdown/parser.rs` の共通経路に置くのが両フレーバーで正しい。未知の種別（`[!FOO]`）は `kind: None` の普通の引用になり、マーカーは本文に残る。
- ブロック木の再帰は `Block::children()` / `children_mut()` だけを通す。走査ごとに container を列挙すると、どこまで降りるかが走査ごとにずれる。container を足すときはこの 2 つに足す。
- すべての user-authored block は `SourceRange`（byte 範囲 + 1-based 行）を持つ。`--line` と `--watch` の読み位置保持がこれに依存するので、新しい block を足すときも必ず持たせる。
- 描画行がソース行と 1 対 1 に対応する block（コードブロックの本文と fence、表の各行と区切り行、生 HTML）は、block 全体ではなくその行の range を描画行に持たせる。block 全体の range だと、長いコードブロックの途中で読んでいてもリサイズや reload のたびに block の先頭へ戻り、`--line` も先頭に着地する。復元側の規則は `docs/render/runtime.md` の「レイアウトの再実行」にある。
- 未終端 `:::` container の body は **最初の空行で打ち切り**、diagnostic として報告する。markdown-it 系のように文書末尾まで伸ばすと、`::: mermaid` の閉じ忘れ 1 個で以降の本文全体が diagram body に吸収され、viewer として最も見たいものが読めなくなる。
- 閉じマーカー探索は code fence を認識する。fence 内の `:::` はリテラルであり closer ではない。fence 内の Azure 記法も同様にリテラルのまま残す。
- `[[_TOC_]]` は大文字小文字を区別し、最初の 1 個だけ展開する（2 個目は「無視された」と表示）。収集するのは `#` 見出しのみ。

## Diagram

diagram は **parse 直後・layout 前に一度だけ** render し、結果を `DiagramBlock::output` に保持する。frame loop と resize は renderer を呼ばない。`merman` 0.9 の幅の上限（`AsciiRequest::viewport` の `max_width`）は指定せず、出力幅は内容から決まるため、resize で再 render しても結果は同一になる。幅に合わせて描かせるときは、resize 経路から `diagram::resolve` を呼び直す。

- テキスト図の width profile（`MermaidWidth`）は、曖昧幅の文字を端末が何列で描くかに合わせる。既定は `Unicode`（1 列）。`Cjk`（2 列）では罫線も曖昧幅なので、`merman` が構造を ASCII で描く。どちらかは端末の設定で決まり、環境変数からは分からないので、判定せず設定で選ばせる。届くのは図の内側だけで、描画コア自身の枠・見出しの罫線は罫線文字のまま 1 列として測る。曖昧幅を 2 列で描く端末では、それらは崩れる。
- **未対応の判定はエラーの型か、`merman` の安定した diagnostic code で行う。** エラーの文字列は版で変わるので照合しない。作者のソースに誤りがないものを「テキストでは描けない図」（warning）とし、それ以外を描画エラー（error）にする。前者は次のとおり。
  - `AsciiError::UnsupportedDiagram` と core の `merman.parse.unsupported_diagram`: テキストで描けない図種。
  - `AsciiError::UnsupportedFeature`: 図種の一部の構造。flowchart では、辺が他のノードの配置を横切る図がこれになる。戻りの辺があるだけでは落ちない。
  - 出力の `projection` が `StructuredText` のもの: 図ではなく、parse した値を並べた要約になる図種（gantt・gitGraph・journey・kanban・mindmap・packet・timeline・treeView）。
  - `RenderError::ResourceLimitExceeded`: 作業量の上限を超えた大きな図。上限は `merman` の既定（interactive profile）のまま緩めない。PR head は信頼できない入力であるため。
  - 図がない（`NoDiagram` と `merman.parse.no_diagram_detected`）も同じ扱いにする。
- xychart は、グラフで正確に示せない値を、グラフの上に `xDomain:`・`values:` の行として書き出す。`merman` の出力をそのまま表示する（0.9.3 で確認）。
- **`merman` の呼び出しと、その SVG を PNG にする処理は `catch_unwind` で囲み、panic はその図だけの描画エラーにする。** 0.7 は erDiagram の非 ASCII の属性名で panic し、文書全体を表示できなかった。版を上げても panic しないとは保証されていない。捕まえても panic hook は走る（`docs/render/runtime.md` の「端末状態」）。
- 図種は `all-diagrams` ですべて有効にする。0.9.3 で確かめたところ、図種の feature を絞ると、外した図種（pie・gantt など）が「テキストでは描けない図」ではなく、Cargo の feature を有効にするよう促す描画エラーとして出た。画像の経路でも描けなくなる。ただし architecture は、外してある Cytoscape の layout が要るので、画像にできずテキストに戻る。

- 描画に失敗した diagram は diagnostic を出してソースへフォールバックし、文書の残りは通常どおり描画され、プロセスは exit 0 のまま。unsupported な diagram family は warning、構文エラーは error として分ける。
- `--mermaid image`（rasterize の境界・大きさ・解像度）の決定事項は `docs/render/mermaid-image.md` にある。
- `azure_compat.rs` の `RULES` は 4 件（`flowchart` root keyword、長い矢印、Font Awesome icon、label 内の HTML タグ）のまま維持する。Azure DevOps の Mermaid サポート仕様は変更が多く、追随コストが実利に見合わない。data-driven な構造は残し、必要が生じた時点で追加する。これは validator ではない。

## インライン画像

端末が描けるときだけ、**中身が `Inline::Image` 1 つだけの段落**を `Placement` を持つ空行の並び（`RenderedLine::image`）へ置き換える。IR は変えない。

- **文中に混ざった画像は placeholder のまま。** 折り返された行の中に複数行の矩形を置くことになり、wrap と行送りの前提が崩れる。単独段落だけなら「n 行を予約する」という単純な操作で済む。
- 予約行は全行に `source_range` を持たせる。`--line` と `--watch` のアンカーが行に紐づいているため。
- 画像化するかどうかは `InlineContext::images`（`highlight` と同じ扱い）で決まる。`None` のときの出力は画像機能が無かった頃と 1 バイトも変わらない。golden snapshot はこの状態を検証している。
- container（quote / list / details）は子行の `image` をそのまま引き継ぐ。落とすと予約された空行だけが残り、何も無い隙間になる。画像の開始桁は「その行が既に持っているテキストの表示幅」なので、prefix を足すだけで自然にずれる。
- **ファイルを読む条件は `image/mod.rs` に集約する。** content root 配下・スキーム無し・拡張子 allowlist・magic byte 一致・サイズ上限のいずれかを満たさなければ placeholder へ戻す。エラーにも exit code の変化にもしない。mdvu は文書中の宛先を開かないのが既定であり、画像だけが例外なので、その例外の範囲を 1 箇所で読めるようにしておく。
- **画像は、文書を読んだ Snapshot から読む。** `InlineContext::source`（`DocumentSource`）が Snapshot と文書のパスを運び、`None`（stdin）なら画像を読まない。root からの逸脱は Snapshot が、content root からの逸脱は `image::load` が `resolve` の結果で拒否する。content root と symlink の規則は `docs/design/p1-core.md` の「root と content root」「symlink と境界」に従う。`InlineContext::base_dir` は表示用の基準のパスで、読み取りには使わない。
- 画素寸法はヘッダから直接読む（`image/dimensions.rs`）。文書が指す画像のデコーダは持ち込まない（`resvg` の PNG encoder は diagram 専用）。プロトコルは元のバイト列を base64 で渡すだけなので、必要なのはセル数の計算に使う寸法だけ。

## StyleRole

span は色ではなく意味的な `StyleRole` を持ち、色は `layout/theme.rs` と backend が決める。新しい表示要素を足すときは色を直書きせず、`StyleRole` を増やして theme と両 backend（`output/ansi.rs`・`pager/view.rs`）に写像を追加する。両 backend は `theme.style(role)` が返す `Style` だけを写像しているので、既存の属性で表せる role なら theme の 1 箇所で足りる。

見出しは 6 段すべてを 16 色で塗り分けない。H1 / H2 / H3 / H4 以降の 4 段の強弱に留め、正確なレベルは行頭に必ず出る `#` の数が担う。見出しレベルをアイコンだけで表す方式（md-render.nvim）は、フォントが無い端末でレベルが読めなくなるため採らない。

## アイコン

Nerd Font のグリフは **`layout/icons.rs` だけに書く**。`merman` が `diagram/mermaid.rs` に閉じているのと同じ扱いで、他のモジュールは `IconSet` にプレフィックスを問い合わせて連結するだけにする。

- 既定は `IconSet::Unicode` で、プレフィックスは全て空文字列。**既定の出力は 1 バイトも変わらない**ことを golden snapshot が保証する（nerd 用の snapshot は `--icons nerd` を付けた別ファイルに分ける）。
- 自動検出はしない。フォントの有無は端末のプロファイルの問題で、環境変数からは分からず、端末へ問い合わせるのは禁じている。`--icons nerd` / `icons = "nerd"` の明示だけが有効化手段。
- グリフの後ろに必ず半角空白を置き、2 桁の枠として出す。PUA は East Asian Width Ambiguous で `display_width` は 1 を返すが、2 桁で描く端末もあるため、空白でラベルとの間隔を確保しつつずれを 1 桁に抑える。この前提は `icons.rs` の unit test（全グリフが 1 桁）が守る。
- **アイコンは色ポリシーに従わせない。** 画像や OSC 8 と違いエスケープ列ではなく普通の文字なので、`--color never` / `NO_COLOR` でも出す。
- 対象は alert ラベル・code fence の言語・画像 / 添付 placeholder に限る。見出しの `#` と箇条書きの `• ◦ ▪` はレベルと深さそのものを表しているので置き換えない。
- `StyleRole` は増やさない。アイコンは、それが付く既存 span と同じ role に載せる。

## 折り返しと禁則

禁則は **`chunk()` の結合で実現し、組み上がった行への後処理はしない**。行頭禁則文字は直前の chunk へ吸収し、行末禁則文字は直後の cell を自分の chunk へ引き込む。禁則違反の位置に break opportunity が生成されないため、規則の適用順という概念自体が存在しない。

後処理（追い出し・追い込み）方式には戻さない。2 つの規則が互いを打ち消し、`あいうえおかきく「け」` を幅 20 で折ると `「` が行末に露出していた。収束ループでも直せるが、入れ子リスト内では実効幅が 1 まで縮むため停止性を別途保証する必要が出る。結合方式は単一パスで完結する。

- **対象は全角約物・半角カナ・繰返し記号・小書き仮名・長音のみ。** ASCII 約物は入れない。Latin は空白区切りで折るので元々ほぼ no-op である一方、ASCII を分類すると語中に break opportunity ができ、`min_unbreakable_width` が意図より小さい値を返す。
- **幅よりも禁則を優先する。** 合法な分割位置が無ければ幅超過を許す（追い込み）。`tests/render.rs` の不変条件は「幅以内」ではなく「幅を超えるのは、その位置での分割が禁則違反になる場合だけ」。
- 判定用の文字集合は `tests/render.rs` にも複製してある。`layout/wrap.rs` の判定関数は crate の外に公開していないため、統合テストから import できない。片方だけ直さない。

## 表

列の最小幅は `wrap_spans(cell, 1)` ではなく `wrap::min_unbreakable_width`（break opportunity を持たない最長 run の幅）で求める。前者は収まらない run を強制分割するため最小幅が常に 1〜2 に潰れ、語中で折れた 3 桁幅の表が出た上に vertical fallback が到達不能になっていた。

## Syntax highlighting

`syntect` は **パーサとしてのみ** 使い、syntect のテーマは使わない。`ParseState` + `ScopeStack` のスコープを `StyleRole::Syntax(SyntaxKind)` の 7 種へ写像し、色は `theme.rs` が決める。16 色の列挙を truecolor へ広げずに済み、dark / light 両テーマ・plain backend・snapshot の安定性がそのまま保たれる。

- `SyntaxSet` は `OnceLock` で遅延初期化する。code block が無い文書では読み込まない。
- ハイライトは **タブ展開後** の文字列に対して行う。展開前だと列がずれる。
- スコープ解決は内側から外側へ走査し、`punctuation.definition` は透過させる。透過しないとコメントの `//` や文字列の引用符が comment / string ではなく punctuation になる。
- 分類できなかった範囲は `StyleRole::Code` ではなく `Normal`。前者だと識別子がすべて色付きになり、分類できたトークンが埋もれる。

## リンクの下線

下線は「宛先が実 URL である」ことを表す。`StyleRole::Link` は色だけを持ち、下線は **`RenderedSpan::link` を持つ span**（`http` / `https`）にだけ backend 側で付ける。リンクとして色が付く範囲（相対パス・`.attachments`・`#123`・`@alias`）は実 URL より広いため、すべて同じ下線だと区別できない。`output/ansi.rs` と `pager/view.rs` の両方で同じ条件を使い、`--hyperlinks` の有無で見た目は変わらない。

リンク化するのは `http` / `https` のみ。制御文字を含む dest はエスケープ列を閉じて任意の OSC を注入できるため、2083 バイト超の dest は壊れた入力である可能性が高いため、どちらもリンク化しない。

## Snapshot

レイアウトに影響する変更は golden snapshot が動く。`cargo insta review` で差分を目視し、意図した変化だけを受け入れる。新しい fixture は幅 40 / 80 / 120 の組み合わせで `tests/render.rs` に登録する。
