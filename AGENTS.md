# AGENTS.md

replor は、ローカルのリポジトリや Wiki のクローンと PR head を、同じエクスプローラで読む TUI である（Rust 2024 edition / Ratatui / Crossterm）。背景と方針は `docs/design/concept.md` にある。

## 現在のフェーズ

P0 Spike は完了し、いまは P1 Core ＋ Explorer（`docs/design/concept.md` のフェーズ）。crate の構成・Snapshot の trait・mdvu の取り込み・`tests/render.rs`・設定ファイルを触る前に `docs/design/p1-core.md` を読む。描画コア（`crates/replor-render/`）と、mdvu から取り込む pager を触る前に `docs/render/` の該当する文書を読む。描画コアの依存と feature を変える前に `docs/render/dependencies.md` を読む。画像の層・merman の版・Diff の描き方・Git の読み取り方式を触る前に `docs/adr/0001-p0-spike.md` を読む。検証の根拠と、未検証の環境（WSL・Azure DevOps）を確かめるスクリプトは `docs/spike/` にある。

## 検証

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features --locked`
- `bash .github/scripts/check-render-deps.sh`（描画コアに禁止した crate が入っていないこと）

## 常に守ること

- 検証に使うリポジトリの作業ツリー・index・ブランチ・Git 設定を変更しない。fetch で書く ref は専用の名前空間に限る。
- 参照先のリポジトリ（mdvu・docsail・replor-go・herdr-reviewr）は読むだけにする。mdvu を変えるときは捨てるブランチで行い、main に入れない。
- タスクの範囲外の整形やリファクタリングを混ぜない。
