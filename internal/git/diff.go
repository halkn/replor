package git

import (
	"context"
	"errors"
	"fmt"
	"strings"

	"github.com/halkn/replor/internal/process"
)

// Scope is one of the comparisons of DESIGN 7.1.
type Scope int

const (
	ScopeUnstaged  Scope = iota // index → work tree
	ScopeStaged                 // HEAD (or the empty tree) → index
	ScopeUntracked              // nothing → work tree file; no git diff
	ScopeBranch                 // merge base → work tree (Comparison)
)

func (s Scope) String() string {
	return [...]string{"unstaged", "staged", "untracked", "branch"}[s]
}

var (
	// ErrNoGitDiff is returned for scopes that git does not compare
	// (untracked files are shown from the work tree by the caller).
	ErrNoGitDiff = errors.New("no git diff for this scope")
	// ErrOutsideWorkspace refuses a diff whose new side lies outside the
	// workspace: its content must not be shown (DESIGN 6.1).
	ErrOutsideWorkspace = errors.New("the changed file is outside the workspace")
	// ErrDiffTooLarge is returned when the diff exceeds the given limit.
	ErrDiffTooLarge = errors.New("diff is too large to show")
)

// diffArgs fix the output format regardless of user config: prefixes
// (diff.noprefix, diff.mnemonicPrefix), paths relative to the top level
// (diff.relative) and submodule summaries.
var diffArgs = []string{
	"diff", "--no-ext-diff", "--no-textconv", "--no-color", "--no-relative",
	"--src-prefix=a/", "--dst-prefix=b/", "-M",
	"--ignore-submodules=dirty", "--submodule=short",
}

// Diff returns the unified diff of one working change, at most max bytes (0 = the
// default command limit). Both paths of a rename or copy are passed so that
// git pairs them, but only paths inside the workspace: a source outside is
// left out and the file then shows as added.
func (r *Repo) Diff(ctx context.Context, scope Scope, c FileChange, max int64) ([]byte, error) {
	switch scope {
	case ScopeStaged:
		// Before the first commit --cached compares with the empty tree.
		return r.diff(ctx, []string{"--cached"}, c, max)
	case ScopeUnstaged:
		return r.diff(ctx, nil, c, max)
	}
	return nil, fmt.Errorf("%w: %v", ErrNoGitDiff, scope)
}

// diff runs diffArgs with what selects the comparison (rev) for one change.
func (r *Repo) diff(ctx context.Context, rev []string, c FileChange, max int64) ([]byte, error) {
	if c.Path == "" {
		return nil, ErrOutsideWorkspace
	}
	args := append([]string{}, diffArgs...)
	if c.Kind == ChangeCopied && c.OrigPath != "" {
		// status may report copies (status.renames=copies); an unmodified
		// source is only considered with --find-copies-harder.
		args = append(args, "-C", "--find-copies-harder")
	}
	args = append(append(args, rev...), "--", r.cwdRel(c.RepoPath))
	if c.OrigPath != "" {
		args = append(args, r.cwdRel(c.RepoOrigPath))
	}
	if max <= 0 {
		max = defaultMaxOutput
	}
	out, err := r.runMax(ctx, max, args...)
	if errors.Is(err, process.ErrOutputLimit) {
		return nil, fmt.Errorf("%w (over %d KiB)", ErrDiffTooLarge, max>>10)
	}
	return out, err
}

// cwdRel turns a toplevel-relative path into one relative to the workspace
// directory git runs in; pathspecs are relative to it and magic such as
// :(top) is disabled by --literal-pathspecs.
func (r *Repo) cwdRel(p string) string {
	return strings.Repeat("../", strings.Count(r.Prefix, "/")) + p
}
