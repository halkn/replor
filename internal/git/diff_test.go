package git

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func change(t *testing.T, s *Snapshot, path string) FileChange {
	t.Helper()
	for _, c := range s.Changes {
		if c.Path == path || c.Path == "" && c.OrigPath == path {
			return c
		}
	}
	t.Fatalf("no change for %q in %+v", path, s.Changes)
	return FileChange{}
}

func TestDiffScopes(t *testing.T) {
	hermetic(t)
	g := detect(t)
	ctx := context.Background()
	repo := newRepo(t)
	write(t, filepath.Join(repo, "docs", "both.md"), "one\n")
	write(t, filepath.Join(repo, "docs", "gone.md"), "gone\n")
	write(t, filepath.Join(repo, "docs", "old.md"), strings.Repeat("same line\n", 10))
	write(t, filepath.Join(repo, "top.md"), strings.Repeat("moved out\n", 10))
	sh(t, repo, "add", ".")
	sh(t, repo, "commit", "-qm", "base")

	write(t, filepath.Join(repo, "docs", "both.md"), "two\n")
	sh(t, repo, "add", "docs/both.md")
	write(t, filepath.Join(repo, "docs", "both.md"), "three\n")
	if err := os.Remove(filepath.Join(repo, "docs", "gone.md")); err != nil {
		t.Fatal(err)
	}
	sh(t, repo, "mv", "docs/old.md", "docs/new.md")
	sh(t, repo, "mv", "top.md", "docs/in.md") // renamed into the workspace
	// diff.relative would restrict paths to the current directory and
	// strip their prefix; the contract overrides it.
	sh(t, repo, "config", "diff.relative", "true")
	sh(t, repo, "config", "diff.noprefix", "true")

	// The workspace is docs/; the rename source top.md lies outside.
	r, err := g.Open(ctx, filepath.Join(repo, "docs"))
	if err != nil {
		t.Fatal(err)
	}
	s, err := r.Snapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}

	diff := func(scope Scope, path string) string {
		t.Helper()
		out, err := r.Diff(ctx, scope, change(t, s, path))
		if err != nil {
			t.Fatalf("%v %s: %v", scope, path, err)
		}
		return string(out)
	}
	if d := diff(ScopeStaged, "both.md"); !strings.Contains(d, "-one") || !strings.Contains(d, "+two") || strings.Contains(d, "three") {
		t.Errorf("staged:\n%s", d)
	}
	if d := diff(ScopeUnstaged, "both.md"); !strings.Contains(d, "-two") || !strings.Contains(d, "+three") || !strings.Contains(d, "--- a/docs/both.md") {
		t.Errorf("unstaged:\n%s", d)
	}
	if d := diff(ScopeUnstaged, "gone.md"); !strings.Contains(d, "deleted file mode") || !strings.Contains(d, "-gone") {
		t.Errorf("deleted:\n%s", d)
	}
	if d := diff(ScopeStaged, "new.md"); !strings.Contains(d, "rename from docs/old.md") || !strings.Contains(d, "rename to docs/new.md") {
		t.Errorf("rename:\n%s", d)
	}
	if d := diff(ScopeStaged, "in.md"); !strings.Contains(d, "rename from top.md") {
		t.Errorf("rename across the boundary:\n%s", d)
	}
	if _, err := r.Diff(ctx, ScopeUntracked, FileChange{Path: "x"}); !errors.Is(err, ErrNoGitDiff) {
		t.Errorf("untracked scope: %v", err)
	}
}

func TestDiffBeforeFirstCommit(t *testing.T) {
	hermetic(t)
	g := detect(t)
	ctx := context.Background()
	repo := newRepo(t)
	write(t, filepath.Join(repo, "first.md"), "hello\n")
	sh(t, repo, "add", ".")
	r, err := g.Open(ctx, repo)
	if err != nil {
		t.Fatal(err)
	}
	s, err := r.Snapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	out, err := r.Diff(ctx, ScopeStaged, change(t, s, "first.md"))
	if err != nil || !strings.Contains(string(out), "new file mode") || !strings.Contains(string(out), "+hello") {
		t.Errorf("initial staged diff: %v\n%s", err, out)
	}
}

func TestDiffSubmodule(t *testing.T) {
	hermetic(t)
	g := detect(t)
	sub := newRepo(t)
	write(t, filepath.Join(sub, "s.md"), "1\n")
	sh(t, sub, "add", ".")
	sh(t, sub, "commit", "-qm", "1")
	repo := newRepo(t)
	sh(t, repo, "-c", "protocol.file.allow=always", "submodule", "-q", "add", sub, "mod")
	sh(t, repo, "commit", "-qm", "add submodule")
	sh(t, filepath.Join(repo, "mod"), "config", "user.email", "t@example.com")
	sh(t, filepath.Join(repo, "mod"), "config", "user.name", "t")
	write(t, filepath.Join(repo, "mod", "s.md"), "2\n")
	sh(t, filepath.Join(repo, "mod"), "commit", "-qam", "2")

	r, err := g.Open(context.Background(), repo)
	if err != nil {
		t.Fatal(err)
	}
	s, err := r.Snapshot(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	out, err := r.Diff(context.Background(), ScopeUnstaged, change(t, s, "mod"))
	if err != nil || !strings.Contains(string(out), "Subproject commit") {
		t.Errorf("submodule diff: %v\n%s", err, out)
	}
}
