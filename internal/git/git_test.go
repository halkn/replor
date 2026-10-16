package git

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestParseVersion(t *testing.T) {
	cases := map[string]Version{
		"git version 2.50.1 (Apple Git-155)\n": {2, 50, 1},
		"git version 2.43.0.windows.1":         {2, 43, 0},
		"git version 2.45.2":                   {2, 45, 2},
		"git version 3.0":                      {3, 0, 0},
	}
	for in, want := range cases {
		got, err := ParseVersion(in)
		if err != nil || got != want {
			t.Errorf("ParseVersion(%q) = %v, %v; want %v", in, got, err, want)
		}
	}
	if _, err := ParseVersion("hub version 1"); err == nil {
		t.Error("expected error for non-git output")
	}
}

// hermetic isolates fixture setup and the code under test from the developer's
// own git config, hooks and environment.
func hermetic(t *testing.T) string {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not installed")
	}
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(home, ".config"))
	t.Setenv("GIT_CONFIG_NOSYSTEM", "1")
	t.Setenv("GIT_CONFIG_GLOBAL", filepath.Join(home, ".gitconfig"))
	return home
}

// sh runs a fixture-setup git command (not under the read-only contract).
func sh(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	cmd.Env = append(os.Environ(), "GIT_AUTHOR_DATE=2026-01-01T00:00:00Z", "GIT_COMMITTER_DATE=2026-01-01T00:00:00Z")
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
	return string(out)
}

func newRepo(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	sh(t, dir, "init", "-q", "-b", "trunk")
	sh(t, dir, "config", "user.email", "t@example.com")
	sh(t, dir, "config", "user.name", "t")
	return dir
}

func write(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func detect(t *testing.T) *Git {
	t.Helper()
	g, err := Detect(context.Background())
	if err != nil {
		t.Skipf("git not usable: %v", err)
	}
	return g
}

// markerScript creates an executable that records its invocation.
func markerScript(t *testing.T, bin, markers, name string) string {
	t.Helper()
	p := filepath.Join(bin, name+".sh")
	write(t, p, fmt.Sprintf("#!/bin/sh\ntouch '%s/%s'\ncat\n", markers, name))
	if err := os.Chmod(p, 0o755); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestContractSuppressesExternalProgramsAndWrites(t *testing.T) {
	home := hermetic(t)
	repo := newRepo(t)
	bin, markers := t.TempDir(), t.TempDir()
	mk := func(n string) string { return markerScript(t, bin, markers, n) }

	write(t, filepath.Join(repo, "a.txt"), "base\n")
	write(t, filepath.Join(repo, ".gitattributes"), "*.txt filter=evil diff=evil\n*.md filter=ev=il\n")
	write(t, filepath.Join(repo, "b.md"), "md\n")
	write(t, filepath.Join(repo, "same.md"), "same\n")
	sh(t, repo, "add", ".")
	sh(t, repo, "commit", "-qm", "init")
	// A signed-looking commit so that log.showSignature would call gpg.program.
	tree := strings.TrimSpace(sh(t, repo, "rev-parse", "HEAD^{tree}"))
	parent := strings.TrimSpace(sh(t, repo, "rev-parse", "HEAD"))
	obj := fmt.Sprintf("tree %s\nparent %s\nauthor t <t@example.com> 0 +0000\ncommitter t <t@example.com> 0 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n -----END PGP SIGNATURE-----\n\nsigned\n", tree, parent)
	write(t, filepath.Join(bin, "commit.txt"), obj)
	signed := strings.TrimSpace(sh(t, repo, "hash-object", "-t", "commit", "-w", filepath.Join(bin, "commit.txt")))
	sh(t, repo, "update-ref", "refs/heads/trunk", signed)

	for k, v := range map[string]string{
		"filter.evil.clean":         mk("clean"),
		"filter.evil.smudge":        mk("smudge"),
		"filter.evil.process":       mk("process"),
		"filter.evil.required":      "true",
		"filter.ev=il.clean":        mk("clean2"),
		"diff.evil.textconv":        mk("textconv"),
		"diff.evil.command":         mk("diffcmd"),
		"diff.external":             mk("extdiff"),
		"core.fsmonitor":            mk("fsmonitor"),
		"core.pager":                mk("pager"),
		"pager.status":              "true",
		"log.showSignature":         "true",
		"gpg.program":               mk("gpg"),
		"core.attributesFile":       filepath.Join(home, "attrs"),
		"status.showUntrackedFiles": "all",
	} {
		sh(t, repo, "config", k, v)
	}
	write(t, filepath.Join(home, "attrs"), "*.go filter=evil\n")
	write(t, filepath.Join(repo, ".git", "info", "attributes"), "*.txt filter=evil\n")

	// Same size + newer mtime forces status to hash the file (and run clean).
	time.Sleep(1100 * time.Millisecond)
	write(t, filepath.Join(repo, "a.txt"), "bas2\n")
	write(t, filepath.Join(repo, "b.md"), "mx\n")
	write(t, filepath.Join(repo, "c.go"), "package c\n")
	// Rewritten unchanged: only its stat differs, which `git diff` would
	// otherwise refresh into the index (diff.autoRefreshIndex).
	write(t, filepath.Join(repo, "same.md"), "same\n")

	// Hostile caller environment must not redirect anything.
	t.Setenv("GIT_DIR", t.TempDir())
	t.Setenv("GIT_INDEX_FILE", filepath.Join(bin, "other-index"))
	t.Setenv("GIT_EXTERNAL_DIFF", mk("envextdiff"))
	t.Setenv("GIT_CONFIG_PARAMETERS", "'core.fsmonitor'='"+mk("envfsmon")+"'")
	t.Setenv("GIT_PAGER", mk("envpager"))

	indexPath := filepath.Join(repo, ".git", "index")
	before, _ := os.ReadFile(indexPath)
	beforeStat, _ := os.Stat(indexPath)

	g := detect(t)
	ctx := context.Background()
	r, err := g.Open(ctx, repo)
	if err != nil {
		t.Fatal(err)
	}
	cmds := [][]string{
		{"status", "--porcelain=v2", "-z", "--untracked-files=all", "--ignore-submodules=dirty"},
		{"diff", "--no-ext-diff", "--no-textconv", "--no-color", "-z", "--name-status"},
		{"diff", "--no-ext-diff", "--no-textconv", "--no-color"},
		{"diff", "--no-ext-diff", "--no-textconv", "--no-color", "--cached"},
		{"diff", "--no-ext-diff", "--no-textconv", "--no-color", "HEAD"},
		{"log", "-1", "--format=%H"},
		{"merge-base", "--all", "HEAD", "HEAD~1"},
		{"cat-file", "blob", "HEAD:a.txt"},
	}
	for _, c := range cmds {
		if _, err := r.Run(ctx, c...); err != nil {
			t.Errorf("%v: %v", c, err)
		}
	}
	s, err := r.Snapshot(ctx)
	if err != nil {
		t.Fatalf("Snapshot: %v", err)
	}
	cmp, err := r.Compare(ctx, "HEAD~1", s)
	if err != nil || cmp.Unavailable != "" {
		t.Fatalf("Compare: %+v, %v", cmp, err)
	}
	for _, c := range cmp.Changes {
		if c.Kind == ChangeUntracked {
			continue
		}
		if _, err := r.BranchDiff(ctx, cmp, c, 0); err != nil {
			t.Errorf("BranchDiff %s: %v", c.Path, err)
		}
		if _, err := r.BaseContent(ctx, cmp, c, 0); err != nil {
			t.Errorf("BaseContent %s: %v", c.Path, err)
		}
	}

	entries, _ := os.ReadDir(markers)
	if len(entries) > 0 {
		var names []string
		for _, e := range entries {
			names = append(names, e.Name())
		}
		t.Errorf("external programs ran: %v", names)
	}
	after, _ := os.ReadFile(indexPath)
	afterStat, _ := os.Stat(indexPath)
	if !bytes.Equal(before, after) || !beforeStat.ModTime().Equal(afterStat.ModTime()) {
		t.Error(".git/index was modified")
	}
}

func TestOpenResolvesPrefixAndNonRepository(t *testing.T) {
	hermetic(t)
	g := detect(t)
	ctx := context.Background()

	if _, err := g.Open(ctx, t.TempDir()); !errors.Is(err, ErrNotRepository) {
		t.Errorf("non-repo: got %v", err)
	}

	repo := newRepo(t)
	sub := filepath.Join(repo, "docs", "sub")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	r, err := g.Open(ctx, sub)
	if err != nil {
		t.Fatal(err)
	}
	if r.Prefix != "docs/sub/" {
		t.Errorf("prefix = %q", r.Prefix)
	}
	want, _ := filepath.EvalSymlinks(repo)
	if r.Toplevel != want {
		t.Errorf("toplevel = %q, want %q", r.Toplevel, want)
	}
}

func TestSpecialFileNamesAndLiteralPathspec(t *testing.T) {
	hermetic(t)
	repo := newRepo(t)
	names := []string{"-n", "with space.txt", "new\nline.txt", "日本語.md", "*", ":(glob)x"}
	for _, n := range names {
		write(t, filepath.Join(repo, n), "x\n")
	}
	g := detect(t)
	ctx := context.Background()
	r, err := g.Open(ctx, repo)
	if err != nil {
		t.Fatal(err)
	}
	out, err := r.Run(ctx, "status", "--porcelain=v2", "-z", "--untracked-files=all")
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]bool{}
	for rec := range bytes.SplitSeq(out, []byte{0}) {
		if p, ok := bytes.CutPrefix(rec, []byte("? ")); ok {
			got[string(p)] = true
		}
	}
	for _, n := range names {
		if !got[n] {
			t.Errorf("status -z missing %q (got %v)", n, got)
		}
	}
	// "*" must match only the file literally named "*".
	out, err = r.Run(ctx, "ls-files", "-z", "--others", "--", "*")
	if err != nil {
		t.Fatal(err)
	}
	if string(out) != "*\x00" {
		t.Errorf("literal pathspec matched %q", out)
	}
}

func partialClone(t *testing.T) string {
	t.Helper()
	base := t.TempDir()
	seed := newRepo(t)
	write(t, filepath.Join(seed, "f.txt"), "one\n")
	sh(t, seed, "add", ".")
	sh(t, seed, "commit", "-qm", "1")
	write(t, filepath.Join(seed, "f.txt"), "two\n")
	sh(t, seed, "commit", "-qam", "2")
	bare := filepath.Join(base, "src.git")
	sh(t, base, "init", "-q", "--bare", bare)
	sh(t, base, "--git-dir="+bare, "config", "uploadpack.allowFilter", "true")
	sh(t, seed, "push", "-q", "--no-verify", bare, "HEAD:refs/heads/trunk")
	sh(t, base, "--git-dir="+bare, "symbolic-ref", "HEAD", "refs/heads/trunk")
	clone := filepath.Join(base, "pc")
	sh(t, base, "-c", "protocol.file.allow=always", "clone", "-q", "--filter=blob:none", "--no-checkout", "file://"+bare, clone)
	return clone
}

func TestPartialCloneDoesNotFetch(t *testing.T) {
	hermetic(t)
	g := detect(t)
	if g.Version.Less(LazyFetchVersion) {
		t.Skipf("git %s < %s: partial clones are refused instead (TestPartialCloneRefusedOnOldGit)", g.Version, LazyFetchVersion)
	}
	clone := partialClone(t)
	count := func() string { return sh(t, clone, "count-objects", "-v") }
	before := count()
	r, err := g.Open(context.Background(), clone)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := r.Run(context.Background(), "diff", "--no-ext-diff", "--no-textconv", "HEAD~1", "HEAD"); err == nil {
		t.Error("diff of missing blobs should fail instead of fetching")
	}
	if after := count(); after != before {
		t.Errorf("objects were fetched:\nbefore %s\nafter %s", before, after)
	}
}

// Older git cannot be stopped from lazy fetching, so a partial clone disables
// Git features instead.
func TestPartialCloneRefusedOnOldGit(t *testing.T) {
	hermetic(t)
	g := detect(t)
	clone := partialClone(t)
	old := &Git{Path: g.Path, Version: Version{2, 44, 0}}
	if _, err := old.Open(context.Background(), clone); !errors.Is(err, ErrUnavailable) {
		t.Errorf("partial clone on old git: %v", err)
	}
	if _, err := old.Open(context.Background(), newRepo(t)); err != nil {
		t.Errorf("regular repo on old git: %v", err)
	}
}

func TestRenameAcrossWorkspaceBoundary(t *testing.T) {
	hermetic(t)
	repo := newRepo(t)
	write(t, filepath.Join(repo, "old", "page.md"), strings.Repeat("same content\n", 20))
	write(t, filepath.Join(repo, "docs", "keep.md"), "k\n")
	sh(t, repo, "add", ".")
	sh(t, repo, "commit", "-qm", "init")
	sh(t, repo, "mv", "old/page.md", "docs/page.md")

	g := detect(t)
	ctx := context.Background()
	r, err := g.Open(ctx, filepath.Join(repo, "docs"))
	if err != nil {
		t.Fatal(err)
	}
	// Workspace-scoped pathspec would drop the rename pairing, so the whole
	// repository is queried and mapped afterwards.
	out, err := r.Run(ctx, "status", "--porcelain=v2", "-z", "--untracked-files=all")
	if err != nil {
		t.Fatal(err)
	}
	recs := bytes.Split(out, []byte{0})
	var newPath, origPath string
	for i, rec := range recs {
		if bytes.HasPrefix(rec, []byte("2 ")) {
			f := strings.SplitN(string(rec), " ", 10)
			newPath, origPath = f[9], string(recs[i+1])
		}
	}
	if newPath != "docs/page.md" || origPath != "old/page.md" {
		t.Fatalf("rename record = %q <- %q", newPath, origPath)
	}
	if rel, ok := r.FromGit(newPath); !ok || rel != "page.md" {
		t.Errorf("new side = %q,%v", rel, ok)
	}
	if _, ok := r.FromGit(origPath); ok {
		t.Error("old side outside the workspace was mapped inside")
	}
	if _, ok := r.FromGit("docs"); ok {
		t.Error("prefix without slash matched")
	}
	if got := r.ToGit("page.md"); got != "docs/page.md" {
		t.Errorf("ToGit = %q", got)
	}
}

// TestLoadStatus records status cost for P0-09. Run with
// REPLOR_LOAD=1 go test ./internal/git -run Load -v
func TestLoadStatus(t *testing.T) {
	if os.Getenv("REPLOR_LOAD") == "" {
		t.Skip("set REPLOR_LOAD=1")
	}
	hermetic(t)
	repo := newRepo(t)
	for d := range 200 {
		for f := range 100 {
			write(t, filepath.Join(repo, fmt.Sprintf("d%03d", d), fmt.Sprintf("f%03d.txt", f)), "x\n")
		}
	}
	sh(t, repo, "add", ".")
	sh(t, repo, "commit", "-qm", "bulk")
	for d := range 20 {
		write(t, filepath.Join(repo, fmt.Sprintf("d%03d", d), "f000.txt"), "changed\n")
		write(t, filepath.Join(repo, fmt.Sprintf("d%03d", d), "new.txt"), "n\n")
	}
	g := detect(t)
	ctx := context.Background()
	for i := range 3 {
		start := time.Now()
		r, err := g.Open(ctx, repo)
		if err != nil {
			t.Fatal(err)
		}
		out, err := r.Run(ctx, "status", "--porcelain=v2", "-z", "--untracked-files=all", "--ignore-submodules=dirty")
		if err != nil {
			t.Fatal(err)
		}
		t.Logf("run %d: files=20000 open+status=%v bytes=%d", i, time.Since(start).Round(time.Millisecond), len(out))
	}
}

// A filter driver configured after Open must still be neutralized.
func TestFilterAddedAfterOpenIsNeutralized(t *testing.T) {
	hermetic(t)
	repo := newRepo(t)
	bin, markers := t.TempDir(), t.TempDir()
	write(t, filepath.Join(repo, "a.txt"), "base\n")
	write(t, filepath.Join(repo, ".gitattributes"), "*.txt filter=late\n")
	sh(t, repo, "add", ".")
	sh(t, repo, "commit", "-qm", "init")

	g := detect(t)
	ctx := context.Background()
	r, err := g.Open(ctx, repo)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := r.Run(ctx, "status", "--porcelain=v2", "-z"); err != nil {
		t.Fatal(err)
	}

	sh(t, repo, "config", "filter.late.clean", markerScript(t, bin, markers, "late"))
	time.Sleep(1100 * time.Millisecond)
	write(t, filepath.Join(repo, "a.txt"), "bas2\n")
	for _, args := range [][]string{
		{"status", "--porcelain=v2", "-z"},
		{"diff", "--no-ext-diff", "--no-textconv"},
	} {
		if _, err := r.Run(ctx, args...); err != nil {
			t.Fatal(err)
		}
	}
	if entries, _ := os.ReadDir(markers); len(entries) > 0 {
		t.Error("filter configured after Open was executed")
	}
}

func TestOpenPathsWithNewlines(t *testing.T) {
	hermetic(t)
	root := filepath.Join(t.TempDir(), "re\npo")
	if err := os.MkdirAll(root, 0o755); err != nil {
		t.Fatal(err)
	}
	sh(t, root, "init", "-q")
	sub := filepath.Join(root, "d\nx")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	r, err := detect(t).Open(context.Background(), sub)
	if err != nil {
		t.Fatal(err)
	}
	want, _ := filepath.EvalSymlinks(root)
	if r.Toplevel != want || r.Prefix != "d\nx/" {
		t.Errorf("toplevel=%q prefix=%q", r.Toplevel, r.Prefix)
	}
	if rel, ok := r.FromGit("d\nx/f.md"); !ok || rel != "f.md" {
		t.Errorf("FromGit = %q, %v", rel, ok)
	}
}
