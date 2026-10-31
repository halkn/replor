| 操作 | 方式 | 起動した外部プログラム | index の書き換え |
|---|---|---|---|
| `git status --porcelain=v2` | 素の git | clean・fsmonitor | あり |
| `git status --porcelain=v2` | 契約付き | なし | なし |
| `git diff` | 素の git | clean・extdiff・fsmonitor | あり |
| `git diff` | 契約付き | なし | なし |
| `git diff HEAD~1 HEAD` | 素の git | extdiff・smudge | なし |
| `git diff HEAD~1 HEAD` | 契約付き | なし | なし |
| `git log -p -1` | 素の git | smudge・textconv | なし |
| `git log -p -1` | 契約付き | なし | なし |
| `git show HEAD:a.txt` | 素の git | なし | なし |
| `git show HEAD:a.txt` | 契約付き | なし | なし |
| `git ls-tree -r HEAD` | 素の git | なし | なし |
| `git ls-tree -r HEAD` | 契約付き | なし | なし |
| gix: open・rev_parse・tree traverse・find_object（32 bytes）・index 読み取り（3 件） | gix | なし | なし |
