package app

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/git"
	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/workspace"
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

func TestRenamedOutOfWorkspaceIsNotDiffed(t *testing.T) {
	hermeticGit(t)
	dir := writeFiles(t, map[string]string{"docs/out.md": strings.Repeat("inside\n", 10)})
	gitIn(t, dir, "init", "-q", "-b", "trunk")
	gitIn(t, dir, "config", "user.email", "t@example.com")
	gitIn(t, dir, "config", "user.name", "t")
	gitIn(t, dir, "add", ".")
	gitIn(t, dir, "commit", "-qm", "base")
	gitIn(t, dir, "mv", "docs/out.md", "moved.md")
	if err := os.WriteFile(filepath.Join(dir, "moved.md"), []byte(strings.Repeat("inside\n", 10)+"outside secret\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	m := start(t, filepath.Join(dir, "docs"), 100, 20)
	drive(t, m, key("c"))
	for _, section := range []string{sectionStaged, sectionUnstaged} {
		if entryIndex(m, section, "out.md") < 0 {
			continue
		}
		openEntry(t, m, section, "out.md")
		if r := m.viewer.rendered(); r != nil && strings.Contains(strings.Join(r.Plain, "\n"), "outside secret") {
			t.Fatalf("%s diff shows content outside the workspace", section)
		}
		if !strings.Contains(m.status, "outside") {
			t.Errorf("%s: status %q", section, m.status)
		}
	}
}

func TestDecomposedNameHasDiff(t *testing.T) {
	dir := gitWorkspace(t)
	nfd := "ガ.md"
	if err := os.WriteFile(filepath.Join(dir, nfd), []byte("a\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	gitIn(t, dir, "add", nfd)
	gitIn(t, dir, "commit", "-qm", "nfd")
	if err := os.WriteFile(filepath.Join(dir, nfd), []byte("b\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	m := start(t, dir, 100, 20)
	run(t, m, m.open(Target{Path: nfd}, preview.Position{}, nil, false))
	if !strings.Contains(plain(screen(t, m)[0]), "Diff") {
		t.Errorf("no Diff tab: %q", plain(screen(t, m)[0]))
	}
	drive(t, m, key("d"))
	if m.viewer.target.Mode != ModeDiff || m.viewer.err != nil {
		t.Errorf("d on an NFD name: %+v %v %q", m.viewer.target, m.viewer.err, m.status)
	}
}

func TestShowingChangesPaneRelayouts(t *testing.T) {
	m := start(t, changesWorkspace(t), 120, 30)
	drive(t, m, key("e"))
	drive(t, m, key("c"))
	if r := m.viewer.rendered(); r == nil || r.Width != m.viewerWidth() {
		t.Errorf("rendered at width %v, viewer is %d", r.Width, m.viewerWidth())
	}
}

func TestChangesHeadingStaysVisible(t *testing.T) {
	dir := gitWorkspace(t)
	for i := range 40 {
		if err := os.WriteFile(filepath.Join(dir, fmt.Sprintf("u%02d.md", i)), nil, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	m := start(t, dir, 100, 12)
	drive(t, m, key("c"))
	// The scroll position follows the cursor when the pane is drawn.
	drive(t, m, key("G"))
	screen(t, m)
	drive(t, m, key("g"))
	screen(t, m)
	if first := plain(m.changesLines(m.bodyHeight())[0]); !strings.HasPrefix(first, "Unstaged") {
		t.Errorf("first row %q, want the section heading", first)
	}
}

func TestGitFailureClearsDiff(t *testing.T) {
	dir := changesWorkspace(t)
	m := start(t, dir, 120, 30)
	run(t, m, m.open(Target{Path: "README.md", Mode: ModeDiff, Scope: git.ScopeUnstaged}, preview.Position{}, nil, true))
	if err := os.RemoveAll(filepath.Join(dir, ".git")); err != nil {
		t.Fatal(err)
	}
	drive(t, m, key("r"))
	if m.viewer.rendered() != nil {
		t.Error("diff from the previous snapshot still shown after git failed")
	}
	drive(t, m, key("c"))
	if first := plain(m.changesLines(m.bodyHeight())[0]); !strings.HasPrefix(first, "git:") {
		t.Errorf("Changes pane does not explain the failure: %q", first)
	}
}

func TestDiffTargetBecomingConflictReturnsToFile(t *testing.T) {
	m := start(t, changesWorkspace(t), 120, 30)
	run(t, m, m.open(Target{Path: "README.md", Mode: ModeDiff, Scope: git.ScopeUnstaged}, preview.Position{}, nil, true))
	// Simulate the next snapshot reporting a conflict for the file.
	for i, c := range m.git.snap.Changes {
		if c.Path == "README.md" {
			m.git.snap.Changes[i].Kind = git.ChangeUnmerged
		}
	}
	m.git.index(m.git.snap)
	drive(t, m, key("d"))
	if m.viewer.target.Mode == ModeDiff {
		t.Errorf("d did not leave the diff of a conflicted file: %+v", m.viewer.target)
	}
}

func TestTabsMatchWhatSWorks(t *testing.T) {
	dir := changesWorkspace(t)
	if err := os.WriteFile(filepath.Join(dir, "empty.md"), nil, 0o644); err != nil {
		t.Fatal(err)
	}
	m := start(t, dir, 120, 30)
	run(t, m, m.open(Target{Path: "empty.md"}, preview.Position{}, nil, false))
	if h := plain(screen(t, m)[0]); strings.Contains(h, "Source") {
		t.Errorf("empty Markdown offers Source: %q", h)
	}
	run(t, m, m.open(Target{Path: "README.md", Mode: ModeDiff, Scope: git.ScopeStaged}, preview.Position{}, nil, false))
	if h := plain(screen(t, m)[0]); strings.Contains(h, "Source") {
		drive(t, m, key("s"))
		if m.viewer.target.Mode != ModeSource {
			t.Errorf("Source tab shown but s refused: %q", m.status)
		}
	}
}

func TestChangesSelectionFollowsCursorWhenEntryDisappears(t *testing.T) {
	dir := changesWorkspace(t)
	m := start(t, dir, 120, 30)
	drive(t, m, key("c"))
	m.changes.setCursor(entryIndex(m, sectionStaged, "README.md"))
	gitIn(t, dir, "restore", "--staged", "README.md")
	drive(t, m, key("r"))
	moved := m.changes.entries[m.changes.cursor].key()
	gitIn(t, dir, "add", "README.md")
	drive(t, m, key("r"))
	if got := m.changes.entries[m.changes.cursor].key(); got != moved {
		t.Errorf("cursor jumped back to %q, want %q", got, moved)
	}
}

// A diff does not wrap, so a resize reuses it; `r` reloads it once, after
// the new snapshot.
func TestDiffNotReloadedNeedlessly(t *testing.T) {
	m := start(t, changesWorkspace(t), 120, 30)
	run(t, m, m.open(Target{Path: "README.md", Mode: ModeDiff, Scope: git.ScopeUnstaged}, preview.Position{}, nil, true))
	if _, cmd := m.Update(tea.WindowSizeMsg{Width: 90, Height: 30}); cmd != nil {
		t.Error("resize reloads the diff")
	}
	m.Update(key("r"))
	if m.viewer.pending {
		t.Error("r loads the diff before the new snapshot")
	}
}

func TestChangesWhileGitLoads(t *testing.T) {
	isolateGit(t)
	ws, err := workspace.Open(changesWorkspace(t))
	if err != nil {
		t.Fatal(err)
	}
	defer ws.Close()
	m := New(ws, Options{})
	m.Update(tea.WindowSizeMsg{Width: 100, Height: 20})
	m.Update(key("c"))
	if !strings.Contains(m.status, "loading") {
		t.Errorf("status %q", m.status)
	}
}

func TestUntrackedDirectoryAndSymlink(t *testing.T) {
	dir := gitWorkspace(t)
	nested := filepath.Join(dir, "vendor", "nested")
	if err := os.MkdirAll(nested, 0o755); err != nil {
		t.Fatal(err)
	}
	gitIn(t, nested, "init", "-q")
	if err := os.Symlink("README.md", filepath.Join(dir, "link.md")); err != nil {
		t.Fatal(err)
	}
	m := start(t, dir, 100, 20)
	drive(t, m, key("c"))
	openEntry(t, m, sectionUntracked, "vendor/nested")
	if m.viewer.err != nil || !strings.Contains(strings.Join(m.viewer.rendered().Plain, "\n"), "directory") {
		t.Errorf("nested repository: %v", m.viewer.err)
	}
	openEntry(t, m, sectionUntracked, "link.md")
	if all := strings.Join(m.viewer.rendered().Plain, "\n"); !strings.Contains(all, "symbolic link") || !strings.Contains(all, "README.md") || strings.Contains(all, "# readme") {
		t.Errorf("symlink:\n%s", all)
	}
}
