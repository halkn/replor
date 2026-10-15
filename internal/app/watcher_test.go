package app

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/preview"
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
	if !m.watch.limited() || !strings.Contains(plain(screen(t, m)[0]), "watch partial") {
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

// #1: a watch-triggered refresh does not cancel one already running; it is
// queued and issued once that one completes.
func TestWatchQueuesRefreshesInFlight(t *testing.T) {
	dir := gitWorkspace(t)
	m := startWatching(t, dir)
	inflight := m.loadGit() // a slow status read is running
	ticket := m.latest.seq["git"]
	change(t, filepath.Join(dir, "new.md"), "edited\n")
	tick(t, m)
	if m.latest.seq["git"] != ticket || !m.git.queued {
		t.Fatalf("running read was cancelled (seq %d → %d) or not queued", ticket, m.latest.seq["git"])
	}
	drive(t, m, inflight()) // the running read completes ...
	if m.git.queued || m.latest.seq["git"] == ticket {
		t.Error("... and the queued refresh must follow")
	}
}

// #3: a change while the finder is open does not reset its results.
func TestWatchKeepsOpenFinder(t *testing.T) {
	dir := gitWorkspace(t)
	m := startWatching(t, dir)
	drive(t, m, key("f"))
	typeText(t, m, "md")
	drive(t, m, key("down"))
	cursor, results := m.finder.cursor, len(m.finder.results)
	change(t, filepath.Join(dir, "another.md"), "x\n")
	tick(t, m)
	if m.finder.cursor != cursor || len(m.finder.results) != results || !m.finder.stale {
		t.Errorf("finder reset: cursor %d→%d results %d→%d stale %v", cursor, m.finder.cursor, results, len(m.finder.results), m.finder.stale)
	}
	drive(t, m, key("esc"))
	drive(t, m, key("f"))
	if m.finder.stale || !slices.Contains(m.finder.paths, "another.md") {
		t.Error("reopening the finder did not rescan")
	}
}

// #4, #5: editing a file in place reloads git but does not re-list its
// directory; a re-list keeps expanded subtrees on screen.
func TestWatchRelistsOnlyOnNameChanges(t *testing.T) {
	dir := gitWorkspace(t)
	m := startWatching(t, dir)
	m.tree.setCursor(m.tree.indexOf("docs"))
	drive(t, m, key("l"))
	tick(t, m)
	m.tree.setCursor(m.tree.indexOf("docs/guide.md"))

	change(t, filepath.Join(dir, "tracked.log"), "edited in place\n")
	_, cmd := m.Update(watchScan(t, m))
	if m.tree.rootLoading {
		t.Error("in-place edit re-lists the root")
	}
	run(t, m, cmd)

	// A new name in the root re-lists it; docs/ stays expanded meanwhile.
	change(t, filepath.Join(dir, "fresh.md"), "x\n")
	msg := watchScan(t, m)
	_, cmd = m.Update(msg)
	listing := runOne(t, cmd, func(msg tea.Msg) bool { d, ok := msg.(dirListedMsg); return ok && d.dir == "." })
	drive(t, m, listing)
	if m.tree.indexOf("docs/guide.md") < 0 || m.tree.rows[m.tree.cursor].node.Rel != "docs/guide.md" {
		t.Errorf("subtree dropped or cursor moved on re-list: cursor %q", m.tree.rows[m.tree.cursor].node.Rel)
	}
}

// #6: a change between loading a file and the next scan is not lost.
func TestWatchBaselineIsTakenAtLoad(t *testing.T) {
	dir := gitWorkspace(t)
	m := startWatching(t, dir)
	run(t, m, m.open(Target{Path: "docs/keep.md"}, preview.Position{}, nil, true))
	change(t, filepath.Join(dir, "docs", "keep.md"), "# changed right after the load\n")
	tick(t, m)
	if !onScreen(m, "changed right after the load") {
		t.Error("change after the load was missed")
	}
}

// #8: README.md created later does not take over the viewer.
func TestWatchDoesNotOpenLateReadme(t *testing.T) {
	dir := writeFiles(t, map[string]string{"a.md": "# a\n"})
	m := startWatching(t, dir)
	change(t, filepath.Join(dir, "README.md"), "# late\n")
	tick(t, m)
	if m.viewer.target != nil {
		t.Errorf("late README opened: %+v", m.viewer.target)
	}
}

// #11: the partial notice comes after the file name and tabs.
func TestWatchPartialNoticeLast(t *testing.T) {
	old := watchMaxEntries
	watchMaxEntries = 1 // docs/ holds two files
	t.Cleanup(func() { watchMaxEntries = old })
	dir := gitWorkspace(t)
	m := startWatching(t, filepath.Join(dir, "docs", "keep.md"))
	h := plain(screen(t, m)[0])
	if i, j := strings.Index(h, "keep.md"), strings.Index(h, "watch partial"); i < 0 || j < 0 || j < i {
		t.Errorf("header %q", h)
	}
}

// #12: an edit above the reading position keeps the reader on the same
// text and the same selected link.
func TestWatchReloadFollowsContentAndLink(t *testing.T) {
	var body strings.Builder
	for i := range 30 {
		fmt.Fprintf(&body, "Paragraph %d with [link %d](#p%d).\n\n", i, i, i)
	}
	dir := gitWorkspace(t)
	change(t, filepath.Join(dir, "doc.md"), body.String())
	m := startWatching(t, filepath.Join(dir, "doc.md"))
	m.focus = paneViewer
	for range 20 {
		drive(t, m, key(">"))
	}
	top, link := topLine(m), m.viewer.doc().Links[m.viewer.linkIdx].Dest
	change(t, filepath.Join(dir, "doc.md"), "Inserted paragraph with [new](#new).\n\n"+body.String())
	tick(t, m)
	if topLine(m) != top {
		t.Errorf("top %q, was %q", topLine(m), top)
	}
	if got := m.viewer.doc().Links[m.viewer.linkIdx].Dest; got != link {
		t.Errorf("selected link %q, was %q", got, link)
	}
}

// #13: a renamed file tells where it went.
func TestWatchRenamedFile(t *testing.T) {
	dir := gitWorkspace(t)
	m := startWatching(t, filepath.Join(dir, "docs", "keep.md"))
	gitIn(t, dir, "mv", "docs/keep.md", "docs/kept.md")
	tick(t, m)
	if s := plainScreen(t, m); !strings.Contains(s, "moved to docs/kept.md") && !strings.Contains(s, "moved to kept.md") {
		t.Errorf("screen:\n%s", s)
	}
}

// Moving a branch without touching the index (reset --soft) is seen through
// the ref in the common directory of a linked worktree.
func TestWatchWorktreeRefOnlyChange(t *testing.T) {
	dir := gitWorkspace(t)
	wt := filepath.Join(t.TempDir(), "wt")
	gitIn(t, dir, "worktree", "add", "-q", "-b", "side", wt)
	change(t, filepath.Join(wt, "w.md"), "w\n")
	gitIn(t, wt, "add", "w.md")
	gitIn(t, wt, "-c", "user.email=t@example.com", "-c", "user.name=t", "commit", "-qm", "w")
	m := startWatching(t, wt)
	// Date everything back so that recently written files are not "racy"
	// (which would make any scan report a change); only the ref move must.
	for _, root := range []string{wt, m.git.repo.GitDir, m.git.repo.CommonDir} {
		settleTree(t, root)
	}
	tick(t, m) // baseline with settled times
	gitIn(t, wt, "reset", "-q", "--soft", "HEAD~1")
	tick(t, m)
	if m.git.markOf("w.md") != 'A' {
		t.Errorf("ref move not seen: %v", m.git.marks)
	}
}

// A note from the loader is not repeated over another status on reload.
func TestWatchReloadKeepsStatus(t *testing.T) {
	dir := gitWorkspace(t)
	change(t, filepath.Join(dir, "big.md"), "# big\n\n"+strings.Repeat("word ", preview.MaxPreview/5+10))
	m := startWatching(t, filepath.Join(dir, "big.md"))
	m.status = "something else"
	change(t, filepath.Join(dir, "big.md"), "# big2\n\n"+strings.Repeat("word ", preview.MaxPreview/5+10))
	tick(t, m)
	if m.status != "something else" {
		t.Errorf("status %q", m.status)
	}
}

// watchScan performs one scan and returns its result without applying it.
func watchScan(t *testing.T, m *Model) tea.Msg {
	t.Helper()
	_, cmd := m.Update(watchTickMsg{})
	return cmd()
}

// runOne runs cmd and returns the first message matching want, without
// applying the others.
func runOne(t *testing.T, cmd tea.Cmd, want func(tea.Msg) bool) tea.Msg {
	t.Helper()
	var found tea.Msg
	var walk func(tea.Cmd)
	walk = func(c tea.Cmd) {
		if c == nil || found != nil {
			return
		}
		msg := c()
		if v := reflect.ValueOf(msg); v.Kind() == reflect.Slice {
			for i := range v.Len() {
				if cc, ok := v.Index(i).Interface().(tea.Cmd); ok {
					walk(cc)
				}
			}
			return
		}
		if want(msg) {
			found = msg
		}
	}
	walk(cmd)
	if found == nil {
		t.Fatal("expected message not produced")
	}
	return found
}

func settleTree(t *testing.T, root string) {
	t.Helper()
	old := time.Now().Add(-time.Minute)
	err := filepath.WalkDir(root, func(p string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		return os.Chtimes(p, old, old)
	})
	if err != nil {
		t.Fatal(err)
	}
}

// With the reftable backend there is no refs/heads file; tables.list moves.
func TestWatchReftableRefOnlyChange(t *testing.T) {
	hermeticGit(t)
	dir := writeFiles(t, map[string]string{"a.md": "a\n"})
	if err := exec.Command("git", "-C", dir, "init", "-q", "-b", "trunk", "--ref-format=reftable").Run(); err != nil {
		t.Skip("git without reftable support:", err)
	}
	gitIn(t, dir, "config", "user.email", "t@example.com")
	gitIn(t, dir, "config", "user.name", "t")
	gitIn(t, dir, "add", ".")
	gitIn(t, dir, "commit", "-qm", "1")
	change(t, filepath.Join(dir, "b.md"), "b\n")
	gitIn(t, dir, "add", "b.md")
	gitIn(t, dir, "commit", "-qm", "2")
	m := startWatching(t, dir)
	settleTree(t, dir)
	tick(t, m)
	gitIn(t, dir, "reset", "-q", "--soft", "HEAD~1")
	tick(t, m)
	if m.git.markOf("b.md") != 'A' {
		t.Errorf("ref move not seen: %v", m.git.marks)
	}
}

// #1 for the viewer: a load in progress (e.g. the second pass of a large
// document) is not cancelled by a watch event; the reload follows it.
func TestWatchQueuesViewerReload(t *testing.T) {
	dir := gitWorkspace(t)
	m := startWatching(t, filepath.Join(dir, "docs", "keep.md"))
	inflight := m.load()
	ticket := m.latest.seq["preview"]
	change(t, filepath.Join(dir, "docs", "keep.md"), "# second version\n")
	tick(t, m)
	if m.latest.seq["preview"] != ticket || !m.viewer.dirty {
		t.Fatal("running load was cancelled or the reload not queued")
	}
	drive(t, m, inflight())
	if !onScreen(m, "second version") {
		t.Error("queued reload did not run")
	}
}
