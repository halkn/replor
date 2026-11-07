# `--mermaid image` と kitty 描画の設計

`--mermaid image` で Mermaid 図を PNG にして端末へ描く経路と、pager が kitty graphics protocol で画像を扱う方法の決定事項。`crates/replor-render/` の `src/diagram/raster.rs`・`src/diagram/mermaid.rs` の画像化・`src/image/` と、pager の `pager/images.rs` を触る前に読む。文中の `--mermaid`・`--images` などは mdvu の CLI のフラグを、`Cli::mermaid_mode` は mdvu の module を指す。replor では、画像を出さない `render_to_string` が同じ判定を持つ。`replor` crate から画像を描くときも、同じく画像が off なら `unicode` に落とす。

## 有効になる条件

- `--mermaid image` は画像を描ける端末でだけ有効にする。判定は `Cli::mermaid_mode` の 1 箇所で、画像が off（`--images never`・`--color never`・パイプ・未対応の端末）なら `unicode` として resolve する。
- PNG 化に失敗した diagram はテキスト描画へ落とし、エラー報告はテキスト側の経路に任せる。
- PNG は文書準備時に `ImageAsset` に保持し、layout では幅に応じた `Placement` にして単独画像の段落と同じ予約行にする。

## rasterize の境界

- **rasterize は `merman` の `raster` feature ではなく `diagram/raster.rs` が持つ。** `merman` の raster helper は `usvg` の既定 href resolver を使い、image shape（`A@{ img: "/abs/path" }`）や C4 の `<image>` が指すローカルファイルを content root の外でも読む（merman 0.7.0 で確認）。文書が `mdvu` に任意のファイルを読ませられることになる。
- `raster.rs` は href を一切解決しない resolver を渡し、`data:` URL も読まない。図の中の画像は描かない。
- `resvg` の `raster-images` を入れていないので、PNG を指されても描けず、読まれたかどうかが画素に現れない。この境界のテストが SVG ファイルを使うのはそのため。
- system font の走査は 1 図あたりの費用の大半なので、`fontdb` は `OnceLock` で 1 プロセス 1 回だけ読む。

## 大きさと解像度

- 図の表示サイズは、端末のセル画素数でも PNG の画素数でもなく **CSS 幅（1 列 = 8 px）** で決める（`DiagramPng::css_width` → `image::from_png`）。高密度ディスプレイの端末は物理ピクセルでセル寸法を返すため、画像ファイルと同じ `fit` にかけると図がブラウザの半分の大きさになる。行数上限（`MAX_ROWS`）は画像ファイルと共通。
- PNG は最大 2 倍で描き、**400 万画素と 1 辺 8192 px に収まるよう倍率を下げる**。等倍だと高密度ディスプレイで引き伸ばされてぼやける。倍率が図ごとに変わるので、表示サイズを画素数から逆算してはいけない。
- 画素数の上限は Ghostty 1.3.1 の実測による。3076×2078（640 万画素）の PNG は何も言わずに描かれず、2500×2000（500 万画素）は描かれた。Ghostty のソースが宣言する上限（1 辺 10000 px・400 MB）より低い。
- 背景は白で塗る。Mermaid の既定テーマは透明背景に暗い線で、dark 端末では消える。`COLORFGBG` のヒントでテーマを出し分けると、判定が外れたときに読めなくなる。

## pager の kitty 描画

- **画像データは 1 回だけ送り、スクロールでは配置し直すだけにする。** 初回に `a=t,i=<Placement::id>` で格納し、以降は `a=p,i=<id>,p=1` を送る。位置が 1 行変わるたびに全 payload を送り直すと、2 倍解像度の図では 1 図あたり数百 KB になり、スクロールが目に見えて遅れる。送信済みの id は `pager/images.rs` が持つ。ID は画像データごとに文書単位で保持し、リサイズでも変えない。
- **全コマンドと chunk の続きに `q=2` を付ける。** `i=` に対する端末の応答は入力として届き、pager は `ESC _Gi=1;OK` の `G` を「末尾へ移動」と読む。仕様上、続きの chunk は `m` と `q` だけを持てる。どの chunk の `q` を見て応答を決めるかは端末によって違いうる。
- **移動は placement id の置き換えで行い、`d=a` を使わない。** 同じ `i`・`p` の `a=p` は前の配置を置き換える。画面から外れた画像だけ `a=d,d=i,i=<id>,p=1` で消し、データは残す。Ghostty 1.3.1 では、`d=a` のあとに同じ画像を `a=p` で置き直すと描かれなかった（格納前に `d=a` した場合は描かれた。どちらも実測）。
- reload・終了時は、画像セッションが送信済みの id ごとに `a=d,d=I,i=<id>` でデータごと解放する。`d=A` は画面上に配置のある画像しか解放せず、画面外へ出した図のデータが残る。リサイズでは同じ ID のまま解放してから送り直す。`ratatui-core` 0.1.2 はリサイズで画面全体を消去（`ClearType::All`）し、kitty graphics protocol はこの消去で画像も消すと定めているため、格納済みのデータが残る前提を置けない。終了時の解放は、正常終了・エラー・panic のどれでも `TerminalGuard` が alternate screen を抜ける前に行う。kitty は画面ごとに画像を持つので、抜けた後に送ると通常画面にある同じ ID の無関係な画像を消しうる。panic hook は unwind より先に画面を戻すため、送信済みの ID は画像セッションの `Drop` ではなく `pager/images.rs` のスレッドローカルに置き、hook からも読めるようにしている。
- 上下の端をまたぐ画像は、画面内の行だけを `a=p` の source rectangle（`x,y,w,h`、元画像の px）で切り出して描く。データは格納済みなので、切り出しに再送は要らない。20 行の図は高さ 24 行の画面では先頭が上 3 行にある間しか丸ごと収まらず、丸ごと収まるときだけ描くと、スクロール中に図が出たり消えたりする。左右にはみ出す画像は描かない。
- iTerm2 は格納も削除も切り出しもできない。毎回全体を送り、丸ごと収まるときだけ描き、画像が変わるフレームでは `Terminal::clear()` で全再描画する。この分岐は `images::needs_repaint` と `Placement::stored` だけで表す。
