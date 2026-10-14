package git

import (
	"context"
	"errors"
	"fmt"
	"strings"
)

// Scope is one of the comparisons of DESIGN 7.1.
type Scope int

const (
	ScopeUnstaged  Scope = iota // index → work tree
	ScopeStaged                 // HEAD (or the empty tree) → index
	ScopeUntracked              // nothing → work tree file; no git diff
)

func (s Scope) String() string {
	return [...]string{"unstaged", "staged", "untracked"}[s]
}

// ErrNoGitDiff is returned for scopes that git does not compare (untracked
// files are shown from the work tree by the caller).
var ErrNoGitDiff = errors.New("no git diff for this scope")

// diffArgs fix the output format regardless of user config: prefixes
// (diff.noprefix, diff.mnemonicPrefix), paths relative to the top level
// (diff.relative) and submodule summaries.
var diffArgs = []string{
	"diff", "--no-ext-diff", "--no-textconv", "--no-color", "--no-relative",
	"--src-prefix=a/", "--dst-prefix=b/", "-M",
	"--ignore-submodules=dirty", "--submodule=short",
}

// Diff returns the unified diff of one change. Both paths of a rename are
// passed so that git pairs them.
func (r *Repo) Diff(ctx context.Context, scope Scope, c FileChange) ([]byte, error) {
	args := append([]string{}, diffArgs...)
	switch scope {
	case ScopeStaged:
		// Before the first commit --cached compares with the empty tree.
		args = append(args, "--cached")
	case ScopeUnstaged:
	default:
		return nil, fmt.Errorf("%w: %v", ErrNoGitDiff, scope)
	}
	args = append(args, "--", r.cwdRel(c.RepoPath))
	if c.RepoOrigPath != "" {
		args = append(args, r.cwdRel(c.RepoOrigPath))
	}
	return r.Run(ctx, args...)
}

// cwdRel turns a toplevel-relative path into one relative to the workspace
// directory git runs in; pathspecs are relative to it and magic such as
// :(top) is disabled by --literal-pathspecs.
func (r *Repo) cwdRel(p string) string {
	return strings.Repeat("../", strings.Count(r.Prefix, "/")) + p
}
