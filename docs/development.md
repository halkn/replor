# 開発環境と検査

ローカルと CI は同じ `make` ターゲットを実行する。CI（`.github/workflows/ci.yml`）は `make ci` を呼ぶだけなので、検査を足すときは Makefile に足す。

## 必要なもの

- Go（版は問わない。`make` が go.mod の `toolchain` 行の版へ切り替え、なければ go コマンドがダウンロードする）
- make
- git（テストが一時リポジトリを作る）

他のツールマネージャーや、事前のツール導入は要らない。clone 直後に `make setup` を実行すると、固定した版の Go・依存モジュール・staticcheck をまとめて取得する。取得しなくても、各ターゲットが必要に応じて取得する。

## ターゲット

| コマンド | 内容 |
|---|---|
| `make setup` | 固定した版の Go・依存モジュール・staticcheck を取得する |
| `make ci` | `fmt-check`・`lint`・`test-race`・`build` をまとめて実行する。push 前と CI で実行する |
| `make fmt` | gofmt で整形する |
| `make fmt-check` | 整形が必要なファイルがあれば失敗する |
| `make lint` | `go vet` と staticcheck |
| `make test` / `make test-race` | テスト（race detector あり／なし） |
| `make build` | `CGO_ENABLED=0` でビルドする |
| `make load` | 負荷計測（`REPLOR_LOAD=1`）。結果は ADR-0001 3.5 の数値と比べる |

## 版の管理

| 対象 | 置き場所 | 更新方法 |
|---|---|---|
| Go | `go.mod` と `tools/go.mod` の `toolchain` 行（同じ版にそろえる） | `make update-go VERSION=X.Y.Z` |
| Go の対応下限 | `go.mod` の `go` 行 | 依存が要求する版より下げない |
| staticcheck | `tools/go.mod` の `tool` 行 | `make update-staticcheck VERSION=vX.Y.Z` |
| GitHub Actions | `.github/workflows/ci.yml` | メジャー版で指定する |

- `toolchain` 行は下限としてしか効かないため、Makefile がその値を `GOTOOLCHAIN` に設定して版を固定する。Makefile を通さずに `go` を直接実行すると、手元の Go が新しい場合はそちらが使われる。
- 開発用ツールは、アプリとは別の `tools/go.mod` に置く。ツールの依存が、アプリの依存グラフ（`golang.org/x/*` など）を引き上げないようにするため。
- Go の新しいマイナー版へ上げるときは、staticcheck がその版に対応しているかを先に確認する。

## lint の範囲

`go vet` と staticcheck に限定し、golangci-lint は使わない。設定の移行と linter の入れ替えに追従する保守コストに比べ、上乗せできる検出が小さいため。安全性の要（Workspace 境界・Git 実行契約・端末出力の無害化）は、lint ではなく悪意ある入力を使ったテストで保証する。エラー戻り値の無視の検出（errcheck）などが必要になったら、乗り換えを検討する。staticcheck の検査は golangci-lint でも同じ ID で動くので、乗り換えても検出規則を引き継げる。

## CI のジョブ

| ジョブ | 内容 |
|---|---|
| `check` | Ubuntu と macOS のランナーで `make ci` |
| `linux-distro` | `ubuntu:24.04` コンテナで実行する。同梱 Git でのテスト、静的バイナリの起動、`make load` |
