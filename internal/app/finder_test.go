package app

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/halkn/replor/internal/workspace"
)

func finderPaths(m *Model) []string {
	var out []string
	for _, r := range m.finder.results {
		out = append(out, m.finder.paths[r.Index])
	}
	return out
}

func TestFinderFindsUnexpandedFilesAndOpens(t *testing.T) {
	dir := writeFiles(t, map[string]string{
		"README.md":            "# r\n",
		"docs/guide/setup.md":  "# Setup\n",
		"src/deep/nested/a.go": "package a\n",
		".git/HEAD":            "ref\n",
		"docs/設計書.md":          "# 設計\n",
		"evil\x1b[2Jname.md":   "x\n",
	})
	m := start(t, dir, 100, 20)
	if m.tree.indexOf("docs/guide/setup.md") >= 0 {
		t.Fatal("precondition: docs must not be expanded")
	}
	drive(t, m, key("f"))
	if m.overlay != overlayFinder || m.finder.enumerating {
		t.Fatalf("overlay %v enumerating %v", m.overlay, m.finder.enumerating)
	}
	for _, p := range m.finder.paths {
		if strings.HasPrefix(p, ".git") {
			t.Errorf(".git listed: %s", p)
		}
	}

	// Global keys are typed into the query.
	typeText(t, m, "setup")
	if m.finder.query != "setup" || m.overlay != overlayFinder {
		t.Fatalf("query %q overlay %v", m.finder.query, m.overlay)
	}
	if got := finderPaths(m); len(got) != 1 || got[0] != "docs/guide/setup.md" {
		t.Fatalf("results %v", got)
	}
	s := plainScreen(t, m)
	if !strings.Contains(s, "1 of 5 files") || !strings.Contains(s, "docs/guide/setup.md") {
		t.Errorf("screen:\n%s", s)
	}
	drive(t, m, key("enter"))
	if m.overlay != overlayNone || m.viewer.target.Path != "docs/guide/setup.md" || !onScreen(m, "Setup") {
		t.Fatalf("not opened: %+v", m.viewer.target)
	}
	drive(t, m, key("backspace"))
	if m.viewer.target.Path != "README.md" {
		t.Errorf("back: %+v", m.viewer.target)
	}

	// Reopening keeps the list and the query; Japanese and hostile names draw safely.
	drive(t, m, key("f"))
	if m.finder.query != "setup" {
		t.Errorf("query not kept: %q", m.finder.query)
	}
	for range len("setup") {
		drive(t, m, key("backspace"))
	}
	typeText(t, m, "設計")
	if got := finderPaths(m); len(got) != 1 || got[0] != "docs/設計書.md" {
		t.Errorf("japanese: %v", got)
	}
	for range 2 {
		drive(t, m, key("backspace"))
	}
	typeText(t, m, "evil")
	if s := strings.Join(screen(t, m), "\n"); strings.Contains(s, "\x1b[2J") {
		t.Error("hostile file name reached the screen")
	}
	drive(t, m, key("esc"))
	if m.overlay != overlayNone {
		t.Error("esc did not close")
	}
}

func TestFinderPartialEnumeration(t *testing.T) {
	old := finderLimits
	finderLimits = workspace.Limits{MaxEntries: 2, MaxDepth: 64}
	t.Cleanup(func() { finderLimits = old })
	dir := writeFiles(t, map[string]string{"a.md": "", "b.md": "", "c.md": "", "d.md": ""})
	m := start(t, dir, 100, 20)
	drive(t, m, key("f"))
	if len(m.finder.paths) != 2 || m.finder.reason != workspace.LimitedEntries {
		t.Fatalf("paths %v reason %q", m.finder.paths, m.finder.reason)
	}
	if !strings.Contains(plainScreen(t, m), "partial: entry limit reached") {
		t.Error("partial state not shown")
	}
}

// `r` starts a new walk; batches of the old one are ignored.
func TestFinderReloadDiscardsOldWalk(t *testing.T) {
	dir := writeFiles(t, map[string]string{"a.md": ""})
	m := start(t, dir, 100, 20)
	m.overlay = overlayFinder
	old := m.enumerate()
	// Let the old walk put its batch into the channel before it is
	// cancelled, so that rejection, not cancellation, has to drop it.
	time.Sleep(100 * time.Millisecond)
	if err := os.WriteFile(filepath.Join(dir, "b.md"), nil, 0o644); err != nil {
		t.Fatal(err)
	}
	fresh := m.invalidateFinder()
	run(t, m, fresh)
	run(t, m, old) // stale batches arrive late
	if len(m.finder.paths) != 2 {
		t.Errorf("paths %v", m.finder.paths)
	}
}
