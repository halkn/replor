package preview

import (
	"bytes"
	"context"
	"errors"
	"image"
	"image/color"
	"image/png"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/charmbracelet/x/ansi"

	"github.com/halkn/replor/internal/workspace"
)

func tempWS(t *testing.T, files map[string]string) *workspace.Workspace {
	t.Helper()
	dir := t.TempDir()
	for name, content := range files {
		p := filepath.Join(dir, name)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	ws, err := workspace.Open(dir)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ws.Close() })
	return ws
}

func load(t *testing.T, ws *workspace.Workspace, path string, source bool) (Result, error) {
	t.Helper()
	return Load(context.Background(), Request{WS: ws, Path: path, Source: source, Width: 80})
}

func TestLoadKinds(t *testing.T) {
	ws := tempWS(t, map[string]string{
		"README.md":  "# Title\n\nbody\n",
		"main.go":    "package main\n\nfunc main() {\n\tprintln(\"hi\")\n}\n",
		"empty.txt":  "",
		"bin.dat":    "abc\x00def",
		"notes.txt":  "plain\x1b[2Jtext\n",
		"big.md":     "# big\n\n" + strings.Repeat("x", MaxPreview),
		"huge.txt":   strings.Repeat("y", MaxSource+1),
		"image.png":  string(testPNG(t)),
		"broken.png": "\x89PNG\r\n\x1a\nnot really",
	})

	r, err := load(t, ws, "README.md", false)
	if err != nil || r.Kind != KindMarkdown || r.Doc == nil || !strings.Contains(strings.Join(r.Rendered.Plain, "\n"), "Title") {
		t.Errorf("README.md: %+v %v", r.Kind, err)
	}

	r, err = load(t, ws, "README.md", true)
	if err != nil || r.Kind != KindMarkdown || r.Doc != nil || !strings.Contains(r.Rendered.Plain[0], "# Title") {
		t.Errorf("README.md source: %+v %v", r.Kind, err)
	}

	r, err = load(t, ws, "main.go", false)
	if err != nil || r.Kind != KindText || len(r.Rendered.Lines) != 5 {
		t.Fatalf("main.go: %+v %v", r, err)
	}
	if !strings.Contains(r.Rendered.Lines[0], "\x1b[") {
		t.Error("main.go is not highlighted")
	}
	if got := r.Rendered.Plain[3]; !strings.Contains(got, "4 │ ") || !strings.Contains(got, "    println") {
		t.Errorf("line 4 = %q (want number gutter and expanded tab)", got)
	}

	if r, err = load(t, ws, "empty.txt", false); err != nil || r.Kind != KindEmpty {
		t.Errorf("empty: %+v %v", r.Kind, err)
	}
	if r, err = load(t, ws, "bin.dat", false); err != nil || r.Kind != KindBinary {
		t.Errorf("binary: %+v %v", r.Kind, err)
	}

	r, err = load(t, ws, "notes.txt", false)
	if err != nil || strings.ContainsRune(strings.Join(r.Rendered.Lines, ""), 0x1b) && strings.Contains(strings.Join(r.Rendered.Lines, ""), "\x1b[2J") {
		t.Errorf("control characters reached the output: %q", r.Rendered.Lines)
	}

	r, err = load(t, ws, "big.md", false)
	if err != nil || r.Kind != KindMarkdown || r.Doc != nil || r.Note == "" {
		t.Errorf("big.md should fall back to source with a note: %+v %v", r.Note, err)
	}

	if _, err = load(t, ws, "huge.txt", false); !errors.Is(err, workspace.ErrTooLarge) {
		t.Errorf("huge: %v", err)
	}

	r, err = load(t, ws, "image.png", false)
	if err != nil || r.Kind != KindImage || len(r.Rendered.Lines) == 0 {
		t.Errorf("image: %+v %v", r.Kind, err)
	}
	if r, err = load(t, ws, "broken.png", false); err != nil || r.Kind != KindBinary {
		t.Errorf("broken image: %+v %v", r.Kind, err)
	}
}

func TestLoadPartialMarkdown(t *testing.T) {
	var src strings.Builder
	for i := range 100 {
		src.WriteString("## Section " + string(rune('A'+i%26)) + "\n\nparagraph\n\n")
	}
	ws := tempWS(t, map[string]string{"long.md": src.String()})
	r, err := Load(context.Background(), Request{WS: ws, Path: "long.md", Width: 80, Blocks: 10})
	if err != nil || r.Rendered.Complete || len(r.Rendered.BlockLine) != 10 {
		t.Fatalf("partial: complete=%v blocks=%d err=%v", r.Rendered.Complete, len(r.Rendered.BlockLine), err)
	}
}

func TestSourceLinesWidth(t *testing.T) {
	r := SourceLines("x.go", "package x\n// 日本語のコメント\n", 80)
	for i, l := range r.Lines {
		if ansi.StringWidth(l) != ansi.StringWidth(r.Plain[i]) {
			t.Errorf("line %d: styled and plain widths differ", i)
		}
	}
	if len(r.BlockLine) != len(r.Lines) {
		t.Error("each source line must be its own block")
	}
}

func testPNG(t *testing.T) []byte {
	t.Helper()
	img := image.NewRGBA(image.Rect(0, 0, 40, 20))
	for x := range 40 {
		for y := range 20 {
			img.Set(x, y, color.RGBA{uint8(x * 6), uint8(y * 12), 100, 255})
		}
	}
	var buf bytes.Buffer
	if err := png.Encode(&buf, img); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}
