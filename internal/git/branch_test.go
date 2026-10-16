package git

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// branchRepo has a base branch "trunk" published as origin/trunk (with
// origin/HEAD pointing at it) and a feature branch checked out.
func branchRepo(t *testing.T) string {
	t.Helper()
	repo := newRepo(t)
	write(t, filepath.Join(repo, "keep.md"), "keep\n")
	write(t, filepath.Join(repo, "undo.md"), "undo\n")
	write(t, filepath.Join(repo, "gone.md"), "gone base\n")
	write(t, filepath.Join(repo, "back.md"), "back base\n")
	write(t, filepath.Join(repo, "touched.md"), "touched\n")
	write(t, filepath.Join(repo, "docs", "old.md"), strings.Repeat("same line\n", 10))
	sh(t, repo, "add", ".")
	sh(t, repo, "commit", "-qm", "base")
	sh(t, repo, "update-ref", "refs/remotes/origin/trunk", "HEAD")
	sh(t, repo, "symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk")
	sh(t, repo, "switch", "-qc", "feature")
	return repo
}

func comparisonPaths(c *Comparison) map[string]FileChange {
	m := map[string]FileChange{}
	for _, ch := range c.Changes {
		m[ch.Path] = ch
	}
	return m
}

func TestCompareBranch(t *testing.T) {
	hermetic(t)
	g := detect(t)
	ctx := context.Background()
	repo := branchRepo(t)
	// Committed on the branch.
	write(t, filepath.Join(repo, "undo.md"), "changed\n")
	sh(t, repo, "mv", "docs/old.md", "docs/new.md")
	sh(t, repo, "rm", "-q", "gone.md", "back.md")
	sh(t, repo, "commit", "-qam", "feature work")
	// Then in the work tree: a committed change undone, a staged and an
	// unstaged change, a new file, and a file deleted on the branch
	// recreated untracked.
	write(t, filepath.Join(repo, "undo.md"), "undo\n")
	write(t, filepath.Join(repo, "staged.md"), "staged\n")
	sh(t, repo, "add", "staged.md")
	write(t, filepath.Join(repo, "keep.md"), "keep edited\n")
	write(t, filepath.Join(repo, "new.md"), "untracked\n")
	write(t, filepath.Join(repo, "back.md"), "back again\n")
	// Rewritten unchanged with a new mtime: only its stat differs.
	later := time.Now().Add(time.Minute)
	write(t, filepath.Join(repo, "touched.md"), "touched\n")
	if err := os.Chtimes(filepath.Join(repo, "touched.md"), later, later); err != nil {
		t.Fatal(err)
	}

	r, err := g.Open(ctx, repo)
	if err != nil {
		t.Fatal(err)
	}
	s, err := r.Snapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	c, err := r.Compare(ctx, "", s)
	if err != nil {
		t.Fatal(err)
	}
	if c.Unavailable != "" {
		t.Fatalf("unavailable: %s", c.Unavailable)
	}
	trunk := strings.TrimSpace(sh(t, repo, "rev-parse", "trunk"))
	if c.Base != "origin/trunk" || c.BaseRef != "refs/remotes/origin/trunk" || c.MergeBase != trunk || c.BaseOID != trunk {
		t.Errorf("base: %+v", c)
	}
	got := comparisonPaths(c)
	want := map[string]FileChange{
		"keep.md":     {Path: "keep.md", RepoPath: "keep.md", Index: '.', Worktree: 'M'},
		"staged.md":   {Path: "staged.md", RepoPath: "staged.md", Index: '.', Worktree: 'A'},
		"gone.md":     {Path: "gone.md", RepoPath: "gone.md", Index: '.', Worktree: 'D'},
		"docs/new.md": {Path: "docs/new.md", OrigPath: "docs/old.md", RepoPath: "docs/new.md", RepoOrigPath: "docs/old.md", Index: '.', Worktree: 'R', Kind: ChangeRenamed},
		"new.md":      {Path: "new.md", RepoPath: "new.md", Index: '?', Worktree: '?', Kind: ChangeUntracked},
		// In the base and untracked now: one entry compared with the base.
		"back.md": {Path: "back.md", RepoPath: "back.md", Index: '?', Worktree: 'M', Kind: ChangeUntrackedInBase},
	}
	if len(got) != len(c.Changes) || len(got) != len(want) {
		t.Errorf("changes: %+v", c.Changes)
	}
	for p, w := range want {
		if got[p] != w {
			t.Errorf("%s:\n got %+v\nwant %+v", p, got[p], w)
		}
	}
	for i := 1; i < len(c.Changes); i++ {
		if c.Changes[i-1].RepoPath > c.Changes[i].RepoPath {
			t.Errorf("not sorted by path: %+v", c.Changes)
		}
	}

	diff := func(p string) string {
		t.Helper()
		out, err := r.BranchDiff(ctx, c, got[p], 0)
		if err != nil {
			t.Fatalf("%s: %v", p, err)
		}
		return string(out)
	}
	if d := diff("keep.md"); !strings.Contains(d, "-keep") || !strings.Contains(d, "+keep edited") {
		t.Errorf("modified:\n%s", d)
	}
	if d := diff("docs/new.md"); !strings.Contains(d, "rename from docs/old.md") {
		t.Errorf("rename:\n%s", d)
	}
	if d := diff("gone.md"); !strings.Contains(d, "-gone base") {
		t.Errorf("deleted:\n%s", d)
	}
	if _, err := r.BranchDiff(ctx, c, got["new.md"], 0); !errors.Is(err, ErrNoGitDiff) {
		t.Errorf("untracked: %v", err)
	}
	base, err := r.BaseContent(ctx, c, got["back.md"], 0)
	if err != nil || string(base) != "back base\n" {
		t.Errorf("base content: %q, %v", base, err)
	}
	if _, err := r.BaseContent(ctx, c, got["back.md"], 3); !errors.Is(err, ErrDiffTooLarge) {
		t.Errorf("base content limit: %v", err)
	}
}

func TestCompareExplicitBaseAndDetachedHead(t *testing.T) {
	hermetic(t)
	g := detect(t)
	ctx := context.Background()
	repo := branchRepo(t)
	sh(t, repo, "symbolic-ref", "--delete", "refs/remotes/origin/HEAD")
	write(t, filepath.Join(repo, "keep.md"), "branch\n")
	sh(t, repo, "commit", "-qam", "branch")
	sh(t, repo, "switch", "-q", "--detach", "HEAD")
	r, err := g.Open(ctx, repo)
	if err != nil {
		t.Fatal(err)
	}
	s, err := r.Snapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if !s.Detached {
		t.Fatal("HEAD is not detached")
	}
	c, err := r.Compare(ctx, "trunk", s)
	if err != nil || c.Unavailable != "" {
		t.Fatalf("%+v, %v", c, err)
	}
	if c.Base != "trunk" || c.BaseRef != "refs/heads/trunk" || len(c.Changes) != 1 || c.Changes[0].Path != "keep.md" {
		t.Errorf("%+v", c)
	}
	// An object name is not a ref: nothing to watch.
	c, err = r.Compare(ctx, c.BaseOID, s)
	if err != nil || c.Unavailable != "" || c.BaseRef != "" || len(c.Changes) != 1 {
		t.Errorf("by OID: %+v, %v", c, err)
	}
}

// Every condition without a well-defined merge base is reported with a
// reason instead of falling back to another base (DESIGN 7.1).
func TestCompareUnavailable(t *testing.T) {
	hermetic(t)
	g := detect(t)
	ctx := context.Background()
	compare := func(repo, base string) *Comparison {
		t.Helper()
		r, err := g.Open(ctx, repo)
		if err != nil {
			t.Fatal(err)
		}
		s, err := r.Snapshot(ctx)
		if err != nil {
			t.Fatal(err)
		}
		c, err := r.Compare(ctx, base, s)
		if err != nil {
			t.Fatalf("Compare: %v", err)
		}
		if len(c.Changes) != 0 {
			t.Errorf("changes despite %q: %+v", c.Unavailable, c.Changes)
		}
		return c
	}
	expect := func(name string, c *Comparison, want string) {
		t.Helper()
		if !strings.Contains(c.Unavailable, want) {
			t.Errorf("%s: reason %q, want it to mention %q", name, c.Unavailable, want)
		}
	}

	empty := newRepo(t)
	expect("no commits", compare(empty, "trunk"), "no commits")

	repo := branchRepo(t)
	expect("missing base", compare(repo, "nope"), `"nope" is not a commit`)
	expect("option-like base", compare(repo, "--all"), "invalid base")
	sh(t, repo, "symbolic-ref", "--delete", "refs/remotes/origin/HEAD")
	expect("no default base", compare(repo, ""), "no base")
	sh(t, repo, "symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/gone")
	expect("dangling origin/HEAD", compare(repo, ""), "refs/remotes/origin/gone")

	sh(t, repo, "switch", "-q", "--orphan", "unrelated")
	write(t, filepath.Join(repo, "o.md"), "o\n")
	sh(t, repo, "add", "o.md")
	sh(t, repo, "commit", "-qm", "orphan")
	expect("unrelated histories", compare(repo, "trunk"), "no common ancestor")

	// Criss-cross merges leave two best common ancestors.
	cc := newRepo(t)
	write(t, filepath.Join(cc, "f.md"), "0\n")
	sh(t, cc, "add", ".")
	sh(t, cc, "commit", "-qm", "0")
	sh(t, cc, "branch", "y")
	write(t, filepath.Join(cc, "x.md"), "x\n")
	sh(t, cc, "add", ".")
	sh(t, cc, "commit", "-qm", "x")
	sh(t, cc, "switch", "-q", "y")
	write(t, filepath.Join(cc, "y.md"), "y\n")
	sh(t, cc, "add", ".")
	sh(t, cc, "commit", "-qm", "y")
	sh(t, cc, "branch", "y0")
	sh(t, cc, "merge", "-q", "--no-edit", "trunk")
	sh(t, cc, "switch", "-q", "trunk")
	sh(t, cc, "merge", "-q", "--no-edit", "y0")
	expect("criss-cross", compare(cc, "y"), "multiple merge bases")

	// A shallow clone of two diverged branches cut above their fork.
	src := newRepo(t)
	write(t, filepath.Join(src, "f.md"), "1\n")
	sh(t, src, "add", ".")
	sh(t, src, "commit", "-qm", "1")
	sh(t, src, "switch", "-qc", "feature")
	write(t, filepath.Join(src, "g.md"), "2\n")
	sh(t, src, "add", ".")
	sh(t, src, "commit", "-qm", "2")
	sh(t, src, "switch", "-q", "trunk")
	write(t, filepath.Join(src, "f.md"), "3\n")
	sh(t, src, "commit", "-qam", "3")
	clone := filepath.Join(t.TempDir(), "clone")
	sh(t, src, "clone", "-q", "--depth", "1", "--no-single-branch", "file://"+src, clone)
	sh(t, clone, "switch", "-q", "feature")
	expect("shallow", compare(clone, ""), "shallow")

	// The merge base's tree is gone (e.g. a broken alternate).
	mb := branchRepo(t)
	write(t, filepath.Join(mb, "keep.md"), "branch\n")
	sh(t, mb, "commit", "-qam", "branch")
	tree := strings.TrimSpace(sh(t, mb, "rev-parse", "trunk^{tree}"))
	if err := os.Remove(filepath.Join(mb, ".git", "objects", tree[:2], tree[2:])); err != nil {
		t.Fatal(err)
	}
	expect("missing objects", compare(mb, ""), "missing")
}

// The workspace is a subdirectory: paths are workspace-relative and a
// rename across the boundary keeps only the inside side (DESIGN 6.1).
func TestCompareInSubdirectory(t *testing.T) {
	hermetic(t)
	g := detect(t)
	ctx := context.Background()
	repo := branchRepo(t)
	write(t, filepath.Join(repo, "top.md"), strings.Repeat("moved in\n", 10))
	sh(t, repo, "add", ".")
	sh(t, repo, "commit", "-qm", "top")
	sh(t, repo, "update-ref", "refs/remotes/origin/trunk", "HEAD")
	sh(t, repo, "mv", "top.md", "docs/in.md")
	sh(t, repo, "commit", "-qm", "move in")
	write(t, filepath.Join(repo, "keep.md"), "outside change\n")

	r, err := g.Open(ctx, filepath.Join(repo, "docs"))
	if err != nil {
		t.Fatal(err)
	}
	s, err := r.Snapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	c, err := r.Compare(ctx, "", s)
	if err != nil || c.Unavailable != "" {
		t.Fatalf("%+v, %v", c, err)
	}
	got := comparisonPaths(c)
	in := got["in.md"]
	if len(c.Changes) != 1 || in.Kind != ChangeRenamed || in.OrigPath != "" || in.RepoOrigPath != "top.md" {
		t.Fatalf("changes: %+v", c.Changes)
	}
	out, err := r.BranchDiff(ctx, c, in, 0)
	if err != nil || !strings.Contains(string(out), "new file mode") || strings.Contains(string(out), "top.md") {
		t.Errorf("renamed in across the boundary: %v\n%s", err, out)
	}
}

func TestParseBranchDiff(t *testing.T) {
	out := nul(
		":100644 100644 aaa 000 M", "docs/a.md",
		":000000 100644 000 000 A", "docs/new.md",
		":100644 000000 aaa 000 D", "docs/gone.md",
		":100644 100644 aaa bbb R087", "docs/old.md", "docs/moved.md",
		":100644 100644 aaa bbb C100", "docs/a.md", "docs/copy.md",
		":160000 160000 aaa 000 M", "docs/sub",
		":000000 000000 000 000 U", "docs/conflict.md",
		":100644 100644 aaa 000 M", "outside.md",
	)
	r := &Repo{Prefix: "docs/"}
	got, err := r.parseBranchDiff(out)
	if err != nil {
		t.Fatal(err)
	}
	want := []FileChange{
		{Path: "a.md", RepoPath: "docs/a.md", Index: '.', Worktree: 'M'},
		{Path: "new.md", RepoPath: "docs/new.md", Index: '.', Worktree: 'A'},
		{Path: "gone.md", RepoPath: "docs/gone.md", Index: '.', Worktree: 'D'},
		{Path: "moved.md", OrigPath: "old.md", RepoPath: "docs/moved.md", RepoOrigPath: "docs/old.md", Index: '.', Worktree: 'R', Kind: ChangeRenamed},
		{Path: "copy.md", OrigPath: "a.md", RepoPath: "docs/copy.md", RepoOrigPath: "docs/a.md", Index: '.', Worktree: 'C', Kind: ChangeCopied},
		{Path: "sub", RepoPath: "docs/sub", Index: '.', Worktree: 'M', Submodule: true},
		{Path: "conflict.md", RepoPath: "docs/conflict.md", Index: 'U', Worktree: 'U', Kind: ChangeUnmerged},
	}
	if len(got) != len(want) {
		t.Fatalf("got %+v", got)
	}
	for i := range want {
		if got[i].FileChange != want[i] {
			t.Errorf("%d:\n got %+v\nwant %+v", i, got[i].FileChange, want[i])
		}
	}
	for _, bad := range [][]byte{nul(":100644 M"), nul(":100644 100644 a b R100", "only-one")} {
		if _, err := r.parseBranchDiff(bad); err == nil {
			t.Errorf("accepted %q", bad)
		}
	}
}
