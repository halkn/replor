package fuzzy

import (
	"context"
	"fmt"
	"slices"
	"testing"
)

func TestMatch(t *testing.T) {
	if _, ok := Match("xyz", "internal/app/model.go"); ok {
		t.Error("non-subsequence matched")
	}
	m, ok := Match("mdl", "internal/app/model.go")
	if !ok || !slices.Equal(m.Positions, []int{13, 15, 17}) {
		t.Errorf("positions = %v", m.Positions)
	}
	if m, ok := Match("", "a.go"); !ok || m.Positions != nil {
		t.Error("empty query should match without positions")
	}
	if _, ok := Match("MODEL", "internal/app/model.go"); !ok {
		t.Error("matching must ignore case")
	}
	// Multi-byte text: positions are byte offsets of rune starts.
	if m, ok := Match("設計", "docs/設計書.md"); !ok || !slices.Equal(m.Positions, []int{5, 8}) {
		t.Errorf("japanese positions = %v %v", m.Positions, ok)
	}
}

func TestRankPrefersBasenameAndContiguity(t *testing.T) {
	paths := []string{
		"internal/app/viewer.go",
		"docs/view/README.md",
		"vendor/x/i/e/w/thing.go",
		"internal/preview/view.go",
	}
	got := Rank(context.Background(), "view", paths, 10)
	var order []string
	for _, r := range got {
		order = append(order, paths[r.Index])
	}
	want := []string{"internal/preview/view.go", "internal/app/viewer.go", "docs/view/README.md", "vendor/x/i/e/w/thing.go"}
	if !slices.Equal(order, want) {
		t.Errorf("order\n got %v\nwant %v", order, want)
	}
	if got := Rank(context.Background(), "view", paths, 2); len(got) != 2 {
		t.Errorf("limit: %d", len(got))
	}
}

func TestRankEmptyQueryKeepsOrder(t *testing.T) {
	paths := []string{"b", "a", "c"}
	got := Rank(context.Background(), "", paths, 2)
	if len(got) != 2 || got[0].Index != 0 || got[1].Index != 1 {
		t.Errorf("got %+v", got)
	}
}

func TestRankStopsOnCancel(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	paths := make([]string, 10000)
	for i := range paths {
		paths[i] = fmt.Sprintf("dir/file%d.go", i)
	}
	if got := Rank(ctx, "file", paths, 10); got != nil {
		t.Errorf("cancelled rank returned %d results", len(got))
	}
}

func BenchmarkRank200k(b *testing.B) {
	paths := make([]string, 200_000)
	for i := range paths {
		paths[i] = fmt.Sprintf("src/module%03d/pkg%02d/file_%05d.go", i%300, i%40, i)
	}
	for b.Loop() {
		Rank(context.Background(), "mod7pkfil", paths, 200)
	}
}
