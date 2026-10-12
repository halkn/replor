package document

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSlug(t *testing.T) {
	cases := map[string]string{
		"GFM 表示評価":             "gfm-表示評価",
		"Data Flow & API (v2)": "data-flow--api-v2",
		"Hello, World!":        "hello-world",
		"snake_case-name":      "snake_case-name",
		"Ｆｕｌｌ　ｗｉｄｔｈ":           "ｆｕｌｌｗｉｄｔｈ",
		"😀 emoji":              "-emoji",
	}
	for in, want := range cases {
		if got := Slug(in); got != want {
			t.Errorf("Slug(%q) = %q, want %q", in, got, want)
		}
	}
}

func loadGFM(t *testing.T) *Doc {
	t.Helper()
	b, err := os.ReadFile(filepath.Join("..", "..", "testdata", "markdown", "gfm.md"))
	if err != nil {
		t.Fatal(err)
	}
	return Parse(string(b))
}

func TestParseStructure(t *testing.T) {
	d := loadGFM(t)

	var ids []string
	for _, h := range d.Headings {
		ids = append(ids, h.ID)
	}
	want := []string{"gfm-表示評価", "表table", "task-list", "alert", "脚注", "日本語長文", "コード", "重複見出し", "重複見出し-1", "data-flow--api-v2"}
	if strings.Join(ids, ",") != strings.Join(want, ",") {
		t.Errorf("heading ids\n got %v\nwant %v", ids, want)
	}

	// Every internal anchor link in the fixture resolves to a heading.
	for _, l := range d.Links {
		if frag, ok := strings.CutPrefix(l.Dest, "#"); ok {
			if _, found := d.HeadingByID(frag); !found {
				t.Errorf("anchor %q has no heading", l.Dest)
			}
		}
	}

	// Blocks tile the source without gaps or overlap.
	pos := 0
	for i, b := range d.Blocks {
		if b.Start != pos || b.End < b.Start {
			t.Fatalf("block %d = [%d,%d), expected start %d", i, b.Start, b.End, pos)
		}
		pos = b.End
	}
	if pos != len(d.Source) {
		t.Errorf("blocks end at %d of %d", pos, len(d.Source))
	}

	// A heading's block source starts with the heading line itself.
	for _, h := range d.Headings {
		if src := d.BlockSource(h.Block); !strings.HasPrefix(src, strings.Repeat("#", h.Level)+" ") {
			t.Errorf("heading %q block starts with %q", h.Text, src[:min(20, len(src))])
		}
	}

	var goBlock *Block
	for i := range d.Blocks {
		if d.Blocks[i].Lang == "go" {
			goBlock = &d.Blocks[i]
		}
	}
	if goBlock == nil || !strings.Contains(goBlock.Code, "こんにちは") || strings.Contains(goBlock.Code, "```") {
		t.Errorf("fenced block = %+v", goBlock)
	}
	img := Parse("text\n\n![図](img/a.png)\n\n![x](b.png) with text\n")
	if img.Blocks[1].Image != "img/a.png" || img.Blocks[2].Image != "" {
		t.Errorf("image blocks = %+v", img.Blocks)
	}

	if !strings.Contains(d.RefDefs, "[ref]: <https://example.com/ref>") {
		t.Errorf("RefDefs = %q", d.RefDefs)
	}
	if strings.ContainsRune(d.Source, 0x1b) {
		t.Error("ESC survived parsing")
	}
}

// Anchors come from the heading's rendered text, not its Markdown source.
func TestHeadingIDsIgnoreInlineSyntax(t *testing.T) {
	d := Parse("# [Title](https://example.com)\n\n## `code` and *em*\n\n# Title\n")
	var ids []string
	for _, h := range d.Headings {
		ids = append(ids, h.ID)
	}
	if want := "title,code-and-em,title-1"; strings.Join(ids, ",") != want {
		t.Errorf("ids = %v, want %s", ids, want)
	}
}

func TestAlertsAndFootnotes(t *testing.T) {
	d := loadGFM(t)
	var alerts []string
	var notes []Footnote
	for _, b := range d.Blocks {
		if b.Alert != "" {
			alerts = append(alerts, b.Alert)
		}
		notes = append(notes, b.Footnotes...)
	}
	if strings.Join(alerts, ",") != "NOTE,WARNING" {
		t.Errorf("alerts = %v", alerts)
	}
	if len(notes) != 2 || notes[0].Ref != "1" || notes[0].Text != "一つ目の脚注。" || notes[1].Ref != "note" {
		t.Errorf("footnotes = %+v", notes)
	}

	// Only a marker alone on the first quoted line makes an alert.
	for _, src := range []string{"> [!NOTE] inline text\n", "> text\n> [!NOTE]\n", "> [!UNKNOWN]\n> x\n"} {
		if a := Parse(src).Blocks[0].Alert; a != "" {
			t.Errorf("%q: alert %q", src, a)
		}
	}
	if a := Parse("> [!tip]\n> lower case\n").Blocks[0].Alert; a != "TIP" {
		t.Errorf("lower case alert = %q", a)
	}
}
