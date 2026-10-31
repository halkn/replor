#!/usr/bin/env bash
# P0 Spike S4（GitHub）: neovim の使い捨て clone に、PR head を refs/replor/ へ fetch する。
# 前後の状態（status・HEAD・index のハッシュ・for-each-ref）を spike/.work/ に書く。
# sandbox の中からは GitHub に届かないため、ユーザーが `!` で実行する。
set -eu
here=$(cd "$(dirname "$0")" && pwd)
spike="$here/.."
bin="$spike/target/release/s4-revision"
repo="$spike/.work/neovim"
work="$spike/.work"
prs=(39871 39791 39773 39735 39727 39672 39642 39628 39586 39559 39502 39496 39470 39391 39378 39258 39200 39181 39112 39073 39033 38999)

specs=()
for n in "${prs[@]}"; do
  specs+=("+refs/pull/$n/head:refs/replor/pr/$n/head")
done

"$bin" state "$repo" >"$work/s4-github-before.txt"
"$bin" fetch "$repo" https://github.com/neovim/neovim "${specs[@]}" | tee "$work/s4-github-fetch.txt"
"$bin" state "$repo" >"$work/s4-github-after.txt"
diff "$work/s4-github-before.txt" "$work/s4-github-after.txt" >"$work/s4-github-state.diff" || true
echo "--- 状態の差分（refs/replor/ 以外が出なければよい）"
cat "$work/s4-github-state.diff"
