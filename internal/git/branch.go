package git

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"slices"
	"strings"

	"github.com/halkn/replor/internal/process"
)

// DefaultBaseRef is where the base branch is looked up when none is given:
// clone records the remote's default branch there, and it changes only on
// fetch or `git remote set-head`, never by itself (no fetch is made).
const DefaultBaseRef = "refs/remotes/origin/HEAD"

// Comparison is the Branch Changes list (DESIGN 7.1): the work tree compared
// with the merge base of the base branch and HEAD, untracked files included.
type Comparison struct {
	// Base names the base as shown; BaseRef is its full ref name, or "" when
	// the base was given as an object name.
	Base, BaseRef      string
	BaseOID, MergeBase string
	// Changes are sorted by repository path. Index is '.' (or '?' for
	// untracked files) and Worktree holds the status letter.
	Changes []FileChange
	// Unavailable explains why there is no comparison; Changes is empty.
	Unavailable string
}

// Compare builds the Branch Changes of snapshot s against base, or against
// the branch DefaultBaseRef points to when base is "". Conditions without a
// single merge base are reported in Unavailable rather than compared with
// some other commit; err is for commands that failed otherwise.
func (r *Repo) Compare(ctx context.Context, base string, s *Snapshot) (*Comparison, error) {
	c := &Comparison{Base: base}
	unavailable := func(format string, a ...any) (*Comparison, error) {
		c.Unavailable = fmt.Sprintf(format, a...)
		return c, nil
	}
	if s.HeadOID == "" {
		return unavailable("no commits yet")
	}
	switch {
	case base == "":
		out, err := r.Run(ctx, "symbolic-ref", "--quiet", DefaultBaseRef)
		if exitCode(err) == 1 {
			return unavailable("no base branch: %s is not set (use --base)", DefaultBaseRef)
		} else if err != nil {
			return nil, err
		}
		c.BaseRef = strings.TrimSpace(string(out))
		c.Base = strings.TrimPrefix(c.BaseRef, "refs/remotes/")
	case strings.HasPrefix(base, "-"):
		// rev-parse would take it as an option.
		return unavailable("invalid base %q", base)
	default:
		// Prints nothing for an object name; fails for an unknown name,
		// which the lookup below reports.
		out, _ := r.Run(ctx, "rev-parse", "--verify", "--quiet", "--symbolic-full-name", base)
		c.BaseRef = strings.TrimSpace(string(out))
	}
	name := c.BaseRef
	if name == "" {
		name = base
	}
	out, err := r.Run(ctx, "rev-parse", "--verify", "--quiet", name+"^{commit}")
	if exitCode(err) == 1 {
		if base == "" {
			return unavailable("%s points to %s, which is not a commit here", DefaultBaseRef, c.BaseRef)
		}
		return unavailable("%q is not a commit in this repository", base)
	} else if err != nil {
		return nil, err
	}
	c.BaseOID = strings.TrimSpace(string(out))

	out, err = r.Run(ctx, "merge-base", "--all", c.BaseOID, s.HeadOID)
	switch {
	case missingObjects(err):
		return unavailable("objects are missing: %s", firstLine(err))
	case exitCode(err) == 1 && len(bytes.TrimSpace(out)) == 0:
		shallow, err := r.Run(ctx, "rev-parse", "--is-shallow-repository")
		if err != nil {
			return nil, err
		}
		if strings.TrimSpace(string(shallow)) == "true" {
			return unavailable("shallow history does not reach a common ancestor of %s and HEAD (no fetch is made)", c.Base)
		}
		return unavailable("%s and HEAD have no common ancestor", c.Base)
	case err != nil:
		return nil, err
	}
	bases := strings.Fields(string(out))
	if len(bases) != 1 {
		return unavailable("%s and HEAD have multiple merge bases", c.Base)
	}
	c.MergeBase = bases[0]

	// The whole repository is compared, like status, so that a rename across
	// the workspace boundary is paired and then cut by add.
	out, err = r.Run(ctx, "diff", "--no-ext-diff", "--no-textconv", "--no-color", "--no-relative", "-M",
		"--ignore-submodules=dirty", "--raw", "--no-abbrev", "-z", c.MergeBase, "--")
	if missingObjects(err) {
		c.MergeBase = ""
		return unavailable("objects are missing: %s", firstLine(err))
	} else if err != nil {
		return nil, err
	}
	entries, err := r.parseBranchDiff(out)
	if err != nil {
		return nil, err
	}
	changes, err := r.dropStatOnly(ctx, entries)
	if err != nil {
		return nil, err
	}
	c.Changes = mergeUntracked(changes, s.Changes)
	return c, nil
}

// mergeUntracked adds the untracked files of the snapshot. A path the merge
// base has but the index does not shows as deleted in the diff; when it
// exists untracked, the one entry compares it with the base (DESIGN 7.1).
func mergeUntracked(changes, status []FileChange) []FileChange {
	at := map[string]int{}
	for i, c := range changes {
		at[c.RepoPath] = i
	}
	for _, u := range status {
		if u.Kind != ChangeUntracked {
			continue
		}
		if i, ok := at[u.RepoPath]; ok && changes[i].Worktree == 'D' {
			changes[i].Index, changes[i].Worktree, changes[i].Kind = '?', 'M', ChangeUntrackedInBase
			continue
		}
		changes = append(changes, u)
	}
	slices.SortStableFunc(changes, func(a, b FileChange) int { return strings.Compare(a.RepoPath, b.RepoPath) })
	return changes
}

// rawEntry is a change with the modes and object names of its two sides.
type rawEntry struct {
	FileChange
	srcMode, dstMode, srcOID, dstOID string
}

// parseBranchDiff reads `git diff --raw -z`: ":<mode> <mode> <oid> <oid>
// <status>" then the path, and for renames and copies the source first.
func (r *Repo) parseBranchDiff(out []byte) ([]rawEntry, error) {
	var list []rawEntry
	recs := strings.Split(strings.TrimSuffix(string(out), "\x00"), "\x00")
	for i := 0; i < len(recs); i++ {
		if recs[i] == "" {
			continue
		}
		f := strings.Fields(strings.TrimPrefix(recs[i], ":"))
		if len(f) != 5 || i+1 >= len(recs) {
			return nil, fmt.Errorf("git diff: malformed record %q", recs[i])
		}
		c := FileChange{Index: '.', Worktree: f[4][0], Submodule: f[0] == "160000" || f[1] == "160000"}
		switch c.Worktree {
		case 'R', 'C':
			if i+2 >= len(recs) {
				return nil, fmt.Errorf("git diff: malformed rename record %q", recs[i])
			}
			c.Kind = ChangeRenamed
			if c.Worktree == 'C' {
				c.Kind = ChangeCopied
			}
			c.RepoOrigPath = recs[i+1]
			i++
		case 'U':
			c.Index, c.Kind = 'U', ChangeUnmerged
		}
		i++
		c.RepoPath = recs[i]
		if c, ok := r.inWorkspace(c); ok {
			list = append(list, rawEntry{c, f[0], f[1], f[2], f[3]})
		}
	}
	return list, nil
}

// hashBatch bounds the paths passed to one hash-object.
const hashBatch = 500

// dropStatOnly removes modifications that are only a changed stat. With
// diff.autoRefreshIndex off (see globalArgs), git reports a work tree file
// whose stat differs from the index as modified without reading it (object
// name all zeros); hashing the file tells whether its content still equals
// the merge base's. hash-object without -w writes nothing. Files that cannot
// be hashed stay listed.
func (r *Repo) dropStatOnly(ctx context.Context, entries []rawEntry) ([]FileChange, error) {
	var check []int
	for i, e := range entries {
		regular := e.srcMode == "100644" || e.srcMode == "100755"
		if e.Worktree == 'M' && e.srcMode == e.dstMode && regular && strings.Trim(e.dstOID, "0") == "" && e.Path != "" {
			check = append(check, i)
		}
	}
	same := map[int]bool{}
	for start := 0; start < len(check); start += hashBatch {
		batch := check[start:min(start+hashBatch, len(check))]
		args := []string{"hash-object", "--"}
		for _, i := range batch {
			args = append(args, r.cwdRel(entries[i].RepoPath))
		}
		out, err := r.Run(ctx, args...)
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		oids := strings.Fields(string(out))
		if err != nil || len(oids) != len(batch) {
			continue
		}
		for j, i := range batch {
			same[i] = oids[j] == entries[i].srcOID
		}
	}
	list := make([]FileChange, 0, len(entries))
	for i, e := range entries {
		if !same[i] {
			list = append(list, e.FileChange)
		}
	}
	return list, nil
}

// BranchDiff returns the unified diff of one Branch change, at most max
// bytes (0 = the default command limit). Untracked files have no git diff
// (ErrNoGitDiff); see BaseContent for ChangeUntrackedInBase.
func (r *Repo) BranchDiff(ctx context.Context, cmp *Comparison, c FileChange, max int64) ([]byte, error) {
	if c.Kind == ChangeUntracked || c.Kind == ChangeUntrackedInBase {
		return nil, fmt.Errorf("%w: untracked %s", ErrNoGitDiff, c.Path)
	}
	return r.diff(ctx, []string{cmp.MergeBase}, c, max)
}

// BaseContent returns the content of c's path in the merge base, at most
// max bytes (0 = the default command limit). cat-file without --filters
// runs no filter driver.
func (r *Repo) BaseContent(ctx context.Context, cmp *Comparison, c FileChange, max int64) ([]byte, error) {
	if max <= 0 {
		max = defaultMaxOutput
	}
	out, err := r.runMax(ctx, max, "cat-file", "blob", cmp.MergeBase+":"+c.RepoPath)
	if errors.Is(err, process.ErrOutputLimit) {
		return nil, fmt.Errorf("%w (over %d KiB)", ErrDiffTooLarge, max>>10)
	}
	return out, err
}

func exitCode(err error) int {
	var pe *process.Error
	if errors.As(err, &pe) {
		return pe.ExitCode
	}
	return 0
}

// missingObjects recognizes git's messages for objects that are not in the
// repository (LC_ALL=C keeps them stable).
func missingObjects(err error) bool {
	var pe *process.Error
	if !errors.As(err, &pe) {
		return false
	}
	for _, s := range []string{"bad object", "bad tree object", "unable to read", "not a valid object", "Not a valid commit name", "missing blob", "missing tree"} {
		if strings.Contains(pe.Stderr, s) {
			return true
		}
	}
	return false
}

func firstLine(err error) string {
	var pe *process.Error
	if errors.As(err, &pe) {
		line, _, _ := strings.Cut(strings.TrimSpace(pe.Stderr), "\n")
		return line
	}
	return err.Error()
}
