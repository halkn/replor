# pager の決定事項

mdvu の pager（`replor` crate に置く。`docs/design/p1-core.md` の「mdvu の module の配置」）と、画像の判定を触る前に読む。文中のパスは mdvu の `src/` からの相対で、`--line`・`--images`・`--color` などは mdvu の CLI のフラグを指す。終了コードと設定ファイルは mdvu の CLI のものなので載せていない。replor の設定ファイルは `docs/design/p1-core.md` の「設定ファイル」に従う。

## 端末状態

`TerminalGuard` が raw mode / alternate screen / カーソル表示のすべての変更を所有し、`Drop` で戻す。正常終了・エラー・panic のいずれでも端末が使える状態で終わること。端末モードを他の場所で直接変更しない。マウスキャプチャは有効にしない。

描画コアは `merman` の panic を `catch_unwind` で捕まえて描画を続ける（`docs/render/rendering.md` の「Diagram」）。panic hook は捕まえる前に走る。既定の hook も stderr にメッセージを書くので、そのままでは捕まえた panic でも TUI の画面が崩れ、hook で端末を戻せば TUI が続けられなくなる。pager の panic hook は、この panic でも TUI が続くように書く。stdout の経路でも、既定の hook は文書の出力とは別に stderr へ panic のメッセージを書く。このメッセージは `RenderedSpan` の sanitize を通らない。

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
