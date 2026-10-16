package app

import (
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/git"
	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/process"
	"github.com/halkn/replor/internal/workspace"
)

// branchWorkspace is gitWorkspace with its base commit published as
// origin/trunk (origin/HEAD pointing at it) and a feature branch that
// changed docs/keep.md, added added.md and removed tracked.log; the work
// tree changes of gitWorkspace stay uncommitted.
func branchWorkspace(t *testing.T) string {
	t.Helper()
	dir := gitWorkspace(t)
	gitIn(t, dir, "update-ref", "refs/remotes/origin/trunk", "HEAD")
	gitIn(t, dir, "symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk")
	gitIn(t, dir, "switch", "-qc", "feature")
	for name, content := range map[string]string{"docs/keep.md": "# keep on branch\n", "added.md": "added\n"} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	gitIn(t, dir, "add", "docs/keep.md", "added.md")
	gitIn(t, dir, "rm", "-q", "tracked.log")
	gitIn(t, dir, "commit", "-qm", "feature")
	return dir
}

func startWith(t *testing.T, target string, opts Options, w, h int) *Model {
	t.Helper()
	isolateGit(t)
	ws, err := workspace.Open(target)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ws.Close() })
	m := New(ws, opts)
	run(t, m, m.Init())
	drive(t, m, tea.WindowSizeMsg{Width: w, Height: h})
	return m
}

func TestBranchChanges(t *testing.T) {
	dir := branchWorkspace(t)
	m := start(t, dir, 120, 30)
	if m.git.cmp != nil {
		t.Error("the comparison is built before Branch is asked for")
	}
	drive(t, m, key("b"))
	if m.left != leftChanges || !m.changes.branch {
		t.Fatalf("b did not show Branch changes: left=%v branch=%v", m.left, m.changes.branch)
	}
	s := plainScreen(t, m)
	for _, want := range []string{"vs origin/trunk", "not fetched", "docs/keep.md", "added.md", "docs/guide.md", "new.md", "tracked.log"} {
		if !strings.Contains(s, want) {
			t.Errorf("Branch pane lacks %q:\n%s", want, s)
		}
	}
	// Committed and uncommitted changes in one list, untracked included.
	for _, p := range []string{"docs/keep.md", "added.md", "docs/guide.md", "tracked.log"} {
		if entryIndex(m, sectionBranch, p) < 0 {
			t.Errorf("no Branch entry for %s: %+v", p, m.changes.entries)
		}
	}
	openEntry(t, m, sectionBranch, "docs/keep.md")
	all := strings.Join(m.viewer.rendered().Plain, "\n")
	if tg := m.viewer.target; tg.Mode != ModeDiff || tg.Scope != git.ScopeBranch || !strings.Contains(all, "+# keep on branch") {
		t.Errorf("branch diff %+v:\n%s", tg, all)
	}
	if h := plain(screen(t, m)[0]); !strings.Contains(h, "Diff branch") {
		t.Errorf("header: %q", h)
	}
	openEntry(t, m, sectionBranch, "new.md")
	if all := strings.Join(m.viewer.rendered().Plain, "\n"); !strings.Contains(all, "+new") || m.viewer.target.Scope != git.ScopeUntracked {
		t.Errorf("untracked in Branch:\n%s", all)
	}

	// b again: Working changes, where the committed change is not listed;
	// the cursor stays on the same file.
	openEntry(t, m, sectionBranch, "docs/guide.md")
	drive(t, m, key("b"))
	if m.changes.branch || entryIndex(m, sectionUnstaged, "docs/keep.md") >= 0 {
		t.Errorf("Working view: %+v", m.changes.entries)
	}
	if e := m.changes.entries[m.changes.cursor]; e.path() != "docs/guide.md" {
		t.Errorf("cursor moved to %s", e.path())
	}

	// d in the viewer cycles to the branch diff after the working ones.
	run(t, m, m.open(Target{Path: "docs/guide.md"}, preview.Position{}, nil, false))
	m.focus = paneViewer
	var modes []string
	for range 3 {
		drive(t, m, key("d"))
		if tg := m.viewer.target; tg.Mode == ModeDiff {
			modes = append(modes, tg.Scope.String())
		} else {
			modes = append(modes, "preview")
		}
	}
	if got := strings.Join(modes, ","); got != "unstaged,branch,preview" {
		t.Errorf("d cycle = %s", got)
	}

	for _, sz := range [][2]int{{80, 20}, {50, 12}} {
		drive(t, m, tea.WindowSizeMsg{Width: sz[0], Height: sz[1]})
		drive(t, m, key("b"))
		screen(t, m)
	}
}

// From Files, b opens the Changes pane in the Branch view.
func TestBranchKeyFromFiles(t *testing.T) {
	m := start(t, branchWorkspace(t), 120, 30)
	m.focus = paneViewer
	drive(t, m, key("b"))
	if m.left != leftChanges || !m.changes.branch || m.focus != paneFiles {
		t.Errorf("left=%v branch=%v focus=%v", m.left, m.changes.branch, m.focus)
	}
	drive(t, m, key("c"))
	if m.left != leftFiles {
		t.Error("c did not go back to Files")
	}
}

// A file in the merge base that is untracked now is one entry, compared
// with its base content (DESIGN 7.1).
func TestBranchUntrackedInBase(t *testing.T) {
	dir := branchWorkspace(t)
	if err := os.WriteFile(filepath.Join(dir, "tracked.log"), []byte("recreated\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// No longer ignored, so that it is untracked.
	if err := os.WriteFile(filepath.Join(dir, ".gitignore"), []byte("build/\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	m := start(t, dir, 120, 30)
	drive(t, m, key("b"))
	n := 0
	for _, e := range m.changes.entries {
		if e.path() == "tracked.log" {
			n++
		}
	}
	if n != 1 {
		t.Fatalf("tracked.log listed %d times: %+v", n, m.changes.entries)
	}
	openEntry(t, m, sectionBranch, "tracked.log")
	all := strings.Join(m.viewer.rendered().Plain, "\n")
	for _, want := range []string{"compared with the merge base", "-tracked", "+recreated"} {
		if !strings.Contains(all, want) {
			t.Errorf("missing %q:\n%s", want, all)
		}
	}
	if !strings.Contains(rowOf(t, m, "tracked.log"), "M") {
		t.Errorf("row: %q", rowOf(t, m, "tracked.log"))
	}
}

func rowOf(t *testing.T, m *Model, p string) string {
	t.Helper()
	for _, l := range screen(t, m) {
		if l := plain(l); strings.Contains(l, p) {
			return l
		}
	}
	t.Fatalf("no row for %s", p)
	return ""
}

func TestBranchUnavailableReasons(t *testing.T) {
	dir := gitWorkspace(t) // no origin/HEAD
	m := start(t, dir, 120, 30)
	drive(t, m, key("b"))
	if s := plainScreen(t, m); !strings.Contains(s, "no base branch") || !strings.Contains(s, "--base") {
		t.Errorf("no base:\n%s", s)
	}
	m = startWith(t, dir, Options{Base: "nope"}, 120, 30)
	drive(t, m, key("b"))
	if s := plainScreen(t, m); !strings.Contains(s, `"nope" is not a commit`) {
		t.Errorf("unknown base:\n%s", s)
	}
	// --base names the base; HEAD itself has no branch changes.
	m = startWith(t, dir, Options{Base: "trunk"}, 120, 30)
	drive(t, m, key("b"))
	if s := plainScreen(t, m); !strings.Contains(s, "vs trunk") || entryIndex(m, sectionBranch, "docs/guide.md") < 0 {
		t.Errorf("explicit base:\n%s", s)
	}
	// Without git there are no Changes at all.
	m = start(t, writeFiles(t, map[string]string{"a.md": "a\n"}), 80, 20)
	drive(t, m, key("b"))
	if m.left != leftFiles || !strings.Contains(m.status, "git repository") {
		t.Errorf("non-git: left=%v status=%q", m.left, m.status)
	}
}

// A fetch moves the base ref; the comparison follows without a manual
// reload, and no fetch is ever made by replor.
func TestWatchFollowsBaseRef(t *testing.T) {
	dir := branchWorkspace(t)
	m := startWatching(t, dir)
	drive(t, m, key("b"))
	if entryIndex(m, sectionBranch, "added.md") < 0 {
		t.Fatalf("before: %+v", m.changes.entries)
	}
	for _, root := range []string{dir, m.git.repo.GitDir} {
		settleTree(t, root)
	}
	tick(t, m)
	gitIn(t, dir, "update-ref", "refs/remotes/origin/trunk", "feature")
	tick(t, m)
	if entryIndex(m, sectionBranch, "added.md") >= 0 || entryIndex(m, sectionBranch, "docs/guide.md") < 0 {
		t.Errorf("base move not followed: %+v", m.changes.entries)
	}
}

// A ref the comparison resolved for the first time is watched from the read
// on: a fetch right after it is not absorbed into the baseline.
func TestWatchFollowsNewlyResolvedBaseRef(t *testing.T) {
	dir := branchWorkspace(t)
	m := startWatching(t, dir)
	for _, root := range []string{dir, m.git.repo.GitDir} {
		settleTree(t, root)
	}
	tick(t, m)
	drive(t, m, key("b"))
	gitIn(t, dir, "update-ref", "refs/remotes/origin/trunk", "feature")
	tick(t, m)
	if entryIndex(m, sectionBranch, "added.md") >= 0 {
		t.Errorf("base move not followed: %+v", m.changes.entries)
	}
}

// With --base, origin/HEAD does not matter and is not watched.
func TestBaseRefsFollowTheBase(t *testing.T) {
	m := startWith(t, branchWorkspace(t), Options{Base: "trunk"}, 120, 30)
	drive(t, m, key("b"))
	refs := m.baseRefs()
	if slices.Contains(refs, git.DefaultBaseRef) || !slices.Contains(refs, "refs/heads/trunk") {
		t.Errorf("refs = %v", refs)
	}
}

// The first switch to Branch keeps the cursor on the file once the
// comparison arrives.
func TestFirstBranchSwitchKeepsCursor(t *testing.T) {
	m := start(t, branchWorkspace(t), 120, 30)
	drive(t, m, key("c"))
	m.changes.setCursor(entryIndex(m, sectionUnstaged, "docs/guide.md"))
	drive(t, m, key("b"))
	if e := m.changes.entries[m.changes.cursor]; e.path() != "docs/guide.md" {
		t.Errorf("cursor on %s", e.path())
	}
}

// The Working snapshot is applied without waiting for the comparison.
func TestSnapshotDoesNotWaitForComparison(t *testing.T) {
	m := start(t, branchWorkspace(t), 120, 30)
	drive(t, m, key("b"))
	before := m.git.cmpRev
	msg := m.loadGit()()
	_, cmd := m.Update(msg)
	if m.git.snap.Revision == before || m.git.cmpRev != before || cmd == nil {
		t.Fatalf("snapshot %d, comparison %d (was %d)", m.git.snap.Revision, m.git.cmpRev, before)
	}
	run(t, m, cmd)
	if m.git.cmpRev != m.git.snap.Revision {
		t.Errorf("comparison %d for snapshot %d", m.git.cmpRev, m.git.snap.Revision)
	}
}

// A file in the base that is an untracked nested repository now.
func TestBranchUntrackedDirectoryInBase(t *testing.T) {
	dir := branchWorkspace(t)
	if err := os.Remove(filepath.Join(dir, "added.md")); err != nil {
		t.Fatal(err)
	}
	gitIn(t, dir, "commit", "-qam", "drop added.md")
	gitIn(t, dir, "update-ref", "refs/remotes/origin/trunk", "HEAD~1")
	gitIn(t, dir, "init", "-q", "added.md")
	m := start(t, dir, 120, 30)
	drive(t, m, key("b"))
	openEntry(t, m, sectionBranch, "added.md")
	if m.viewer.err != nil || !strings.Contains(strings.Join(m.viewer.rendered().Plain, "\n"), "nested repository") {
		t.Errorf("err=%v", m.viewer.err)
	}
}

// git's own error text, not its command line, explains a failure.
func TestBranchErrorShowsGitMessage(t *testing.T) {
	m := start(t, branchWorkspace(t), 120, 30)
	drive(t, m, key("b"))
	m.git.cmp, m.git.cmpErr = nil, &process.Error{Args: []string{"/usr/bin/git", "--no-pager", "merge-base"}, ExitCode: 128, Stderr: "fatal: broken\n"}
	m.rebuildChanges()
	if s := plainScreen(t, m); !strings.Contains(s, "fatal: broken") || strings.Contains(s, "--no-pager") {
		t.Errorf("screen:\n%s", s)
	}
}

// On a very low terminal the cursor row stays visible under the header.
func TestBranchListOnLowTerminal(t *testing.T) {
	m := start(t, branchWorkspace(t), 100, 4)
	drive(t, m, key("b"))
	drive(t, m, key("j"))
	want := m.changes.entries[m.changes.cursor].path()
	if s := plainScreen(t, m); !strings.Contains(s, want) {
		t.Errorf("cursor entry %s not on screen:\n%s", want, s)
	}
}

// The footer names the Changes view and the keys that leave it, and an
// empty Working list says what it lists.
func TestChangesFooterAndEmptyWorking(t *testing.T) {
	dir := branchWorkspace(t)
	gitIn(t, dir, "stash", "-q", "--include-untracked")
	m := start(t, dir, 120, 30)
	drive(t, m, key("b"))
	if f := plain(screen(t, m)[29]); !strings.Contains(f, "Changes: Branch") || !strings.Contains(f, "b Working") || !strings.Contains(f, "c Files") {
		t.Errorf("Branch footer: %q", f)
	}
	drive(t, m, key("b"))
	s := screen(t, m)
	if f := plain(s[29]); !strings.Contains(f, "Changes: Working") || !strings.Contains(f, "b Branch") {
		t.Errorf("Working footer: %q", f)
	}
	if !strings.Contains(plain(s[1]), "No uncommitted changes") {
		t.Errorf("empty Working: %q", plain(s[1]))
	}
	drive(t, m, key("c"))
	if f := plain(screen(t, m)[29]); strings.Contains(f, "Changes:") {
		t.Errorf("Files footer: %q", f)
	}
}
