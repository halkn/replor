package git

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

func nul(records ...string) []byte { return []byte(strings.Join(records, "\x00") + "\x00") }

func TestParseStatus(t *testing.T) {
	out := nul(
		"# branch.oid 1111111111111111111111111111111111111111",
		"# branch.head feature/x",
		"1 .M N... 100644 100644 100644 aaa aaa docs/a.md",
		"1 M. N... 100644 100644 100644 aaa bbb docs/with space.md",
		"1 MM N... 100644 100644 100644 aaa bbb docs/both.md",
		"1 D. N... 100644 000000 000000 aaa 000 docs/gone.md",
		"2 R. N... 100644 100644 100644 aaa aaa R100 docs/new.md", "old/outside.md",
		"2 R. N... 100644 100644 100644 aaa aaa R090 elsewhere/moved.md", "docs/moved-out.md",
		"u UU N... 100644 100644 100644 100644 a b c docs/conflict.md",
		"1 .M S.M. 160000 160000 160000 aaa aaa docs/sub",
		"? docs/new\nline.md",
		"? -dash.md",
		"1 .M N... 100644 100644 100644 aaa aaa outside.md",
	)
	r := &Repo{Prefix: "docs/"}
	s, err := r.parseStatus(out)
	if err != nil {
		t.Fatal(err)
	}
	if s.HeadOID != "1111111111111111111111111111111111111111" || s.Branch != "feature/x" || s.Detached {
		t.Errorf("header: %+v", s)
	}
	byPath := map[string]FileChange{}
	for _, c := range s.Changes {
		key := c.Path
		if key == "" {
			key = "orig:" + c.OrigPath
		}
		byPath[key] = c
	}
	want := map[string]FileChange{
		"a.md":              {Path: "a.md", RepoPath: "docs/a.md", Index: '.', Worktree: 'M'},
		"with space.md":     {Path: "with space.md", RepoPath: "docs/with space.md", Index: 'M', Worktree: '.'},
		"both.md":           {Path: "both.md", RepoPath: "docs/both.md", Index: 'M', Worktree: 'M'},
		"gone.md":           {Path: "gone.md", RepoPath: "docs/gone.md", Index: 'D', Worktree: '.'},
		"new.md":            {Path: "new.md", RepoPath: "docs/new.md", RepoOrigPath: "old/outside.md", Index: 'R', Worktree: '.', Kind: ChangeRenamed},
		"orig:moved-out.md": {OrigPath: "moved-out.md", RepoPath: "elsewhere/moved.md", RepoOrigPath: "docs/moved-out.md", Index: 'R', Worktree: '.', Kind: ChangeRenamed},
		"conflict.md":       {Path: "conflict.md", RepoPath: "docs/conflict.md", Index: 'U', Worktree: 'U', Kind: ChangeUnmerged},
		"sub":               {Path: "sub", RepoPath: "docs/sub", Index: '.', Worktree: 'M', Submodule: true},
		"new\nline.md":      {Path: "new\nline.md", RepoPath: "docs/new\nline.md", Index: '?', Worktree: '?', Kind: ChangeUntracked},
	}
	if len(byPath) != len(want) {
		t.Errorf("got %d changes, want %d: %+v", len(byPath), len(want), s.Changes)
	}
	for k, w := range want {
		if got := byPath[k]; got != w {
			t.Errorf("%q\n got %+v\nwant %+v", k, got, w)
		}
	}

	for _, hdr := range [][]string{
		{"# branch.oid (initial)", "# branch.head main"},
		{"# branch.oid 2222222222222222222222222222222222222222", "# branch.head (detached)"},
	} {
		s, err := (&Repo{}).parseStatus(nul(hdr...))
		if err != nil {
			t.Fatal(err)
		}
		switch {
		case strings.Contains(hdr[0], "initial") && (s.HeadOID != "" || s.Branch != "main" || s.Detached):
			t.Errorf("initial: %+v", s)
		case strings.Contains(hdr[1], "detached") && (!s.Detached || s.Branch != ""):
			t.Errorf("detached: %+v", s)
		}
	}

	if _, err := (&Repo{}).parseStatus([]byte("2 R. N... 1 1 1 a a R100 new.md\x00")); err == nil {
		t.Error("rename without its original path must fail")
	}
}

func TestParseIgnored(t *testing.T) {
	r := &Repo{Prefix: "app/"}
	got, _ := r.parseIgnored(nul("? app/untracked.md", "! app/node_modules/", "! app/debug.log", "! other/x.log"))
	if want := []string{"node_modules/", "debug.log"}; !slices.Equal(got, want) {
		t.Errorf("ignored = %v, want %v", got, want)
	}
}

func TestSnapshotOnRealRepository(t *testing.T) {
	hermetic(t)
	g := detect(t)
	ctx := context.Background()

	repo := newRepo(t)
	r, err := g.Open(ctx, repo)
	if err != nil {
		t.Fatal(err)
	}
	s, err := r.Snapshot(ctx)
	if err != nil || s.HeadOID != "" || s.Branch != "trunk" {
		t.Fatalf("initial snapshot: %+v %v", s, err)
	}

	write(t, filepath.Join(repo, "keep.md"), "keep\n")
	write(t, filepath.Join(repo, "both.md"), "v1\n")
	write(t, filepath.Join(repo, "del.md"), "d\n")
	write(t, filepath.Join(repo, "ren.md"), strings.Repeat("rename me\n", 10))
	write(t, filepath.Join(repo, ".gitignore"), "build/\n*.log\ntracked.log\n")
	write(t, filepath.Join(repo, "tracked.log"), "t\n")
	sh(t, repo, "add", ".")
	sh(t, repo, "add", "-f", "tracked.log")
	sh(t, repo, "commit", "-qm", "base")

	write(t, filepath.Join(repo, "both.md"), "v2\n")
	sh(t, repo, "add", "both.md")
	write(t, filepath.Join(repo, "both.md"), "v3\n")
	if err := os.Remove(filepath.Join(repo, "del.md")); err != nil {
		t.Fatal(err)
	}
	sh(t, repo, "mv", "ren.md", "renamed.md")
	write(t, filepath.Join(repo, "untracked.md"), "u\n")
	write(t, filepath.Join(repo, "build", "out.bin"), "b\n")
	write(t, filepath.Join(repo, "debug.log"), "l\n")

	s, err = r.Snapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if s.HeadOID == "" || s.Branch != "trunk" || s.Revision < 2 {
		t.Errorf("header: %+v", s)
	}
	got := map[string]string{}
	for _, c := range s.Changes {
		got[c.Path] = string([]byte{c.Index, c.Worktree})
	}
	want := map[string]string{"both.md": "MM", "del.md": ".D", "renamed.md": "R.", "untracked.md": "??"}
	for p, xy := range want {
		if got[p] != xy {
			t.Errorf("%s = %q, want %q (all %v)", p, got[p], xy, got)
		}
	}
	if _, ok := got["tracked.log"]; ok {
		t.Error("tracked file matching an ignore pattern reported as changed")
	}
	if !slices.Contains(s.Ignored, "build/") || !slices.Contains(s.Ignored, "debug.log") || slices.Contains(s.Ignored, "tracked.log") {
		t.Errorf("ignored = %v", s.Ignored)
	}

	// Detached HEAD.
	sh(t, repo, "stash", "-q", "-u")
	sh(t, repo, "checkout", "-q", "--detach")
	if s, err = r.Snapshot(ctx); err != nil || !s.Detached || s.Branch != "" {
		t.Errorf("detached: %+v %v", s, err)
	}
}

func TestSnapshotConflict(t *testing.T) {
	hermetic(t)
	g := detect(t)
	repo := newRepo(t)
	write(t, filepath.Join(repo, "c.md"), "base\n")
	sh(t, repo, "add", ".")
	sh(t, repo, "commit", "-qm", "base")
	sh(t, repo, "switch", "-qc", "other")
	write(t, filepath.Join(repo, "c.md"), "other\n")
	sh(t, repo, "commit", "-qam", "other")
	sh(t, repo, "switch", "-q", "trunk")
	write(t, filepath.Join(repo, "c.md"), "trunk\n")
	sh(t, repo, "commit", "-qam", "trunk")
	cmd := []string{"merge", "-q", "other"}
	_ = runGit(repo, cmd...) // fails with a conflict by design

	r, err := g.Open(context.Background(), repo)
	if err != nil {
		t.Fatal(err)
	}
	s, err := r.Snapshot(context.Background())
	if err != nil || len(s.Changes) != 1 || s.Changes[0].Kind != ChangeUnmerged || s.Changes[0].Path != "c.md" {
		t.Errorf("conflict: %+v %v", s, err)
	}
}

func TestSnapshotSubmodule(t *testing.T) {
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
	// A dirty file inside the submodule is not inspected (DESIGN 7.1).
	write(t, filepath.Join(repo, "mod", "dirty.md"), "x\n")

	r, err := g.Open(context.Background(), repo)
	if err != nil {
		t.Fatal(err)
	}
	s, err := r.Snapshot(context.Background())
	if err != nil || len(s.Changes) != 1 || !s.Changes[0].Submodule || s.Changes[0].Path != "mod" || s.Changes[0].Worktree != 'M' {
		t.Errorf("submodule: %+v %v", s, err)
	}
}

// runGit is sh for commands that are expected to fail.
func runGit(dir string, args ...string) error {
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	return cmd.Run()
}

func TestParseIgnoredSkipsRenameOrigins(t *testing.T) {
	r := &Repo{}
	out := nul(
		"# branch.oid 3333333333333333333333333333333333333333",
		"2 R. N... 100644 100644 100644 a a R100 final.md", "! draft.md",
		"! real.log",
	)
	got, head := r.parseIgnored(out)
	if !slices.Equal(got, []string{"real.log"}) || head != "3333333333333333333333333333333333333333" {
		t.Errorf("ignored = %v head = %q", got, head)
	}
}

func TestUntrackedNestedRepositoryHasNoTrailingSlash(t *testing.T) {
	s, err := (&Repo{}).parseStatus(nul("? vendor/x/"))
	if err != nil || len(s.Changes) != 1 || s.Changes[0].Path != "vendor/x" || s.Changes[0].RepoPath != "vendor/x" {
		t.Errorf("changes = %+v %v", s.Changes, err)
	}
}

// Commands for one repository run one at a time; a waiting command gives
// up when its context ends.
func TestRunIsSerializedPerRepository(t *testing.T) {
	hermetic(t)
	g := detect(t)
	r, err := g.Open(context.Background(), newRepo(t))
	if err != nil {
		t.Fatal(err)
	}
	r.sem <- struct{}{} // another command is running
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := r.Run(ctx, "status"); !errors.Is(err, context.Canceled) {
		t.Errorf("waiting Run = %v", err)
	}
	<-r.sem
	if _, err := r.Run(context.Background(), "status"); err != nil {
		t.Errorf("Run after release: %v", err)
	}
}

// A linked worktree keeps its index and HEAD in its own git directory and
// shares refs through the common directory.
func TestMetadataDirsOfWorktree(t *testing.T) {
	hermetic(t)
	g := detect(t)
	repo := newRepo(t)
	write(t, filepath.Join(repo, "a.md"), "a\n")
	sh(t, repo, "add", ".")
	sh(t, repo, "commit", "-qm", "base")
	wt := filepath.Join(t.TempDir(), "wt")
	sh(t, repo, "worktree", "add", "-q", "-b", "side", wt)

	r, err := g.Open(context.Background(), wt)
	if err != nil {
		t.Fatal(err)
	}
	gitDir, common, err := r.MetadataDirs(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	realRepo, _ := filepath.EvalSymlinks(repo)
	if want := filepath.Join(realRepo, ".git"); common != want {
		t.Errorf("common = %q, want %q", common, want)
	}
	if !strings.HasPrefix(gitDir, filepath.Join(realRepo, ".git", "worktrees")+string(filepath.Separator)) {
		t.Errorf("git dir = %q", gitDir)
	}
	if _, err := os.Stat(filepath.Join(gitDir, "index")); err != nil {
		t.Errorf("no index in the worktree git dir: %v", err)
	}
}
