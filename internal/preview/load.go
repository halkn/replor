package preview

import (
	"bytes"
	"context"
	"fmt"
	"image"
	_ "image/gif" // decoders registered for image.Decode
	_ "image/jpeg"
	"image/png"
	"path"
	"strings"
	"sync/atomic"

	"github.com/halkn/replor/internal/document"
	"github.com/halkn/replor/internal/workspace"
)

// Read limits (DESIGN 6.1, ADR-0001 3.5).
const (
	MaxPreview = 1 << 20 // larger Markdown is shown as source
	MaxSource  = 8 << 20 // larger files are not read
)

// Kind is what a file turned out to be.
type Kind int

const (
	KindText Kind = iota
	KindMarkdown
	KindImage
	KindBinary
	KindEmpty
	KindDiff // produced by the caller from git output (DiffLines)
)

// Request asks for one file rendered at one width.
type Request struct {
	WS     *workspace.Workspace
	Path   string
	Source bool // Markdown as source instead of preview
	Width  int
	Images ImageMode
	// Blocks limits Markdown rendering to the first n blocks (see Options).
	Blocks int
}

// Result is a rendered file. Rendered is nil for KindEmpty and KindBinary.
type Result struct {
	Kind     Kind
	Size     int
	Doc      *document.Doc // Markdown preview only
	Rendered *Rendered
	// Transmit holds Kitty upload sequences to write before showing
	// Rendered; KittyIDs are the images to delete when it is replaced.
	Transmit []string
	KittyIDs []uint32
	// Note explains a fallback (e.g. Markdown too large for preview).
	Note string
}

// Load reads and renders a file. It runs off the UI loop and checks ctx
// between stages so that superseded requests stop early.
func Load(ctx context.Context, req Request) (Result, error) {
	data, err := req.WS.ReadFile(req.Path, MaxSource)
	if err != nil {
		return Result{}, err
	}
	res := Result{Size: len(data)}
	if err := ctx.Err(); err != nil {
		return res, err
	}

	switch {
	case len(data) == 0:
		res.Kind = KindEmpty
		return res, nil
	case isImage(data):
		img, _, err := image.Decode(bytes.NewReader(data))
		if err != nil {
			res.Kind = KindBinary
			return res, nil
		}
		res.Kind = KindImage
		lines, tx, id := picture(img, req.Width, req.Images)
		res.Rendered = &Rendered{Width: req.Width, Lines: lines, Plain: plainLines(lines), BlockLine: []int{0}, Complete: true}
		if tx != "" {
			res.Transmit, res.KittyIDs = []string{tx}, []uint32{id}
		}
		return res, nil
	case bytes.IndexByte(data[:min(len(data), 8000)], 0) >= 0:
		res.Kind = KindBinary
		return res, nil
	}

	text := string(data)
	if !IsMarkdown(req.Path) {
		res.Kind = KindText
		res.Rendered = SourceLines(req.Path, text, req.Width)
		return res, nil
	}
	res.Kind = KindMarkdown
	if req.Source || len(data) > MaxPreview {
		if !req.Source {
			res.Note = fmt.Sprintf("shown as source: larger than the %d KiB preview limit", MaxPreview>>10)
		}
		res.Rendered = SourceLines(req.Path, text, req.Width)
		return res, nil
	}

	d := document.Parse(text)
	if err := ctx.Err(); err != nil {
		return res, err
	}
	hook := func(i, w int) ([]string, bool) {
		dest := d.Blocks[i].Image
		if dest == "" || req.Images == ImageOff {
			return nil, false
		}
		// Only workspace files; remote images are never fetched.
		l, err := document.ResolveLink(req.WS, req.Path, ".", dest)
		if err != nil || l.Kind != document.LinkLocal {
			return nil, false
		}
		raw, err := req.WS.ReadFile(l.Path, MaxSource)
		if err != nil || !isImage(raw) {
			return nil, false
		}
		img, _, err := image.Decode(bytes.NewReader(raw))
		if err != nil {
			return nil, false
		}
		lines, tx, id := picture(img, w, req.Images)
		if tx != "" {
			res.Transmit = append(res.Transmit, tx)
			res.KittyIDs = append(res.KittyIDs, id)
		}
		return lines, true
	}
	r, err := RenderMarkdown(d, Options{Width: req.Width, Hook: hook, Blocks: req.Blocks})
	if err != nil {
		return res, err
	}
	res.Doc, res.Rendered = d, r
	return res, nil
}

// IsMarkdown reports whether a path is rendered as Markdown.
func IsMarkdown(p string) bool {
	switch strings.ToLower(path.Ext(p)) {
	case ".md", ".markdown", ".mdown", ".mkd":
		return true
	}
	return false
}

func isImage(data []byte) bool {
	return bytes.HasPrefix(data, []byte("\x89PNG\r\n\x1a\n")) ||
		bytes.HasPrefix(data, []byte("\xff\xd8\xff")) ||
		bytes.HasPrefix(data, []byte("GIF8"))
}

var kittySeq atomic.Uint32

// picture lays out an image no wider than width (and at most 24 rows, so a
// tall image does not push the text away).
func picture(img image.Image, width int, mode ImageMode) (lines []string, transmit string, id uint32) {
	b := img.Bounds()
	if mode == ImageOff {
		return []string{fmt.Sprintf("[image %d×%d]", b.Dx(), b.Dy())}, "", 0
	}
	cols, rows := FitCells(b, min(width, b.Dx()/8+1), 24)
	if mode == ImageKitty {
		cols, rows = min(cols, MaxKittyCells), min(rows, MaxKittyCells)
		// IDs stay below 1<<24 (see KittyPlaceholders).
		id = kittySeq.Add(1)%(1<<24-1) + 1
		var buf bytes.Buffer
		if err := png.Encode(&buf, img); err != nil {
			return []string{fmt.Sprintf("[image %d×%d]", b.Dx(), b.Dy())}, "", 0
		}
		return KittyPlaceholders(id, cols, rows), KittyTransmit(buf.Bytes(), id, cols, rows), id
	}
	return Halfblocks(img, cols, rows), "", 0
}
