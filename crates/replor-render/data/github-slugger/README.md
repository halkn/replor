# github-slugger

[github-slugger](https://github.com/Flet/github-slugger) v2.0.0（commit 3461c43）の次のファイルを、変更せずに複製したもの。halkn/replor-go の `testdata/github-slugger/` から取った。ライセンスは同梱の `LICENSE`（ISC）。

| ファイル | 元の場所 | 使い道 |
|---|---|---|
| `regex.js` | `regex.js` | 見出しから除く文字。`src/navigation.rs` が `include_str!` で読み、最初に使うときに文字の範囲の表へ変換する |
| `fixtures.json` | `test/fixtures.json` | 見出し ID の期待値。`navigation::tests::every_github_slugger_fixture_matches` が照合する |

更新するときは、両方を同じ版から取り直す。`regex.js` が変換できない構文を使うと、テストが失敗する。
