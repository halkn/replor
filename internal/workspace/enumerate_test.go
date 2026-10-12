package workspace

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strconv"
	"testing"
	"time"
)

func tree(t testing.TB, root string, dirs, filesPerDir int) {
	t.Helper()
	for d := range dirs {
		dir := filepath.Join(root, fmt.Sprintf("d%03d", d%100), fmt.Sprintf("s%04d", d))
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatal(err)
		}
		for f := range filesPerDir {
			if err := os.WriteFile(filepath.Join(dir, "f"+strconv.Itoa(f)+".go"), nil, 0o644); err != nil {
				t.Fatal(err)
			}
		}
	}
}

func collect(t *testing.T, w *Workspace, ctx context.Context, lim Limits) ([]string, string) {
	t.Helper()
	var got []string
	reason, err := w.Enumerate(ctx, lim, func(p string) bool { return filepath.Base(p) == ".git" }, func(b []string) { got = append(got, b...) })
	if err != nil {
		t.Fatal(err)
	}
	return got, reason
}

func TestEnumerate(t *testing.T) {
	root, _ := filepath.EvalSymlinks(t.TempDir())
	mkfile(t, filepath.Join(root, "a.md"), "")
	mkfile(t, filepath.Join(root, "x", "y", "z", "deep.md"), "")
	mkfile(t, filepath.Join(root, ".git", "HEAD"), "")
	symlink(t, ".", filepath.Join(root, "loop"))
	w, err := Open(root)
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()

	got, reason := collect(t, w, context.Background(), Limits{})
	slices.Sort(got)
	if want := []string{"a.md", "loop", "x/y/z/deep.md"}; !slices.Equal(got, want) || reason != Complete {
		t.Errorf("got %v (%q), want %v", got, reason, want)
	}

	got, reason = collect(t, w, context.Background(), Limits{MaxDepth: 2})
	if slices.Contains(got, "x/y/z/deep.md") || reason != LimitedDepth {
		t.Errorf("depth: %v %q", got, reason)
	}

	got, reason = collect(t, w, context.Background(), Limits{MaxEntries: 1})
	if len(got) != 1 || reason != LimitedEntries {
		t.Errorf("entries: %v %q", got, reason)
	}
}

func TestEnumerateCancel(t *testing.T) {
	root, _ := filepath.EvalSymlinks(t.TempDir())
	tree(t, root, 50, 40)
	w, err := Open(root)
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()
	ctx, cancel := context.WithCancel(context.Background())
	n := 0
	_, err = w.Enumerate(ctx, Limits{}, nil, func(b []string) {
		n += len(b)
		cancel()
	})
	if err != context.Canceled || n >= 2000 {
		t.Errorf("err=%v n=%d", err, n)
	}
}

// TestLoadEnumerate records enumeration cost for P0-09. Run with
// REPLOR_LOAD=1 go test ./internal/workspace -run Load -v
func TestLoadEnumerate(t *testing.T) {
	if os.Getenv("REPLOR_LOAD") == "" {
		t.Skip("set REPLOR_LOAD=1")
	}
	for _, n := range []int{10_000, 100_000, 300_000} {
		root, _ := filepath.EvalSymlinks(t.TempDir())
		tree(t, root, n/100, 100)
		w, err := Open(root)
		if err != nil {
			t.Fatal(err)
		}
		runtime.GC()
		var before, after runtime.MemStats
		runtime.ReadMemStats(&before)
		start := time.Now()
		var first time.Duration
		var paths []string
		_, err = w.Enumerate(context.Background(), Limits{}, nil, func(b []string) {
			if first == 0 {
				first = time.Since(start)
			}
			paths = append(paths, b...)
		})
		el := time.Since(start)
		runtime.ReadMemStats(&after)
		w.Close()
		if err != nil {
			t.Fatal(err)
		}
		t.Logf("paths=%d total=%v firstBatch=%v heapDelta=%.1fMiB", len(paths), el.Round(time.Millisecond), first.Round(time.Microsecond), float64(after.HeapAlloc-before.HeapAlloc)/(1<<20))
	}
}
