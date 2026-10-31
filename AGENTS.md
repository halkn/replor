# AGENTS.md

replor は、ローカルのリポジトリや Wiki のクローンと PR head を、同じエクスプローラで読む TUI である（Rust 2024 edition / Ratatui / Crossterm）。背景と方針は `docs/design/concept.md` にある。

## 現在のフェーズ

P0 Spike（`docs/design/spike-p0.md`）。本実装の crate はまだない。Spike のコードは `spike/` の下に置き、S ごとに独立した crate とする。本実装から参照しない。

## 検証

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features`

## 常に守ること

- 検証に使うリポジトリの作業ツリー・index・ブランチ・Git 設定を変更しない。fetch で書く ref は専用の名前空間に限る。
- 参照先のリポジトリ（mdvu・docsail・replor-go・herdr-reviewr）は読むだけにする。mdvu を変えるときは捨てるブランチで行い、main に入れない。
- タスクの範囲外の整形やリファクタリングを混ぜない。
