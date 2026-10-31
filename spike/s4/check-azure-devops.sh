#!/usr/bin/env bash
# P0 Spike S4（Azure DevOps）: PR の ref 名を確かめ、PR head を専用の名前空間へ fetch する。
#
# 使い方: spike/s4/check-azure-devops.sh <clone の URL> <PR 番号> [PR の source commit の OID]
#   - 壊れても困らない clone を ./s4-ado-work/repo に新しく作る（既存の clone には触れない）
#   - source commit の OID は PR 画面か `az repos pr show --id <N> --query lastMergeSourceCommit.commitId`
#     で分かる。渡すと、OID を直接 fetch できるかも試す（Open Questions Q5）
# 結果は ./s4-ado-result.md に書く。
#
# git の呼び出しは spike/s4-revision/src/contract.rs と同じ実行契約（旧 replor の DESIGN.md 7.3）
# を shell で組んだもの。Rust が無い環境でも動かせるようにしている。
set -u

url=${1:?clone の URL を指定してください}
pr=${2:?PR 番号を指定してください}
oid=${3:-}
work="$PWD/s4-ado-work"
repo="$work/repo"
out="$PWD/s4-ado-result.md"

if [ -e "$work" ]; then
  echo "$work が既にあります。消してから実行してください" >&2
  exit 1
fi
mkdir -p "$work"

# 許可リストの環境変数（DESIGN.md 7.3）と、fetch に要るプロキシ・ssh-agent だけを渡す。
contract_git() {
  local pass=()
  local name
  for name in PATH HOME USER LOGNAME XDG_CONFIG_HOME TMPDIR SYSTEMROOT \
    HTTPS_PROXY https_proxy HTTP_PROXY http_proxy ALL_PROXY all_proxy NO_PROXY no_proxy SSH_AUTH_SOCK; do
    if [ -n "${!name:-}" ]; then pass+=("$name=${!name}"); fi
  done
  # filter driver を呼び出しのたびに列挙し直して無効にする。
  local drivers count=0 cfg=()
  drivers=$(env -i ${pass[@]+"${pass[@]}"} git -C "$repo" config --name-only --get-regexp '^filter\.' 2>/dev/null |
    sed -E 's/^filter\.(.*)\.[^.]+$/\1/' | sort -u)
  for d in $drivers; do
    for kv in "clean=" "smudge=" "process=" "required=false"; do
      cfg+=("GIT_CONFIG_KEY_$count=filter.$d.${kv%%=*}" "GIT_CONFIG_VALUE_$count=${kv#*=}")
      count=$((count + 1))
    done
  done
  env -i ${pass[@]+"${pass[@]}"} ${cfg[@]+"${cfg[@]}"} GIT_CONFIG_COUNT=$count LC_ALL=C GIT_OPTIONAL_LOCKS=0 \
    GIT_TERMINAL_PROMPT=0 GIT_PAGER=cat PAGER=cat \
    git -C "$repo" --no-pager --no-optional-locks --literal-pathspecs --no-lazy-fetch \
    -c diff.autoRefreshIndex=false -c core.fsmonitor=false -c log.showSignature=false \
    -c core.quotePath=false -c color.ui=false "$@"
}

state() {
  local index
  index=$(contract_git rev-parse --path-format=absolute --git-path index)
  echo "## git status --porcelain=v2 --branch"
  contract_git status --porcelain=v2 --branch --untracked-files=all
  echo "## git rev-parse HEAD"
  contract_git rev-parse HEAD
  echo "## git symbolic-ref HEAD"
  contract_git symbolic-ref HEAD
  echo "## index hash (git hash-object --no-filters)"
  contract_git hash-object --no-filters -- "$index"
  echo "## git for-each-ref"
  contract_git for-each-ref --format='%(objectname) %(refname)'
}

fetch_into() {
  contract_git -c gc.auto=0 -c maintenance.auto=false -c fetch.writeCommitGraph=false \
    fetch --no-tags --no-write-fetch-head --no-recurse-submodules --no-auto-gc \
    --no-auto-maintenance --no-write-commit-graph origin "$@" 2>&1
}

echo "clone しています: $url → $repo"
git clone --quiet "$url" "$repo" || exit 1

{
  echo "# S4: Azure DevOps"
  echo
  echo "- 日時: $(date '+%Y-%m-%d %H:%M')"
  echo "- git: \`$(git --version)\`"
  echo "- PR: $pr"
  echo
  echo "## PR の ref（git ls-remote origin 'refs/pull/$pr/*'）"
  echo
  echo '```text'
  contract_git ls-remote origin "refs/pull/$pr/*" 2>&1
  echo '```'
  echo
  echo "## PR の ref の総数（git ls-remote origin 'refs/pull/*' の種類別）"
  echo
  echo '```text'
  contract_git ls-remote origin 'refs/pull/*' 2>&1 | awk '{print $2}' | sed -E 's#refs/pull/[0-9]+/##' | sort | uniq -c
  echo '```'
} >"$out"

state >"$work/before.txt"

{
  echo
  echo "## fetch"
  echo
  for ref in $(contract_git ls-remote origin "refs/pull/$pr/*" | awk '{print $2}'); do
    name=${ref#refs/pull/$pr/}
    echo "### +$ref:refs/replor/pr/$pr/$name"
    echo
    echo '```text'
    fetch_into "+$ref:refs/replor/pr/$pr/$name"
    echo "exit=$?"
    echo '```'
    echo
  done
  if [ -n "$oid" ]; then
    echo "### OID を直接 fetch: $oid"
    echo
    echo '```text'
    fetch_into "+$oid:refs/replor/pr/$pr/by-oid"
    echo "exit=$?"
    echo '```'
    echo
  fi
} >>"$out"

state >"$work/after.txt"

{
  echo "## 前後の状態の差分（refs/replor/ 以外が出なければよい）"
  echo
  echo '```diff'
  diff "$work/before.txt" "$work/after.txt"
  echo '```'
  echo
  echo "## merge ref の親（merge なら 1 つ目が target、2 つ目が PR の source）"
  echo
  echo '```text'
  contract_git log -1 --format='%H %P' "refs/replor/pr/$pr/merge" 2>&1
  echo '```'
} >>"$out"

echo "記録: $out"
