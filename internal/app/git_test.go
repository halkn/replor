package app

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	uv "github.com/charmbracelet/ultraviolet"
	"github.com/charmbracelet/x/ansi"
	"golang.org/x/text/unicode/norm"

	"github.com/halkn/replor/internal/preview"
)

// hermeticGit is isolateGit for tests that need git installed.
func hermeticGit(t *testing.T) {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git not installed")
	}
	isolateGit(t)
}

// isolateGit is applied to every UI test, since the model runs git on
// start-up. It covers the developer's global config: replor passes HOME and
// XDG_CONFIG_HOME to git. The system config still applies to replor's own
// git calls, because the process allowlist drops GIT_CONFIG_NOSYSTEM; those
// variables only isolate the fixture setup done with gitIn.
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

// The cursor follows its file when the ignored filter changes which rows
// exist: on the first git result and when `i` is pressed.
func TestCursorKeptByPathWhenRowsChange(t *testing.T) {
	dir := gitWorkspace(t)
	m := start(t, dir, 100, 20)
	if got := m.tree.rows[m.tree.cursor].node.Rel; got != "README.md" {
		t.Fatalf("cursor on %q after the git result, want README.md", got)
	}
	m.tree.setCursor(m.tree.indexOf("src"))
	drive(t, m, key("i"))
	if got := m.tree.rows[m.tree.cursor].node.Rel; got != "src" {
		t.Errorf("cursor on %q after i, want src", got)
	}
	// Expanding a directory above the cursor shifts rows below it.
	m.tree.setCursor(m.tree.indexOf("src"))
	m.tree.setCursor(m.tree.indexOf("docs"))
	drive(t, m, key("l"))
	m.tree.setCursor(m.tree.indexOf("src"))
	drive(t, m, key("i"))
	if got := m.tree.rows[m.tree.cursor].node.Rel; got != "src" {
		t.Errorf("cursor on %q after re-listing with docs expanded, want src", got)
	}
}

// macOS file systems may hand back decomposed (NFD) names while git prints
// them composed (NFC).
func TestDecomposedFileNamesMatchGitOutput(t *testing.T) {
	dir := gitWorkspace(t)
	nfd := "ガ.md"      // ガ decomposed
	ignored := "ギ.log" // ギ decomposed, matches *.log
	for _, n := range []string{nfd, ignored} {
		if err := os.WriteFile(filepath.Join(dir, n), []byte("x\n"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	gitIn(t, dir, "add", nfd)
	gitIn(t, dir, "commit", "-qm", "nfd")
	if err := os.WriteFile(filepath.Join(dir, nfd), []byte("changed\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	m := start(t, dir, 100, 30)
	var row, ignoredRow string
	for i, r := range m.tree.rows {
		switch norm.NFC.String(r.node.Name) {
		case "ガ.md":
			row = stripANSI(m.treeRow(i, m.sideWidth()))
		case "ギ.log":
			ignoredRow = r.node.Name
		}
	}
	if !strings.HasSuffix(row, "M") {
		t.Errorf("modified NFD file row %q", row)
	}
	if ignoredRow != "" {
		t.Errorf("ignored NFD file shown: %q", ignoredRow)
	}
}

// Every cell of the header carries the bar background, including the text
// after a coloured status letter.
func TestHeaderKeepsBarBackground(t *testing.T) {
	m := start(t, gitWorkspace(t), 100, 10)
	buf := uv.NewScreenBuffer(m.width, 1)
	uv.NewStyledString(screen(t, m)[0]).Draw(buf, uv.Rect(0, 0, m.width, 1))
	for x := range m.width {
		if c := buf.CellAt(x, 0); c == nil || c.Style.Bg == nil {
			t.Fatalf("header cell %d (%q) has no background", x, c.Content)
		}
	}
}

func TestGitFailureClearsMarks(t *testing.T) {
	dir := gitWorkspace(t)
	m := start(t, dir, 100, 20)
	if len(m.git.marks) == 0 {
		t.Fatal("precondition: marks")
	}
	if err := os.RemoveAll(filepath.Join(dir, ".git")); err != nil {
		t.Fatal(err)
	}
	drive(t, m, key("r"))
	if m.git.phase != gitFailed || len(m.git.marks) != 0 || len(m.git.dirty) != 0 {
		t.Errorf("phase %v marks %v dirty %v", m.git.phase, m.git.marks, m.git.dirty)
	}
	if strings.Contains(plain(screen(t, m)[0]), "M:") {
		t.Error("header still counts changes")
	}
}

func TestRenameMarksSourceDirectory(t *testing.T) {
	dir := gitWorkspace(t)
	gitIn(t, dir, "mv", "src/main.go", "main.go")
	m := start(t, dir, 100, 20)
	if !m.git.dirty["src"] {
		t.Errorf("src not marked after a file was renamed out of it: %v", m.git.dirty)
	}
}

// DESIGN 6.2: a file being viewed stays in the tree, marked, even when
// ignored files are hidden.
func TestViewedIgnoredFileStaysInTree(t *testing.T) {
	m := start(t, gitWorkspace(t), 100, 20)
	drive(t, m, key("i"))
	run(t, m, m.open(Target{Path: "debug.log"}, preview.Position{}, nil, true))
	drive(t, m, key("i"))
	if got := rowText(t, m, "debug.log"); !strings.HasSuffix(got, "!") {
		t.Errorf("viewed ignored file row %q", got)
	}
	if m.tree.indexOf("build") >= 0 {
		t.Error("other ignored entries must stay hidden")
	}
}

func TestSameSetTreatsMissingAsEmpty(t *testing.T) {
	if !sameSet(nil, map[string]bool{}) || sameSet(nil, map[string]bool{"a": true}) {
		t.Error("sameSet: nil must equal the empty set")
	}
}
