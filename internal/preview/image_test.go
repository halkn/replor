package preview

import (
	"image"
	"image/color"
	"strings"
	"testing"

	uv "github.com/charmbracelet/ultraviolet"
	"github.com/charmbracelet/x/ansi"
)

func TestDetectImageMode(t *testing.T) {
	cases := []struct {
		env  map[string]string
		want ImageMode
	}{
		{map[string]string{"TERM": "xterm-ghostty", "TERM_PROGRAM": "ghostty"}, ImageKitty},
		{map[string]string{"KITTY_WINDOW_ID": "1"}, ImageKitty},
		{map[string]string{"WT_SESSION": "x", "TERM": "xterm-256color"}, ImageHalfblocks},
		{map[string]string{"TERM_PROGRAM": "ghostty", "TMUX": "/tmp/x"}, ImageHalfblocks},
		{map[string]string{"COLORTERM": "truecolor"}, ImageHalfblocks},
		{map[string]string{"TERM": "dumb"}, ImageOff},
	}
	for _, c := range cases {
		if got := DetectImageMode(func(k string) string { return c.env[k] }); got != c.want {
			t.Errorf("%v: got %v, want %v", c.env, got, c.want)
		}
	}
}

// Placeholder cells must survive Bubble Tea's cell renderer as one cell each,
// with both width methods the renderer may negotiate.
func TestKittyPlaceholdersSurviveCellBuffer(t *testing.T) {
	const cols, rows = 12, 4
	lines := KittyPlaceholders(0x0a0b0c, cols, rows)
	for _, m := range []ansi.Method{ansi.WcWidth, ansi.GraphemeWidth} {
		for r, l := range lines {
			if w := m.StringWidth(l); w != cols {
				t.Errorf("method %v row %d: width %d, want %d", m, r, w, cols)
			}
		}
		buf := uv.NewScreenBuffer(cols+4, rows)
		buf.Method = m
		uv.NewStyledString(strings.Join(lines, "\n")).Draw(buf, uv.Rect(0, 0, cols+4, rows))
		for r := range rows {
			for c := range cols {
				cell := buf.CellAt(c, r)
				want := kittyPlaceholder + string(kittyDiacritics[r]) + string(kittyDiacritics[c])
				if cell == nil || cell.Content != want {
					t.Fatalf("method %v cell(%d,%d) = %+q, want %+q", m, c, r, cell, want)
				}
			}
			if cell := buf.CellAt(cols, r); cell != nil && cell.Content != " " && cell.Content != "" {
				t.Errorf("method %v: spill at (%d,%d): %q", m, cols, r, cell.Content)
			}
		}
	}
}

func TestHalfblocksSize(t *testing.T) {
	img := image.NewRGBA(image.Rect(0, 0, 200, 100))
	for x := range 200 {
		for y := range 100 {
			img.Set(x, y, color.RGBA{uint8(x), uint8(y), 128, 255})
		}
	}
	cols, rows := FitCells(img.Bounds(), 40, 0)
	if cols != 40 || rows != 10 {
		t.Fatalf("FitCells = %d,%d", cols, rows)
	}
	lines := Halfblocks(img, cols, rows)
	if len(lines) != rows {
		t.Fatalf("rows = %d, want %d", len(lines), rows)
	}
	for i, l := range lines {
		if w := ansi.StringWidth(l); w != cols {
			t.Errorf("row %d width %d", i, w)
		}
	}
}

func TestKittyTransmitChunks(t *testing.T) {
	seq := KittyTransmit(make([]byte, 10000), 7, 10, 5)
	if !strings.HasPrefix(seq, "\x1b_Ga=T,f=100,U=1,i=7,c=10,r=5,q=2,m=1;") {
		t.Errorf("header: %q", seq[:60])
	}
	if !strings.HasSuffix(seq, "\x1b\\") || strings.Count(seq, "\x1b_G") < 3 {
		t.Errorf("chunking: %d chunks", strings.Count(seq, "\x1b_G"))
	}
	if !strings.Contains(seq, "\x1b_Gm=0;") {
		t.Error("last chunk must carry m=0")
	}
}
