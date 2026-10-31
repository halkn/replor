#!/usr/bin/env bash
# P0 Spike S1: 端末の環境変数を記録し、mdvu の --images を auto → kitty → iterm2 の順に試す。
#
# 使い方: docs/spike/p0/scripts/check-images.sh <環境名>
#   環境名の例: ghostty / ghostty-herdr / noctty / noctty-herdr / wezterm / wezterm-herdr
# mdvu は PATH 上のものを使う。別のものを使うときは MDVU=/path/to/mdvu を付ける。
# 結果は カレントディレクトリの s1-<環境名>.md に追記する。
set -u

label=${1:?環境名を指定してください（例: ghostty-herdr）}
here=$(cd "$(dirname "$0")" && pwd)
fixtures="$here/../fixtures"
mdvu=${MDVU:-mdvu}
out="$PWD/s1-$label.md"

if ! command -v "$mdvu" >/dev/null 2>&1; then
  echo "mdvu が見つかりません（MDVU=... で指定できます）" >&2
  exit 1
fi

show() {
  local name=$1
  local value
  value=$(printenv "$name" 2>/dev/null) || value="(未設定)"
  printf '| `%s` | `%s` |\n' "$name" "$value"
}

# mdvu の src/image/mod.rs の protocol_from_env と同じ判定（v0.10.2）。
mdvu_auto() {
  if [ -n "${TMUX:-}" ]; then echo "off（TMUX）"; return; fi
  if [ -n "${KITTY_WINDOW_ID:-}" ] || [ -n "${KONSOLE_VERSION:-}" ] ||
    [ "${TERM:-}" = xterm-kitty ] || [ "${TERM:-}" = xterm-ghostty ]; then
    echo kitty; return
  fi
  case "${TERM_PROGRAM:-}" in
    ghostty | WezTerm) echo kitty ;;
    iTerm.app) echo iterm2 ;;
    *) echo "off（判定できない）" ;;
  esac
}

ask() {
  local prompt=$1 answer
  read -r -p "$prompt [y/n/-] " answer
  case "$answer" in
    y | Y) echo "可" ;;
    n | N) echo "不可" ;;
    *) echo "-" ;;
  esac
}

{
  echo "# S1: $label"
  echo
  echo "- 日時: $(date '+%Y-%m-%d %H:%M')"
  echo "- uname: \`$(uname -srm)\`"
  echo "- mdvu: \`$("$mdvu" --version 2>&1)\`"
  if [ -n "${WSL_DISTRO_NAME:-}" ]; then echo "- WSL: \`$WSL_DISTRO_NAME\`"; fi
  echo "- mdvu の auto が選ぶ protocol（環境変数からの推定）: $(mdvu_auto)"
  echo
  echo "| 環境変数 | 値 |"
  echo "|---|---|"
  for name in TERM TERM_PROGRAM TERM_PROGRAM_VERSION COLORTERM KITTY_WINDOW_ID KONSOLE_VERSION \
    TMUX WEZTERM_EXECUTABLE WEZTERM_PANE GHOSTTY_RESOURCES_DIR WT_SESSION WSLENV WSL_DISTRO_NAME; do
    show "$name"
  done
  env | grep -i '^herdr' | sed 's/^\([^=]*\)=\(.*\)$/| `\1` | `\2` |/'
  echo
} | tee "$out"

cat <<'EOF'

これから mdvu を 3 回起動します（--images auto → kitty → iterm2）。
それぞれで次を確認し、q で終了してから質問に答えてください。
  1. flowchart・sequenceDiagram・erDiagram が画像で出るか（日本語ラベルが読めるか）
  2. j/k・Space/b でスクロールし、図が上下端にかかったときに残骸や欠けが出ないか
  3. 表示できたら、図が 3 つとも入るようにスクリーンショットを撮る
     （ファイル名の例: s1-<環境名>-<mode>.png）
EOF

{
  echo "| --images | 図の表示 | 日本語ラベル | スクロール時の残骸なし | 終了後の残骸なし | メモ |"
  echo "|---|---|---|---|---|---|"
} >>"$out"

for mode in auto kitty iterm2; do
  echo
  read -r -p "Enter で mdvu --mermaid image --images $mode を起動します" _
  "$mdvu" --mermaid image --images "$mode" --paging always "$fixtures/mermaid-ja.md"
  shown=$(ask "[$mode] 図が画像で表示されたか")
  japanese=$(ask "[$mode] 日本語ラベルが読めたか")
  scroll=$(ask "[$mode] スクロールで残骸・欠けが出なかったか")
  after=$(ask "[$mode] 終了後の画面に画像が残らなかったか")
  read -r -p "[$mode] メモ（任意）: " memo
  echo "| $mode | $shown | $japanese | $scroll | $after | $memo |" >>"$out"
done

cat <<'EOF'

最後に、長い文書でスクロールを確かめます（図と画像が上下端をまたぐ配置）。
表示できた --images の値を入力してください（どれも出なければ Enter で終了）。
EOF
read -r -p "--images の値: " best
if [ -n "$best" ]; then
  "$mdvu" --mermaid image --images "$best" --paging always "$fixtures/long-ja.md"
  scroll=$(ask "[long-ja.md / $best] 上下端をまたぐ図・画像に残骸や欠けが出なかったか")
  read -r -p "メモ（任意）: " memo
  {
    echo
    echo "- 長い文書（--images ${best}）: スクロール時の残骸なし=${scroll}。$memo"
  } >>"$out"
fi

echo
echo "記録: $out"
