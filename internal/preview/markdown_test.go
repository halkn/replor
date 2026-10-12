package preview

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/charmbracelet/x/ansi"

	"github.com/halkn/replor/internal/document"
)

func gfmDoc(t testing.TB) *document.Doc {
	t.Helper()
	b, err := os.ReadFile(filepath.Join("..", "..", "testdata", "markdown", "gfm.md"))
	if err != nil {
		t.Fatal(err)
	}
	return document.Parse(string(b))
}

func render(t testing.TB, d *document.Doc, width int) *Rendered {
	t.Helper()
	r, err := RenderMarkdown(d, width, nil)
	if err != nil {
		t.Fatal(err)
	}
	return r
}

func TestHeadingLinesAtEveryWidth(t *testing.T) {
	d := gfmDoc(t)
	for _, w := range []int{30, 60, 100, 160} {
		r := render(t, d, w)
		for i := 1; i < len(r.BlockLine); i++ {
			if r.BlockLine[i] < r.BlockLine[i-1] {
				t.Fatalf("w=%d: BlockLine not monotonic at %d", w, i)
			}
		}
		for _, h := range d.Headings {
			line := r.LineOf(Position{Block: h.Block})
			if !strings.Contains(r.Plain[line], h.Text) {
				t.Errorf("w=%d: heading %q expected at line %d, got %q", w, h.Text, line, r.Plain[line])
			}
		}
	}
}

// The P0-03 scenario: select a link, follow it, go back, jump via TOC,
// search, resize — the reading position must stay meaningful throughout.
func TestNavigationScenario(t *testing.T) {
	d := gfmDoc(t)
	r := render(t, d, 80)

	// Reader is two lines into the paragraph under 日本語長文.
	cur := Position{Block: mustHeading(t, d, "日本語長文").Block + 1, Offset: 2}
	want := r.Plain[r.LineOf(cur)]

	// Follow the in-document link "#data-flow--api-v2".
	var link document.DocLink
	for _, l := range d.Links {
		if l.Dest == "#data-flow--api-v2" {
			link = l
		}
	}
	h, ok := d.HeadingByID(strings.TrimPrefix(link.Dest, "#"))
	if !ok {
		t.Fatal("anchor not found")
	}
	saved := cur
	cur = Position{Block: h.Block}
	if got := r.Plain[r.LineOf(cur)]; !strings.Contains(got, "Data Flow") {
		t.Fatalf("after link: %q", got)
	}

	// Back restores the previous position exactly.
	cur = saved
	if got := r.Plain[r.LineOf(cur)]; got != want || got == "" {
		t.Fatalf("after back: %q", got)
	}

	// TOC jump to the second duplicate heading.
	dup := mustHeadingID(t, d, "重複見出し-1")
	cur = Position{Block: dup.Block}
	first := mustHeadingID(t, d, "重複見出し")
	if r.LineOf(cur) <= r.LineOf(Position{Block: first.Block}) {
		t.Error("duplicate heading jumped to the first occurrence")
	}

	// Search, then resize: the hit's block stays in view.
	hits := r.Find("書生")
	if len(hits) == 0 {
		t.Fatal("no search hit")
	}
	cur = r.PositionAt(hits[0].Line)
	for _, w := range []int{40, 120} {
		r2 := render(t, d, w)
		line := r2.LineOf(cur)
		if r2.BlockAt(line) != cur.Block {
			t.Errorf("w=%d: resize moved to block %d, want %d", w, r2.BlockAt(line), cur.Block)
		}
		found := false
		for _, m := range r2.Find("書生") {
			if r2.BlockAt(m.Line) == cur.Block {
				found = true
			}
		}
		if !found {
			t.Errorf("w=%d: hit not in restored block", w)
		}
	}
}

func TestRenderNeutralizesInjectedEscapes(t *testing.T) {
	d := gfmDoc(t)
	r := render(t, d, 80)
	for i, l := range r.Lines {
		// Only SGR and OSC 8 produced by glamour itself may remain; the
		// fixture's injected clear-screen and evil hyperlink must not.
		if strings.Contains(l, "\x1b[2J") || strings.Contains(l, "evil.example") && strings.Contains(l, "\x1b]8") {
			t.Errorf("line %d carries injected escape: %q", i, l)
		}
	}
}

func TestOnlyHTTPHyperlinksSurvive(t *testing.T) {
	d := document.Parse("[a](javascript:alert(1)) [b](file:///etc/passwd) [c](./x.md) [d](https://example.com)\n")
	r := render(t, d, 80)
	all := strings.Join(r.Lines, "\n")
	for _, m := range osc8.FindAllStringSubmatch(all, -1) {
		if m[2] != "" && !strings.HasPrefix(m[2], "https://") {
			t.Errorf("hyperlink to %q survived", m[2])
		}
	}
	if !strings.Contains(all, "https://example.com") {
		t.Error("https hyperlink removed")
	}
}

func TestFindColumns(t *testing.T) {
	r := &Rendered{Plain: []string{"日本語 abc ABC"}}
	got := r.Find("abc")
	if len(got) != 2 || got[0].Col != ansi.StringWidth("日本語 ") || got[1].Col != ansi.StringWidth("日本語 abc ") {
		t.Errorf("Find = %+v", got)
	}
}

func mustHeading(t *testing.T, d *document.Doc, text string) document.Heading {
	t.Helper()
	for _, h := range d.Headings {
		if h.Text == text {
			return h
		}
	}
	t.Fatalf("no heading %q", text)
	return document.Heading{}
}

func mustHeadingID(t *testing.T, d *document.Doc, id string) document.Heading {
	t.Helper()
	h, ok := d.HeadingByID(id)
	if !ok {
		t.Fatalf("no heading id %q", id)
	}
	return h
}

func BenchmarkRenderGFM(b *testing.B) {
	d := gfmDoc(b)
	for b.Loop() {
		render(b, d, 100)
	}
}

func syntheticMarkdown(size int) string {
	var b strings.Builder
	for i := 0; b.Len() < size; i++ {
		fmt.Fprintf(&b, "## 節 %d Section\n\n", i)
		b.WriteString(strings.Repeat("日本語と English が混在する段落です。リンク [x](./a.md) を含む。", 4) + "\n\n")
		if i%5 == 0 {
			b.WriteString("| a | b | c |\n|---|---|---|\n| 1 | 二 | three |\n| 4 | 五 | six |\n\n")
		}
		if i%7 == 0 {
			b.WriteString("```go\nfunc f() int { return 42 }\n```\n\n")
		}
	}
	return b.String()
}

// TestLoadRender records parse/render cost for P0-09. Run with
// REPLOR_LOAD=1 go test ./internal/preview -run Load -v
func TestLoadRender(t *testing.T) {
	if os.Getenv("REPLOR_LOAD") == "" {
		t.Skip("set REPLOR_LOAD=1")
	}
	for _, size := range []int{100 << 10, 1 << 20, 4 << 20} {
		src := syntheticMarkdown(size)
		start := time.Now()
		d := document.Parse(src)
		parsed := time.Since(start)
		r := render(t, d, 100)
		total := time.Since(start)
		bytes := 0
		for i := range r.Lines {
			bytes += len(r.Lines[i]) + len(r.Plain[i])
		}
		t.Logf("size=%dKiB blocks=%d lines=%d parse=%v parse+render=%v renderedBytes=%.1fMiB", len(src)>>10, len(d.Blocks), len(r.Lines), parsed.Round(time.Millisecond), total.Round(time.Millisecond), float64(bytes)/(1<<20))
	}
}
