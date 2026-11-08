# pager の決定事項

mdvu の pager（`replor` crate に置く。`docs/design/p1-core.md` の「mdvu の module の配置」）と、画像の判定を触る前に読む。文中のパスは mdvu の `src/` からの相対で、`--line`・`--images`・`--color` などは mdvu の CLI のフラグを指す。終了コードと設定ファイルは mdvu の CLI のものなので載せていない。replor の設定ファイルは `docs/design/p1-core.md` の「設定ファイル」に従う。

## 端末状態

`TerminalGuard` が raw mode / alternate screen / カーソル表示のすべての変更を所有し、`Drop` で戻す。正常終了・エラー・panic のいずれでも端末が使える状態で終わること。端末モードを他の場所で直接変更しない。マウスキャプチャは有効にしない。

描画コアは `merman` の panic を `catch_unwind` で捕まえて描画を続ける（`docs/render/rendering.md` の「Diagram」）。panic hook は捕まえる前に走る。既定の hook も stderr にメッセージを書くので、そのままでは捕まえた panic でも TUI の画面が崩れ、hook で端末を戻せば TUI が続けられなくなる。そのため pager の panic hook（`pager/mod.rs`）は、`diagram::mermaid::panic_is_caught()` が真なら何もしない。端末を戻さず、メッセージも書かない。stdout の経路でも、既定の hook は文書の出力とは別に stderr へ panic のメッセージを書く。このメッセージは `RenderedSpan` の sanitize を通らない。

## Files lens

replor では、mdvu の `pager/app.rs` の代わりに `crates/replor/src/app.rs` がイベントループを持ち、左にツリー、右に pager を置く。`pager/watch.rs` は `--watch`（P5）まで取り込まない。

- **幅の配分は固定にする。** ツリーは画面幅の 3 割で、20〜40 桁かつ半分以下にする。変える操作は持たない。
- **キーの振り分けは `app::route` が決める。**
  - 検索の入力・見出しの一覧・ヘルプが開いている間は、フォーカスに関わらず pager がキーを受ける。
  - それ以外では、`Tab`・`r`・`?`・`q`/`Esc`・`Ctrl-c` を app が受け、残りをフォーカスのあるペインに渡す。ツリーのキーは docsail、プレビューのキーは mdvu に揃える。ツリーには docsail に無い `h`（ディレクトリを閉じる。閉じたディレクトリやファイルでは 1 つ上へ移る）と `l`（ディレクトリを開く。ファイルでは `Enter` と同じく開く）を足した。
  - キーの正は `pager/help.rs` の `ENTRIES` で、各キーが載せたペインで割り当て済みであることをテスト（`every_advertised_key_is_bound`）で確かめる。
- **どのファイルも、描画コアの文書にしてプレビューに出す（`preview.rs`）。** 検索と見出しの一覧が、どのファイルでも同じ面で使える。
  - Markdown は `document::load` で読む。
  - 画像は 1 枚だけの文書にし、文書が指す画像と同じ検査を通す。
  - テキストは、拡張子を言語にした fenced code block にする。
  - 読めないものは、理由を一文で示す。
- **Markdown は、プレビューの `s` で Preview（描画）と Source（テキスト）を切り替える。** キーは replor-go の設計（`docs/plans/initial/DESIGN.md` の 4.2）と同じにした。
  - 切り替えられるのは Markdown だけ。テキストはもともとソースで出ており、画像にソースは無い。ほかのファイルで押すと、理由を link bar に出す。ほかの「動かなかった理由」と同じ置き場で、別のファイルを開くと消える。ステータスバーに置くと、次のファイルまで残る。
  - Source は、テキストのファイルと同じ経路（`preview::text`）で作り、同じ上限を使う。Markdown の読み取りの上限（64 MiB）のまま出すと、ハイライトで固まる。上限を超えるときは、Preview のまま理由を link bar に出す。理由だけの文書に切り替えると、読み位置を失う。
  - Source の文書は Source に切り替えるたびに作り、開いたファイルすべてではハイライトの費用を払わない。Preview は、Source を出している間も取っておき、戻るときに使う。戻るたびに読み直すと、Mermaid の図も描き直すことになる。ファイルを読み直すのは `r` だけで、Source のときは両方を読み直す。
  - 読み位置はファイルの行で引き継ぐ。描画行の数は表示によって違う。Source の文書は先頭に fence の 1 行が足されるので、`preview::Content::line_offset` で文書の行とファイルの行を変換する。Back / Forward が覚える行と `App::top_line` も、ファイルの行である。
  - 別のファイルを開くと（ツリー・リンク・Back / Forward のどれでも）Preview に戻る。リンクをたどれるのは Preview だけで、モードが残るとリンクで読み進められない。このため、Back / Forward の履歴はモードを持たない。`r` は今のモードのまま読み直す。
  - ステータスバーの種別は、Source のとき `source` になる。
- **テキストは 1 MiB・5,000 行まで出す。** 開いたときに同期でハイライトをかけ、syntect は約 0.1 ms／行かかる（ADR-0001 S3）。超えたものと、先頭 8000 バイトに NUL があるもの・UTF-8 でないものは、内容を出さずに理由を出す。
- ファイル名は `text::visible` で制御文字を無害化してから描く。ファイル名から作る Markdown は、記号をすべてエスケープしてリンクや画像にならないようにする。

## リンクと Back / Forward

- **見出しの ID とリンクの索引は IR から作る（描画コアの `navigation.rs`）。** 描画結果の文字列は検索しない。
  - 見出しの ID は github-slugger 互換にする。除く文字は、同梱した github-slugger の `regex.js`（`crates/replor-render/data/github-slugger/`）を、初めて使うときに文字の範囲の表へ変換して決める。表を手で書かないため。
  - どのリンクかは `LinkKey`（リンクの元の byte offset。TOC の項目は見出しの offset）で表す。layout は、リンクの文字の span に同じ `LinkKey` を付け、折り返しても保つ。同じ行に同じリンク先が並んでも、選んだ 1 つだけを示せる。
  - 画面外のリンクを選んだときにスクロールする先は、IR が持つ block の範囲（表は行の範囲）で描画行を探して決める。`[[_TOC_]]` の項目は見出しの範囲を持つので、TOC の先頭の行より後で探す。
- **リンクは、プレビューで `f`/`F` で順に選び、`Enter` でたどる。** `Tab` はペインの切り替えに使っている。
  - 最初の選択は、画面に見えている範囲から始める。
  - 選んだリンクの文字だけを反転する。
  - リンク先は、ステータスバーの上の 1 行（link bar）に `→` を付けて出し、右に何番目のリンクかを出す。link bar は本文の最後の行に重ねるので、選んでも描き直しは起きない。
  - ほかの操作でスクロールすると、選択と link bar は消える。
- **リンクは Snapshot のパスで解決する（`crates/replor/src/navigate.rs`）。** Snapshot の外を指すリンク（`../` で root を出る、外を指す symlink）、存在しないファイル・見出し、URL は、たどらない。理由は link bar に `!` を付けて警告の色で出す。URL は開かない。別のファイルの見出しへのリンクは、見出しがあると確かめてから開く。
- **Back / Forward（`[`/`]`）は、ファイルとソースの行の組で覚える。** 描き直しで描画行がずれても、同じ位置に戻る。リンクをたどったときだけ記録し、ツリーから開いたときは記録しない。各方向 100 件までで、古いものから捨てる。

## レイアウトの再実行

ドキュメントは起動時に一度だけ parse し、layout はリサイズと reload のときだけ再実行する。画像読込と構文ハイライトは文書単位で準備し、reload で更新する。frame ごとに layout や diagram 描画を走らせない。reload は起動時と同じ解析・リソース準備を通し、再読込した文書を開き直した文書と区別できないようにする。parse から diagram 描画までの手順は `document::build` にだけ書く。

- **読み位置は「描画行が示す source range・その range の何番目の連なりか・連なりの中の位置」で覚える（`pager/state.rs` の `Anchor`）。** 行番号だけでは、折り返した段落やコンテナの区切り行のように 1 つの range が複数の描画行を持つとき、復元先がその先頭へ戻る。幅が変わって連なりの長さが変わったときは、連なりの中の位置を比例で保つ。reload で byte がずれて同じ range が見つからないときは、range の開始行から探し直す。
- source 行から描画行を引くとき（`--line`・見出し一覧・上記の探し直し）は、その行を覆う描画行のうち **range が最も狭いもの** を選ぶ。コンテナ（リスト項目・引用・alert・`<details>`）の区切り行とタイトル行はコンテナ全体の range を持って中身より前に出るので、最初に覆う行を選ぶと中身へ届かない。

## File watch

`notify` の `RecommendedWatcher` を `std::sync::mpsc` で受け、既存の `crossterm::event::poll`（250ms）ループのタイムアウト側で回収する。async runtime は追加しない。

- **監視対象はファイルではなく親ディレクトリ。** エディタや Coding Agent の atomic save（一時ファイル → rename）は inode を差し替えるため、ファイル自体への watch は静かに外れる。イベントは file name で絞り込む。
- イベント種別は `EventKind::Access` だけを捨て、他はすべて変更として扱う。backend ごとに分類の粒度が違い、取りこぼしより余分な再読込の方が安い。
- デバウンスは 100ms。`notify-debouncer-*` は追加せず、`Instant` を引数に取る純粋な `Debounce` として持ち、タイマ非依存にテストする。
- 読み込み失敗（一時的な truncate、消失、非 UTF-8 への差し替え）は fatal にせず、status bar に出して直前の描画を保持する。

### watch テストが sandbox で落ちる

`a_write_to_the_file_is_noticed` と `an_atomic_save_is_noticed` は実際にファイルを書き換えて FSEvents を待つため、sandbox 内ではイベントが 1 件も届かず失敗する。**sandbox の制約であってコードの不具合ではない**（`PollWatcher` に差し替えると同じ 2 件が通る）。この 2 件を無効化したり `PollWatcher` へ切り替えたりせず、CI で検証する。

FSEvents は watch 開始直前の書き込みを起動後に報告することがあるため、テストは `Watch::new` 直後の積み残しを `settle` で捨ててから本題の操作を行う。

## Overlay と検索プロンプト

- **キー一覧の正は `src/pager/help.rs`。** README のキー表はそちらに追従させる。pager の中から README は読めないので、overlay 自体が完結している必要がある。表に載せたキーが実際に bind されていることは unit test で担保する。
- overlay（`t` の見出し一覧、`?` のキー一覧）は開いている間すべてのキーを所有する。背後の本文は動かさない。端末が低くて全項目が入らない場合もリストの末尾へ到達できること。
- **検索はインクリメンタル。** `/` を押した時点の `top` / `left` を退避し、打鍵のたびにそこから検索し直す。1 文字ごとに現在位置から先へ進める方式にしない（打つほど viewport が流れる）。
- プレビューの一致集合は確定済みの `Search` と別に持つ。`Esc` は viewport と確定済み検索の両方を押し戻し、`Enter` は画面に出ているものを確定する。
- プロンプトの caret はグラフェム境界で動かす。端末カーソルは `TerminalGuard` が非表示のまま所有するので、caret は status bar の反転セルとして描く。カーソル表示を切り替えない。
- 現在の一致は他の一致と別の `StyleRole`（`CurrentMatch`）で描く。すべて同じ反転だと、`n` がどこへ移ったのかが画面から読めない。
- 照合は smart case（クエリが全て小文字なら case insensitive、大文字を含めば case sensitive）。lowercase 後の一致範囲は原文の文字境界へ写像する。総バイト長が同じでも途中の位置が一致するとは限らない。

## OSC 8

ハイパーリンクは stdout backend（`output/ansi.rs`）だけが出力する。`ratatui` 0.30 の `Cell` はハイパーリンク属性を持たず、symbol にエスケープ列を埋めると差分描画で閉じ側が出力されず画面全体がリンク扱いになる。pager 内では下線付きテキストに留める。

## 画像プロトコル

`ratatui` の `Cell` に画像は入らないので、pager では **フレーム描画後に stdout へ直接**エスケープを出す（`pager/images.rs`）。`view.rs` は buffer にしか書けないため、この処理だけ backend の外側にある。

- 端末に能力を問い合わせない。判定は環境変数のみ（`TERM` / `TERM_PROGRAM` / `KITTY_WINDOW_ID` / `KONSOLE_VERSION`）。DA1 や kitty query は応答待ちが要るので使わない。`--images <protocol>` が唯一の強制手段。
- tmux 配下では `auto` を off にする。passthrough の可否が外側の端末と tmux のバージョンで決まり、環境変数からは分からない。
- 画像はエスケープ列なので、`--color never` / `NO_COLOR` では出さない。`auto` は OSC 8 と同じく TTY を要求する。
- **配置集合が前フレームと同じなら何も送らない。** `draw` は 250ms ごとに回るため、毎回送るとちらつき、数 MB の payload を繰り返し流すことになる。比較は `Rc::ptr_eq` と画面座標で行い、バイト列は比較しない。
- 画像の格納・移動・消去・切り出し（Kitty と iTerm2 の分岐）は `docs/render/mermaid-image.md` の「pager の kitty 描画」に従う。

## テスト

pager の対話部分は自動テストできない。画像の配置計算は `app.rs` のループではなく `pager/images.rs` の純粋関数に置き、そこを unit test で固める。状態遷移は `state.rs` / `event.rs` の純粋な unit test で固め、描画は `ratatui` の `TestBackend` で 1 フレーム描いて検証する。ロジックを `app.rs` のイベントループへ書き足すのではなく、`state.rs` の純粋関数側へ寄せる。
