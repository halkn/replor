package app

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/workspace"
)

// startWatching starts the model with watching on. Ticks are not driven by
// run; tick() performs one scan explicitly.
func startWatching(t *testing.T, target string) *Model {
	t.Helper()
	old := watchInterval
	watchInterval = time.Millisecond
	t.Cleanup(func() { watchInterval = old })
	isolateGit(t)
	ws, err := workspace.Open(target)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ws.Close() })
	m := New(ws, Options{Watch: true})
	run(t, m, m.Init())
	drive(t, m, tea.WindowSizeMsg{Width: 100, Height: 20})
	tick(t, m) // baseline
	return m
}

func tick(t *testing.T, m *Model) {
	t.Helper()
	drive(t, m, watchTickMsg{})
}

// change writes a file with an mtime in the future so that coarse mtime
// file systems see it as changed.
func change(t *testing.T, p, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	later := time.Now().Add(3 * time.Second)
	if err := os.Chtimes(p, later, later); err != nil {
		t.Fatal(err)
	}
}

func TestWatchReloadsViewedFileKeepingPosition(t *testing.T) {
	var src strings.Builder
	for i := range 60 {
		src.WriteString("line " + string(rune('A'+i%26)) + strings.Repeat("x", i) + "\n\n")
	}
	dir := gitWorkspace(t)
	change(t, filepath.Join(dir, "long.md"), src.String())
	m := startWatching(t, filepath.Join(dir, "long.md"))
	m.focus = paneViewer
	for range 15 {
		drive(t, m, key("j"))
	}
	before := topLine(m)
	change(t, filepath.Join(dir, "long.md"), src.String()+"\nappended at the end\n")
	tick(t, m)
	if all := strings.Join(m.viewer.rendered().Plain, "\n"); !strings.Contains(all, "appended at the end") {
		t.Fatal("viewed file not reloaded")
	}
	if topLine(m) != before {
		t.Errorf("position lost: top %q, was %q", topLine(m), before)
	}
}

func TestWatchUpdatesTreeAndGit(t *testing.T) {
	dir := gitWorkspace(t)
	m := startWatching(t, dir)
	m.tree.setCursor(m.tree.indexOf("src"))
	drive(t, m, key("l")) // expand src
	tick(t, m)            // new baseline including src
	m.tree.setCursor(m.tree.indexOf("src/main.go"))

	change(t, filepath.Join(dir, "src", "added.go"), "package main\n")
	change(t, filepath.Join(dir, "top.md"), "# top\n")
	tick(t, m)
	if m.tree.indexOf("src/added.go") < 0 || m.tree.indexOf("top.md") < 0 {
		t.Fatalf("new files not listed: %v", m.tree.rows)
	}
	if got := m.tree.rows[m.tree.cursor].node.Rel; got != "src/main.go" {
		t.Errorf("cursor on %q, want src/main.go", got)
	}
	if m.git.markOf("src/added.go") != '?' {
		t.Errorf("git not refreshed: %v", m.git.marks)
	}

	// `git add` only touches the index.
	gitIn(t, dir, "add", "src/added.go")
	tick(t, m)
	if m.git.markOf("src/added.go") != 'A' {
		t.Errorf("index change not seen: %q", m.git.markOf("src/added.go"))
	}
}

func TestWatchShowsDeletedFile(t *testing.T) {
	dir := gitWorkspace(t)
	m := startWatching(t, filepath.Join(dir, "docs", "keep.md"))
	if err := os.Remove(filepath.Join(dir, "docs", "keep.md")); err != nil {
		t.Fatal(err)
	}
	tick(t, m)
	s := plainScreen(t, m)
	if !strings.Contains(s, "keep.md was deleted") || !strings.Contains(s, "Press d") {
		t.Errorf("screen:\n%s", s)
	}
	drive(t, m, key("d"))
	if m.viewer.target.Mode != ModeDiff {
		t.Error("d did not open the deletion")
	}
}

// In a linked worktree the index lives in its own git directory and the
// branch ref in the common one; a commit touches both.
func TestWatchWorktree(t *testing.T) {
	dir := gitWorkspace(t)
	wt := filepath.Join(t.TempDir(), "wt")
	gitIn(t, dir, "worktree", "add", "-q", "-b", "side", wt)
	m := startWatching(t, wt)
	change(t, filepath.Join(wt, "wt.md"), "w\n")
	tick(t, m)
	if m.git.markOf("wt.md") != '?' {
		t.Fatalf("marks %v", m.git.marks)
	}
	gitIn(t, wt, "add", "wt.md")
	gitIn(t, wt, "-c", "user.email=t@example.com", "-c", "user.name=t", "commit", "-qm", "wt")
	tick(t, m)
	if m.git.markOf("wt.md") != 0 {
		t.Errorf("commit in the worktree not seen: %v", m.git.marks)
	}
}

func TestWatchLimitIsShown(t *testing.T) {
	old := watchMaxEntries
	watchMaxEntries = 2
	t.Cleanup(func() { watchMaxEntries = old })
	m := startWatching(t, gitWorkspace(t))
	if !m.watch.limited || !strings.Contains(plain(screen(t, m)[0]), "watch partial") {
		t.Error("partial watching not shown")
	}
}

func TestNoWatch(t *testing.T) {
	isolateGit(t)
	ws, err := workspace.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer ws.Close()
	m := New(ws, Options{Watch: false})
	if m.scheduleWatch() != nil {
		t.Error("ticks scheduled with watching off")
	}
}

// Started in a subdirectory, no listed directory contains .git: an index
// change (`git add`) is seen only through git's metadata files.
func TestWatchIndexOnlyChange(t *testing.T) {
	dir := gitWorkspace(t)
	change(t, filepath.Join(dir, "src", "x.go"), "package main\n")
	m := startWatching(t, filepath.Join(dir, "src"))
	if m.git.markOf("x.go") != '?' {
		t.Fatalf("marks %v", m.git.marks)
	}
	gitIn(t, dir, "add", "src/x.go")
	tick(t, m)
	if m.git.markOf("x.go") != 'A' {
		t.Errorf("index change not seen: %q", m.git.markOf("x.go"))
	}
}
