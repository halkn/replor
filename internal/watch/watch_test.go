package watch

import (
	"os"
	"path/filepath"
	"slices"
	"testing"
	"time"

	"github.com/halkn/replor/internal/workspace"
)

func setup(t *testing.T) (string, *workspace.Workspace) {
	t.Helper()
	dir, _ := filepath.EvalSymlinks(t.TempDir())
	for name, c := range map[string]string{"a.md": "a", "docs/b.md": "b", "docs/c.md": "c"} {
		p := filepath.Join(dir, name)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(c), 0o644); err != nil {
			t.Fatal(err)
		}
		settle(t, p)
	}
	settle(t, filepath.Join(dir, "docs"))
	settle(t, dir)
	ws, err := workspace.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ws.Close() })
	return dir, ws
}

// settle dates a path back so that it is not "racy" (recently written).
func settle(t *testing.T, p string) {
	t.Helper()
	old := time.Now().Add(-time.Minute)
	if err := os.Chtimes(p, old, old); err != nil {
		t.Fatal(err)
	}
}

func touch(t *testing.T, p, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	// Make the change visible even on coarse mtime file systems.
	later := time.Now().Add(2 * time.Second)
	if err := os.Chtimes(p, later, later); err != nil {
		t.Fatal(err)
	}
}

func TestScanDetectsChanges(t *testing.T) {
	dir, ws := setup(t)
	gitFile := filepath.Join(t.TempDir(), "index")
	touch(t, gitFile, "1")
	set := Set{Files: []string{"a.md"}, Dirs: []string{".", "docs"}, Git: []string{gitFile}}

	first := Scan(ws, set, 100)
	if ch := Compare(first, Scan(ws, set, 100)); !ch.Empty() {
		t.Fatalf("no change expected: %+v", ch)
	}

	touch(t, filepath.Join(dir, "a.md"), "edited")
	touch(t, filepath.Join(dir, "docs", "new.md"), "n")
	touch(t, gitFile, "2")
	ch := Compare(first, Scan(ws, set, 100))
	if !slices.Equal(ch.Files, []string{"a.md"}) || !slices.Equal(ch.Listed, []string{"docs"}) || !slices.Equal(ch.Modified, []string{"."}) || !ch.Git {
		t.Errorf("changes = %+v", ch)
	}

	// An in-place edit of an entry does not change the names in its
	// directory; it is reported separately (it may change the git status).
	settle(t, filepath.Join(dir, "docs")) // just written, so racy
	second := Scan(ws, set, 100)
	touch(t, filepath.Join(dir, "docs", "b.md"), "bb")
	if ch := Compare(second, Scan(ws, set, 100)); !slices.Equal(ch.Modified, []string{"docs"}) || len(ch.Listed) != 0 || len(ch.Files) != 0 {
		t.Errorf("entry edit = %+v", ch)
	}

	// Deleting the watched file is a change, too.
	third := Scan(ws, set, 100)
	if err := os.Remove(filepath.Join(dir, "a.md")); err != nil {
		t.Fatal(err)
	}
	if ch := Compare(third, Scan(ws, set, 100)); !slices.Equal(ch.Files, []string{"a.md"}) {
		t.Errorf("delete = %+v", ch)
	}
}

func TestScanLimit(t *testing.T) {
	_, ws := setup(t)
	s := Scan(ws, Set{Dirs: []string{".", "docs"}}, 2)
	if !s.Limited {
		t.Error("entry limit not reported")
	}
	if s := Scan(ws, Set{Dirs: []string{".", "docs"}}, 100); s.Limited {
		t.Error("limit reported without reaching it")
	}
}

// A changed watch set is not a change of the files themselves.
func TestCompareIgnoresKeysNotInBoth(t *testing.T) {
	_, ws := setup(t)
	old := Scan(ws, Set{Files: []string{"a.md"}}, 100)
	cur := Scan(ws, Set{Files: []string{"a.md", "docs/b.md"}, Dirs: []string{"docs"}}, 100)
	if ch := Compare(old, cur); !ch.Empty() {
		t.Errorf("changes = %+v", ch)
	}
}

// The tree hides .git, and every git command that writes the index changes
// its mtime; it must not count as a change of the directory.
func TestScanSkipsGitDirectory(t *testing.T) {
	dir, ws := setup(t)
	touch(t, filepath.Join(dir, ".git", "index"), "1")
	set := Set{Dirs: []string{"."}}
	first := Scan(ws, set, 100)
	touch(t, filepath.Join(dir, ".git", "index.lock"), "x")
	if ch := Compare(first, Scan(ws, set, 100)); !ch.Empty() {
		t.Errorf("changes = %+v", ch)
	}
}

// A file written within the last moments may be written again within the
// same mtime tick with the same size; it is reported until it settles.
func TestRacyEntriesAreRechecked(t *testing.T) {
	dir, ws := setup(t)
	p := filepath.Join(dir, "a.md")
	if err := os.WriteFile(p, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	set := Set{Files: []string{"a.md"}}
	first := Scan(ws, set, 100)
	if ch := Compare(first, Scan(ws, set, 100)); len(ch.Files) != 1 {
		t.Errorf("recent file not rechecked: %+v", ch)
	}
	old := time.Now().Add(-time.Minute)
	if err := os.Chtimes(p, old, old); err != nil {
		t.Fatal(err)
	}
	settled := Scan(ws, set, 100)
	if ch := Compare(settled, Scan(ws, set, 100)); !ch.Empty() {
		t.Errorf("settled file reported: %+v", ch)
	}
}

// A path added to the watch set is compared with what was seen when it was
// read, not with the next scan, so changes in between are not lost.
func TestMergeSeedsTheBaseline(t *testing.T) {
	dir, ws := setup(t)
	last := Scan(ws, Set{}, 100)
	last.Merge(Scan(ws, Set{Files: []string{"a.md"}, Dirs: []string{"docs"}}, 100)) // when loaded
	touch(t, filepath.Join(dir, "a.md"), "changed after the load")
	touch(t, filepath.Join(dir, "docs", "late.md"), "x")
	ch := Compare(last, Scan(ws, Set{Files: []string{"a.md"}, Dirs: []string{"docs"}}, 100))
	if !slices.Equal(ch.Files, []string{"a.md"}) || !slices.Equal(ch.Listed, []string{"docs"}) {
		t.Errorf("changes = %+v", ch)
	}
}

// Once the entry budget is spent, later directories are not read at all.
func TestScanStopsReadingAtTheLimit(t *testing.T) {
	_, ws := setup(t)
	s := Scan(ws, Set{Dirs: []string{"docs", "."}}, 2)
	if !s.Limited || len(s.dirs["."].entries) != 0 || s.dirs["."].complete {
		t.Errorf("root read after the budget: %+v", s.dirs["."])
	}
}
