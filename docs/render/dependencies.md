# 描画コアの依存

`crates/replor-render/Cargo.toml` の依存と feature を変える前に読む。どの crate を描画コアに入れてはならないかは `docs/design/p1-core.md` の「描画コアの禁止」にある。

- runtime 依存に Node.js / Chromium / WebView / 外部コマンドを持ち込まない。JavaScript engine を含む crate（`boa` / `v8` / `quickjs`）も入れない。
- 新しい外部 crate は 1 module に隔離する。`merman` は `diagram/mermaid.rs`、`resvg` は `diagram/raster.rs`、`syntect` は `layout/highlight.rs` が唯一の接触面。差し替えや不具合対応はその境界の内側で行う。
- feature は最小限に固定してある。
  - `merman` は `default-features = false` で、`ascii`・`svg`（SVG まで）・`all-diagrams` のみ。`png` は持ち込まない。SVG 中の `<image>` が指すローカルファイルを読む resolver を使うため（`docs/render/mermaid-image.md` の「rasterize の境界」）。既定の feature が入れる ELK・Cytoscape の layout と math も入れない。`all-diagrams` にする理由は `docs/render/rendering.md` の「Diagram」。
  - `resvg` は `text` / `system-fonts` / `memmap-fonts` のみで、`raster-images` は入れない（図中の画像は描かない）。
  - `syntect` は `parsing` / `default-syntaxes` / `regex-fancy` のみ。C 依存の `onig` を避け、色は `layout/theme.rs` が決めるので `default-themes` は要らない。
- `RUSTFLAGS: -D warnings` を CI 全体に置かない。依存 crate のコンパイルにも適用され、どれかが警告を出すと落ちる。lint gate は `cargo clippy -- -D warnings` に任せる。
