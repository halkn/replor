package git

import (
	"context"
	"fmt"
	"strings"
)

// ChangeKind distinguishes the record types of `git status --porcelain=v2`.
type ChangeKind int

const (
	ChangeOrdinary ChangeKind = iota
	ChangeRenamed
	ChangeCopied
	ChangeUnmerged
	ChangeUntracked
)

// FileChange is one entry of the working changes. Path and OrigPath are
// workspace-relative and empty for a side that lies outside the workspace
// (a rename across the boundary); RepoPath and RepoOrigPath are the
// toplevel-relative paths to pass back to git.
type FileChange struct {
	Path, OrigPath         string
	RepoPath, RepoOrigPath string
	// Index and Worktree are the X and Y status letters ('.' = unchanged).
	Index, Worktree byte
	Kind            ChangeKind
	Submodule       bool
}

// Snapshot is the repository state read in one refresh (DESIGN 7.2).
type Snapshot struct {
	// Revision increases with every snapshot of the same Repo; results
	// derived from a snapshot carry it so that mixed states are detected.
	Revision uint64
	HeadOID  string // "" before the first commit
	Branch   string // "" when detached
	Detached bool
	Changes  []FileChange
	// Ignored lists untracked ignored entries inside the workspace;
	// directories end with "/" and cover everything below them.
	Ignored []string
	// IgnoredErr is set when the ignored entries could not be read; the
	// changes are still valid and the caller keeps its previous ignored set.
	IgnoredErr error
	// Stale is set when HEAD moved between the two status reads even after
	// a retry, so the two halves may not belong together (DESIGN 7.2).
	Stale bool
}

// Snapshot reads the working changes and the ignored entries. The two
// reads are separate commands; if HEAD differs between them the snapshot is
// read once more and marked Stale when it still differs.
func (r *Repo) Snapshot(ctx context.Context) (*Snapshot, error) {
	var s *Snapshot
	for range 2 {
		var head string
		var err error
		s, head, err = r.snapshotOnce(ctx)
		if err != nil {
			return nil, err
		}
		s.Stale = s.IgnoredErr == nil && head != s.HeadOID
		if !s.Stale {
			break
		}
	}
	s.Revision = r.revision.Add(1)
	return s, nil
}

func (r *Repo) snapshotOnce(ctx context.Context) (s *Snapshot, ignoredHead string, err error) {
	// --no-ahead-behind: the upstream distance is not shown and can take a
	// long commit walk on a diverged branch.
	out, err := r.Run(ctx, "status", "--porcelain=v2", "-z", "--branch", "--no-ahead-behind", "--untracked-files=all", "--ignore-submodules=dirty")
	if err != nil {
		return nil, "", err
	}
	if s, err = r.parseStatus(out); err != nil {
		return nil, "", err
	}
	// Untracked directories stay collapsed here so that a large ignored
	// tree (node_modules, build output) is one entry.
	ign, err := r.Run(ctx, "status", "--porcelain=v2", "-z", "--branch", "--no-ahead-behind", "--untracked-files=normal", "--ignored=traditional", "--ignore-submodules=all")
	if err != nil {
		if ctx.Err() != nil {
			return nil, "", err
		}
		s.IgnoredErr = err
		return s, s.HeadOID, nil
	}
	s.Ignored, ignoredHead = r.parseIgnored(ign)
	return s, ignoredHead, nil
}

func (r *Repo) parseStatus(out []byte) (*Snapshot, error) {
	s := &Snapshot{}
	recs := strings.Split(strings.TrimSuffix(string(out), "\x00"), "\x00")
	for i := 0; i < len(recs); i++ {
		rec := recs[i]
		if rec == "" {
			continue
		}
		switch rec[0] {
		case '#':
			switch {
			case strings.HasPrefix(rec, "# branch.oid "):
				if oid := strings.TrimPrefix(rec, "# branch.oid "); oid != "(initial)" {
					s.HeadOID = oid
				}
			case strings.HasPrefix(rec, "# branch.head "):
				if head := strings.TrimPrefix(rec, "# branch.head "); head == "(detached)" {
					s.Detached = true
				} else {
					s.Branch = head
				}
			}
		case '1':
			f := strings.SplitN(rec, " ", 9)
			if len(f) != 9 {
				return nil, fmt.Errorf("git status: malformed record %q", rec)
			}
			r.add(s, FileChange{RepoPath: f[8], Index: f[1][0], Worktree: f[1][1], Submodule: f[2][0] == 'S'})
		case '2':
			f := strings.SplitN(rec, " ", 10)
			if len(f) != 10 || i+1 >= len(recs) {
				return nil, fmt.Errorf("git status: malformed rename record %q", rec)
			}
			i++
			kind := ChangeRenamed
			if f[8][0] == 'C' {
				kind = ChangeCopied
			}
			r.add(s, FileChange{RepoPath: f[9], RepoOrigPath: recs[i], Index: f[1][0], Worktree: f[1][1], Kind: kind, Submodule: f[2][0] == 'S'})
		case 'u':
			f := strings.SplitN(rec, " ", 11)
			if len(f) != 11 {
				return nil, fmt.Errorf("git status: malformed unmerged record %q", rec)
			}
			r.add(s, FileChange{RepoPath: f[10], Index: f[1][0], Worktree: f[1][1], Kind: ChangeUnmerged, Submodule: f[2][0] == 'S'})
		case '?':
			// An untracked nested repository is reported as "dir/".
			p := strings.TrimSuffix(strings.TrimPrefix(rec, "? "), "/")
			r.add(s, FileChange{RepoPath: p, Index: '?', Worktree: '?', Kind: ChangeUntracked})
		}
	}
	return s, nil
}

// add maps the paths into the workspace and keeps the change if either side
// is inside it.
func (r *Repo) add(s *Snapshot, c FileChange) {
	var in, origIn bool
	c.Path, in = r.FromGit(c.RepoPath)
	if !in {
		c.Path = ""
	}
	if c.RepoOrigPath != "" {
		c.OrigPath, origIn = r.FromGit(c.RepoOrigPath)
		if !origIn {
			c.OrigPath = ""
		}
	}
	if in || origIn {
		s.Changes = append(s.Changes, c)
	}
}

// parseIgnored returns the ignored entries and the HEAD the read saw. A
// rename record is followed by its original path as a separate field, which
// must not be taken for a record of its own.
func (r *Repo) parseIgnored(out []byte) (ign []string, head string) {
	recs := strings.Split(strings.TrimSuffix(string(out), "\x00"), "\x00")
	for i := 0; i < len(recs); i++ {
		rec := recs[i]
		switch {
		case strings.HasPrefix(rec, "2 "):
			i++
		case strings.HasPrefix(rec, "# branch.oid "):
			if oid := strings.TrimPrefix(rec, "# branch.oid "); oid != "(initial)" {
				head = oid
			}
		case strings.HasPrefix(rec, "! "):
			if rel, in := r.FromGit(strings.TrimPrefix(rec, "! ")); in {
				ign = append(ign, rel)
			}
		}
	}
	return ign, head
}
