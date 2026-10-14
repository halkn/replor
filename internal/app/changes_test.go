package app

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/git"
	"github.com/halkn/replor/internal/preview"
)

// changesWorkspace has a staged and unstaged change to the same file, a
// deletion, a rename and an untracked file.
func changesWorkspace(t *testing.T) string {
	t.Helper()
	dir := gitWorkspace(t) // docs/guide.md unstaged, new.md untracked
	write := func(name, content string) {
		t.Helper()
		if err := os.WriteFile(filepath.Join(dir, name), []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	write("README.md", "# readme staged\n")
	gitIn(t, dir, "add", "README.md")
	write("README.md", "# readme unstaged\n")
	if err := os.Remove(filepath.Join(dir, "docs", "keep.md")); err != nil {
		t.Fatal(err)
	}
	gitIn(t, dir, "mv", "src/main.go", "src/app.go")
	return dir
}

func entryIndex(m *Model, section, path string) int {
	for i, e := range m.changes.entries {
		if e.section == section && e.path() == path {
			return i
		}
	}
	return -1
}

func openEntry(t *testing.T, m *Model, section, path string) {
	t.Helper()
	i := entryIndex(m, section, path)
	if i < 0 {
		t.Fatalf("no %s entry for %s: %+v", section, path, m.changes.entries)
	}
	m.changes.setCursor(i)
	m.focus = paneFiles
	drive(t, m, key("enter"))
}

func TestChangesPaneAndDiffs(t *testing.T) {
	m := start(t, changesWorkspace(t), 120, 30)
	drive(t, m, key("c"))
	if m.left != leftChanges {
		t.Fatal("c did not switch to Changes")
	}
	s := plainScreen(t, m)
	for _, want := range []string{"Staged", "Unstaged", "Untracked", "src/main.go → src/app.go", "new.md"} {
		if !strings.Contains(s, want) {
			t.Errorf("Changes pane lacks %q:\n%s", want, s)
		}
	}
	if entryIndex(m, sectionStaged, "README.md") < 0 || entryIndex(m, sectionUnstaged, "README.md") < 0 {
		t.Error("README.md needs separate staged and unstaged entries")
	}

	openEntry(t, m, sectionStaged, "README.md")
	all := strings.Join(m.viewer.rendered().Plain, "\n")
	if m.viewer.target.Mode != ModeDiff || !strings.Contains(all, "+# readme staged") || strings.Contains(all, "unstaged") {
		t.Errorf("staged diff:\n%s", all)
	}
	if h := plain(screen(t, m)[0]); !strings.Contains(h, "Diff staged") {
		t.Errorf("header: %q", h)
	}

	openEntry(t, m, sectionUnstaged, "docs/keep.md")
	if all := strings.Join(m.viewer.rendered().Plain, "\n"); !strings.Contains(all, "deleted file mode") {
		t.Errorf("deleted:\n%s", all)
	}
	openEntry(t, m, sectionStaged, "src/app.go")
	if all := strings.Join(m.viewer.rendered().Plain, "\n"); !strings.Contains(all, "rename from src/main.go") {
		t.Errorf("rename:\n%s", all)
	}
	openEntry(t, m, sectionUntracked, "new.md")
	if all := strings.Join(m.viewer.rendered().Plain, "\n"); !strings.Contains(all, "+new") || !strings.Contains(all, "untracked") {
		t.Errorf("untracked:\n%s", all)
	}

	// History covers diffs.
	drive(t, m, key("backspace"))
	if m.viewer.target.Path != "src/app.go" || m.viewer.target.Mode != ModeDiff {
		t.Errorf("back: %+v", m.viewer.target)
	}
	for _, sz := range [][2]int{{80, 20}, {50, 12}} {
		drive(t, m, tea.WindowSizeMsg{Width: sz[0], Height: sz[1]})
		screen(t, m)
	}
	drive(t, m, key("c"))
	if m.left != leftFiles {
		t.Error("c did not switch back to Files")
	}
}

func TestDKeyCyclesDiffs(t *testing.T) {
	m := start(t, changesWorkspace(t), 120, 30) // README.md opened at start-up
	m.focus = paneViewer
	var modes []string
	for range 4 {
		drive(t, m, key("d"))
		tg := m.viewer.target
		if tg.Mode == ModeDiff {
			modes = append(modes, "diff:"+tg.Scope.String())
		} else {
			modes = append(modes, "preview")
		}
	}
	if got := strings.Join(modes, ","); got != "diff:unstaged,diff:staged,preview,diff:unstaged" {
		t.Errorf("d cycle = %s", got)
	}
	run(t, m, m.open(Target{Path: "docs/guide.md"}, preview.Position{}, nil, false))
	run(t, m, m.open(Target{Path: "tracked.log"}, preview.Position{}, nil, false))
	drive(t, m, key("d"))
	if m.viewer.target.Mode == ModeDiff || !strings.Contains(m.status, "no changes") {
		t.Errorf("unchanged file: %+v %q", m.viewer.target, m.status)
	}
}

// A diff made from an older snapshot than the list is not shown.
func TestDiffFromOlderSnapshotIsReloaded(t *testing.T) {
	m := start(t, changesWorkspace(t), 120, 30)
	cmd := m.open(Target{Path: "README.md", Mode: ModeDiff, Scope: git.ScopeUnstaged}, preview.Position{}, nil, true)
	msg := cmd().(loadedMsg)
	m.git.snap.Revision++ // a newer snapshot arrived meanwhile
	_, again := m.Update(msg)
	if m.viewer.rendered() != nil || again == nil {
		t.Fatal("stale diff applied")
	}
	run(t, m, again)
	if m.viewer.rendered() == nil || !strings.Contains(strings.Join(m.viewer.rendered().Plain, "\n"), "+# readme unstaged") {
		t.Error("diff not reloaded")
	}
}

// `r` refreshes the list and the diff together; a change that disappeared
// is reported instead of showing its old diff.
func TestRefreshUpdatesDiff(t *testing.T) {
	dir := changesWorkspace(t)
	m := start(t, dir, 120, 30)
	run(t, m, m.open(Target{Path: "docs/guide.md", Mode: ModeDiff, Scope: git.ScopeUnstaged}, preview.Position{}, nil, true))
	gitIn(t, dir, "checkout", "--", "docs/guide.md")
	drive(t, m, key("r"))
	if m.viewer.err == nil || !strings.Contains(plainScreen(t, m), "no longer changed") {
		t.Errorf("err %v", m.viewer.err)
	}
	if entryIndex(m, sectionUnstaged, "docs/guide.md") >= 0 {
		t.Error("list not refreshed")
	}
}

func TestConflictOpensWorkTreeFile(t *testing.T) {
	hermeticGit(t)
	dir := writeFiles(t, map[string]string{"c.md": "base\n"})
	gitIn(t, dir, "init", "-q", "-b", "trunk")
	gitIn(t, dir, "config", "user.email", "t@example.com")
	gitIn(t, dir, "config", "user.name", "t")
	gitIn(t, dir, "add", ".")
	gitIn(t, dir, "commit", "-qm", "base")
	gitIn(t, dir, "switch", "-qc", "other")
	if err := os.WriteFile(filepath.Join(dir, "c.md"), []byte("other\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	gitIn(t, dir, "commit", "-qam", "other")
	gitIn(t, dir, "switch", "-q", "trunk")
	if err := os.WriteFile(filepath.Join(dir, "c.md"), []byte("trunk\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	gitIn(t, dir, "commit", "-qam", "trunk")
	_ = exec.Command("git", "-C", dir, "merge", "-q", "other").Run() // conflicts by design

	m := start(t, dir, 100, 20)
	drive(t, m, key("c"))
	openEntry(t, m, sectionConflicts, "c.md")
	if m.viewer.target.Mode != ModeSource || !strings.Contains(m.status, "conflict") || !onScreen(m, "<<<<<<<") {
		t.Errorf("conflict: %+v status %q", m.viewer.target, m.status)
	}
	drive(t, m, key("d"))
	if m.viewer.target.Mode == ModeDiff {
		t.Error("d offered a diff for a conflict")
	}
}

func TestChangesNeedGit(t *testing.T) {
	m := start(t, writeFiles(t, map[string]string{"a.md": "# a\n"}), 100, 20)
	drive(t, m, key("c"))
	if m.left != leftFiles || !strings.Contains(m.status, "git") {
		t.Errorf("left %v status %q", m.left, m.status)
	}
}
