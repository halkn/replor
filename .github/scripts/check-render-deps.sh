#!/usr/bin/env bash
# 描画コア（replor-render）の依存に、docs/design/p1-core.md の「描画コアの禁止」で
# 外すと決めた crate が無いことを確かめる。
set -euo pipefail

tree=$(cargo tree -p replor-render -e normal --all-features --target all --prefix none)
names=$(awk '{ print $1 }' <<<"$tree" | sort -u)

# gix-hash や ratatui-core のような分割された crate も拾うため、前方一致にする。
prefix='^(gix|ratatui|crossterm|notify)'
exact='^(ignore|replor-snapshot)$'

found=$(grep -E "$prefix|$exact" <<<"$names" || true)
if [[ -n $found ]]; then
  echo "replor-render depends on forbidden crates:" >&2
  echo "$found" >&2
  exit 1
fi
echo "replor-render has no forbidden dependencies"
