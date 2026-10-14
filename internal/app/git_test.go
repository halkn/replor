package app

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/charmbracelet/x/ansi"
)

// hermeticGit isolates git from the developer's configuration (see
// docs/development.md).
func hermeticGit(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not installed")
	}
	isolateGit(t)
}

// isolateGit is applied to every UI test, since the model runs git on
// start-up.
func isolateGit(t *testing.T) {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("XDG_CONFIG_HOME", filepath.Join(home, ".config"))
	t.Setenv("GIT_CONFIG_NOSYSTEM", "1")
	t.Setenv("GIT_CONFIG_GLOBAL", filepath.Join(home, ".gitconfig"))
}

func gitIn(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %v: %v\n%s", args, err, out)
	}
}

// gitWorkspace builds a repository with every status the tree marks.
func gitWorkspace(t *testing.T) string {
	t.Helper()
	hermeticGit(t)
	dir := writeFiles(t, map[string]string{
		"README.md":     "# readme\n",
		"docs/guide.md": "# guide\n",
		"docs/keep.md":  "# keep\n",
		".gitignore":    "build/\n*.log\n",
		"tracked.log":   "tracked\n",
		"src/main.go":   "package main\n",
	})
	gitIn(t, dir, "init", "-q", "-b", "trunk")
	gitIn(t, dir, "config", "user.email", "t@example.com")
	gitIn(t, dir, "config", "user.name", "t")
	gitIn(t, dir, "add", ".")
	gitIn(t, dir, "add", "-f", "tracked.log")
	gitIn(t, dir, "commit", "-qm", "base")
	for name, content := range map[string]string{
		"docs/guide.md":    "# guide changed\n",
		"new.md":           "new\n",
		"build/out.bin":    "bin\n",
		"build/deep/x.txt": "x\n",
		"debug.log":        "log\n",
	} {
		p := filepath.Join(dir, name)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

func rowText(t *testing.T, m *Model, rel string) string {
	t.Helper()
	i := m.tree.indexOf(rel)
	if i < 0 {
		return ""
	}
	return strings.TrimRight(stripANSI(m.treeRow(i, m.sideWidth())), " ")
}

func stripANSI(s string) string { return strings.Join(strings.Fields(plain(s)), " ") }

func TestGitMarksAndIgnoredFiles(t *testing.T) {
	dir := gitWorkspace(t)
	m := start(t, dir, 100, 20)
	if m.git.phase != gitReady {
		t.Fatalf("git phase %v: %s", m.git.phase, m.git.reason)
	}
	header := plain(screen(t, m)[0])
	for _, want := range []string{"trunk", "M:1", "?:1"} {
		if !strings.Contains(header, want) {
			t.Errorf("header lacks %q: %q", want, header)
		}
	}
	if got := rowText(t, m, "new.md"); !strings.HasSuffix(got, "?") {
		t.Errorf("new.md row %q", got)
	}
	if got := rowText(t, m, "docs"); !strings.HasSuffix(got, "•") {
		t.Errorf("docs row %q (dirty directory)", got)
	}
	if got := rowText(t, m, "tracked.log"); got == "" || strings.HasSuffix(got, "!") {
		t.Errorf("tracked file matching an ignore pattern: %q", got)
	}
	for _, hidden := range []string{"build", "debug.log"} {
		if m.tree.indexOf(hidden) >= 0 {
			t.Errorf("%s shown while ignored files are hidden", hidden)
		}
	}

	// The finder applies the same filter.
	drive(t, m, key("f"))
	for _, p := range m.finder.paths {
		if strings.HasPrefix(p, "build/") || p == "debug.log" {
			t.Errorf("finder lists ignored %s", p)
		}
	}
	drive(t, m, key("esc"))

	drive(t, m, key("i"))
	if got := rowText(t, m, "build"); !strings.HasSuffix(got, "!") {
		t.Errorf("build row %q after i", got)
	}
	drive(t, m, key("f"))
	found := false
	for _, p := range m.finder.paths {
		found = found || p == "build/deep/x.txt"
	}
	if !found {
		t.Error("finder does not list ignored files after i")
	}
}

func TestGitStatesInHeader(t *testing.T) {
	hermeticGit(t)
	m := start(t, writeFiles(t, map[string]string{"a.md": "# a\n"}), 100, 10)
	if m.git.phase != gitNone || !strings.Contains(plain(screen(t, m)[0]), "no git") {
		t.Errorf("non-repo: phase %v header %q", m.git.phase, plain(screen(t, m)[0]))
	}

	t.Setenv("PATH", t.TempDir()) // no git on PATH
	m = start(t, writeFiles(t, map[string]string{"a.md": "# a\n"}), 100, 10)
	if m.git.phase != gitUnavailable || !strings.Contains(plain(screen(t, m)[0]), "git is not installed") {
		t.Errorf("no git: phase %v header %q", m.git.phase, plain(screen(t, m)[0]))
	}
	if m.tree.indexOf("a.md") < 0 {
		t.Error("file browsing must continue without git")
	}
}

// Started in a subdirectory, paths are relative to it and changes outside
// are not counted.
func TestGitSubdirectoryWorkspace(t *testing.T) {
	dir := gitWorkspace(t)
	m := start(t, filepath.Join(dir, "docs"), 100, 20)
	if m.git.phase != gitReady || m.git.marks["guide.md"] != 'M' {
		t.Fatalf("marks %v (%s)", m.git.marks, m.git.reason)
	}
	header := plain(screen(t, m)[0])
	if strings.Contains(header, "?:") {
		t.Errorf("untracked file outside the workspace counted: %q", header)
	}
}

func plain(s string) string { return ansi.Strip(s) }
