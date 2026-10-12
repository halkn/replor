package workspace

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"
)

func mkfile(t *testing.T, p, s string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(s), 0o644); err != nil {
		t.Fatal(err)
	}
}

func symlink(t *testing.T, target, link string) {
	t.Helper()
	if err := os.Symlink(target, link); err != nil {
		t.Fatal(err)
	}
}

// layout:
//
//	base/secret.txt
//	base/ws/README.md
//	base/ws/docs/a.md
//	base/ws/in.md       -> docs/a.md          (relative, inside)
//	base/ws/up.md       -> ../secret.txt      (relative, escapes)
//	base/ws/abs-in.md   -> <abs>/ws/docs/a.md (absolute, inside)
//	base/ws/outdir      -> ..                 (directory, escapes)
//	base/ws/docs/back   -> ..                 (directory, inside)
func layout(t *testing.T) (base, ws string) {
	t.Helper()
	base, _ = filepath.EvalSymlinks(t.TempDir())
	ws = filepath.Join(base, "ws")
	mkfile(t, filepath.Join(base, "secret.txt"), "secret")
	mkfile(t, filepath.Join(ws, "README.md"), "# readme")
	mkfile(t, filepath.Join(ws, "docs", "a.md"), "# a")
	symlink(t, "docs/a.md", filepath.Join(ws, "in.md"))
	symlink(t, "../secret.txt", filepath.Join(ws, "up.md"))
	symlink(t, filepath.Join(ws, "docs", "a.md"), filepath.Join(ws, "abs-in.md"))
	symlink(t, "..", filepath.Join(ws, "outdir"))
	symlink(t, "..", filepath.Join(ws, "docs", "back"))
	return base, ws
}

func TestBoundary(t *testing.T) {
	_, ws := layout(t)
	w, err := Open(ws)
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()

	allowed := []string{"README.md", "docs/a.md", "in.md", "docs/back/README.md", "docs/../README.md", "./README.md"}
	for _, p := range allowed {
		if _, err := w.ReadFile(p, 1<<20); err != nil {
			t.Errorf("ReadFile(%q) = %v; want allowed", p, err)
		}
	}
	rejected := []string{"../secret.txt", "up.md", "outdir/secret.txt", "/etc/passwd", "docs/../../secret.txt", "a\\..\\..\\secret.txt", "x\x00y"}
	for _, p := range rejected {
		if _, err := w.ReadFile(p, 1<<20); !errors.Is(err, ErrOutside) {
			t.Errorf("ReadFile(%q) = %v; want ErrOutside", p, err)
		}
	}
	// os.Root refuses absolute link targets even when they point inside.
	if _, err := w.ReadFile("abs-in.md", 1<<20); err == nil {
		t.Error("absolute symlink was followed")
	}
	if _, err := w.ReadDir("outdir"); !errors.Is(err, ErrOutside) {
		t.Errorf("ReadDir(outdir) = %v; want ErrOutside", err)
	}
}

func TestOpenFileSelectsInParent(t *testing.T) {
	_, ws := layout(t)
	w, err := Open(filepath.Join(ws, "docs", "a.md"))
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()
	if w.Root != filepath.Join(ws, "docs") || w.Selected != "a.md" {
		t.Errorf("root=%q selected=%q", w.Root, w.Selected)
	}
	// Starting in docs/ must not expose the parent README.
	if _, err := w.ReadFile("../README.md", 1<<20); !errors.Is(err, ErrOutside) {
		t.Errorf("parent read = %v", err)
	}
}

func TestOpenResolvesSymlinkedRoot(t *testing.T) {
	base, ws := layout(t)
	link := filepath.Join(base, "ws-link")
	symlink(t, ws, link)
	w, err := Open(link)
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()
	if w.Root != ws {
		t.Errorf("root = %q, want %q", w.Root, ws)
	}
}

func TestReadFileLimit(t *testing.T) {
	_, ws := layout(t)
	w, err := Open(ws)
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()
	if _, err := w.ReadFile("README.md", 3); !errors.Is(err, ErrTooLarge) {
		t.Errorf("got %v; want ErrTooLarge", err)
	}
	if _, err := w.ReadFile("docs", 1<<20); err == nil {
		t.Error("directory read as file")
	}
}

func TestListDir(t *testing.T) {
	_, ws := layout(t)
	mkfile(t, filepath.Join(ws, ".git", "HEAD"), "")
	mkfile(t, filepath.Join(ws, "b.txt"), "")
	w, err := Open(ws)
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()

	nodes, truncated, err := w.ListDir(".", 0)
	if err != nil || truncated {
		t.Fatal(err, truncated)
	}
	got := map[string]Node{}
	var order []string
	for _, n := range nodes {
		got[n.Name] = n
		order = append(order, n.Name)
	}
	if want := "docs,README.md,abs-in.md,b.txt,in.md,outdir,up.md"; strings.Join(order, ",") != want {
		t.Errorf("order = %v, want %s", order, want)
	}
	check := func(name string, kind NodeKind, link bool) {
		t.Helper()
		n := got[name]
		if n.Kind != kind || n.Symlink != link {
			t.Errorf("%s = %+v, want kind %v symlink %v", name, n, kind, link)
		}
	}
	check("docs", NodeDir, false)
	check("in.md", NodeFile, true)
	check("up.md", NodeBroken, true)
	check("abs-in.md", NodeBroken, true)
	check("outdir", NodeBroken, true)
	if got["up.md"].Reason == "" || got["docs"].Rel != "docs" {
		t.Errorf("reason/rel: %+v %+v", got["up.md"], got["docs"])
	}

	sub, _, err := w.ListDir("docs", 0)
	if err != nil || len(sub) != 2 || sub[0].Name != "back" || sub[0].Kind != NodeDir || !sub[0].Symlink || sub[1].Rel != "docs/a.md" {
		t.Errorf("docs = %+v %v", sub, err)
	}

	limited, truncated, err := w.ListDir(".", 2)
	if err != nil || !truncated || len(limited) != 2 {
		t.Errorf("limit: %d %v %v", len(limited), truncated, err)
	}
	if _, _, err := w.ListDir("../", 0); !errors.Is(err, ErrOutside) {
		t.Errorf("escape: %v", err)
	}
}

// Opening a FIFO for reading blocks until a writer appears; it must be
// refused before open.
func TestReadFileRefusesFIFO(t *testing.T) {
	dir, _ := filepath.EvalSymlinks(t.TempDir())
	if err := syscall.Mkfifo(filepath.Join(dir, "pipe"), 0o644); err != nil {
		t.Skip("mkfifo:", err)
	}
	w, err := Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()
	done := make(chan error, 1)
	go func() {
		_, err := w.ReadFile("pipe", 1<<20)
		done <- err
	}()
	select {
	case err := <-done:
		if err == nil {
			t.Error("FIFO was read")
		}
	case <-time.After(2 * time.Second):
		t.Fatal("ReadFile blocked on a FIFO")
	}
}
