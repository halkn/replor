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
	}
	ws, err := workspace.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ws.Close() })
	return dir, ws
}

func touch(t *testing.T, p, content string) {
	t.Helper()
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
	if !slices.Equal(ch.Files, []string{"a.md"}) || !slices.Contains(ch.Dirs, "docs") || !slices.Contains(ch.Dirs, ".") || !ch.Git {
		t.Errorf("changes = %+v", ch)
	}

	// In-place edits of an entry also count as a change of its directory,
	// since they can change the git status shown in the tree.
	second := Scan(ws, set, 100)
	touch(t, filepath.Join(dir, "docs", "b.md"), "bb")
	if ch := Compare(second, Scan(ws, set, 100)); !slices.Equal(ch.Dirs, []string{"docs"}) || len(ch.Files) != 0 {
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
