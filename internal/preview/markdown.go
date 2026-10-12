// Package preview turns documents into terminal lines while keeping the
// mapping back to document blocks needed for navigation.
package preview

import (
	"regexp"
	"sort"
	"strings"

	"charm.land/glamour/v2"
	"github.com/charmbracelet/x/ansi"

	"github.com/halkn/replor/internal/document"
)

// Rendered is a document laid out for one width. Lines carry ANSI styling;
// Plain holds the same lines without it for searching.
type Rendered struct {
	Width     int
	Lines     []string
	Plain     []string
	BlockLine []int // first line of each document block
}

// Position is a width-independent reading position: a block and the number
// of lines into it. It survives re-layout on resize.
type Position struct {
	Block  int
	Offset int
}

// BlockHook may replace the rendering of a block (e.g. a Mermaid fence with
// an image). Returning ok=false keeps the default rendering.
type BlockHook func(block int, width int) (lines []string, ok bool)

// RenderMarkdown renders each top-level block separately so that line ranges
// map back to blocks exactly. Reference definitions are appended to every
// block so reference-style links still resolve.
func RenderMarkdown(d *document.Doc, width int, hook BlockHook) (*Rendered, error) {
	tr, err := glamour.NewTermRenderer(
		glamour.WithStandardStyle("dark"),
		glamour.WithWordWrap(width),
	)
	if err != nil {
		return nil, err
	}
	r := &Rendered{Width: width, BlockLine: make([]int, len(d.Blocks))}
	for i := range d.Blocks {
		r.BlockLine[i] = len(r.Lines)
		var lines []string
		if hook != nil {
			lines, _ = hook(i, width)
		}
		if lines == nil {
			out, err := tr.Render(d.BlockSource(i) + "\n" + d.RefDefs)
			if err != nil {
				return nil, err
			}
			lines = trimBlank(strings.Split(filterHyperlinks(out), "\n"))
		}
		if len(lines) == 0 {
			continue
		}
		if i > 0 && len(r.Lines) > 0 {
			r.BlockLine[i]++
			r.Lines = append(r.Lines, "")
		}
		r.Lines = append(r.Lines, lines...)
	}
	r.Plain = make([]string, len(r.Lines))
	for i, l := range r.Lines {
		r.Plain[i] = ansi.Strip(l)
	}
	return r, nil
}

// glamour emits an OSC 8 hyperlink for any parseable URL, which would let a
// terminal click open file:, javascript: or custom-scheme URLs. Only http(s)
// targets keep their hyperlink; the visible text is untouched.
var osc8 = regexp.MustCompile("\x1b\\]8;([^;\x07\x1b]*);([^\x07\x1b]*)(\x07|\x1b\\\\)")

func filterHyperlinks(s string) string {
	return osc8.ReplaceAllStringFunc(s, func(m string) string {
		sub := osc8.FindStringSubmatch(m)
		u := strings.ToLower(sub[2])
		if u == "" || strings.HasPrefix(u, "http://") || strings.HasPrefix(u, "https://") {
			return m
		}
		return ""
	})
}

func trimBlank(lines []string) []string {
	isBlank := func(s string) bool { return strings.TrimSpace(ansi.Strip(s)) == "" }
	for len(lines) > 0 && isBlank(lines[0]) {
		lines = lines[1:]
	}
	for len(lines) > 0 && isBlank(lines[len(lines)-1]) {
		lines = lines[:len(lines)-1]
	}
	return lines
}

// BlockAt returns the block whose lines contain line.
func (r *Rendered) BlockAt(line int) int {
	i := sort.Search(len(r.BlockLine), func(i int) bool { return r.BlockLine[i] > line })
	return max(i-1, 0)
}

// PositionAt converts a line to a Position.
func (r *Rendered) PositionAt(line int) Position {
	if len(r.BlockLine) == 0 {
		return Position{}
	}
	b := r.BlockAt(line)
	return Position{Block: b, Offset: line - r.BlockLine[b]}
}

// LineOf converts a Position back to a line, clamping the offset to the block
// (a block can become shorter at a wider width).
func (r *Rendered) LineOf(p Position) int {
	if len(r.BlockLine) == 0 {
		return 0
	}
	p.Block = min(max(p.Block, 0), len(r.BlockLine)-1)
	end := len(r.Lines)
	if p.Block+1 < len(r.BlockLine) {
		end = r.BlockLine[p.Block+1]
	}
	return min(r.BlockLine[p.Block]+max(p.Offset, 0), max(end-1, r.BlockLine[p.Block]))
}

// Match is a search hit in rendered text, in display columns.
type Match struct {
	Line, Col int
}

// Find returns literal, case-insensitive matches in document order.
func (r *Rendered) Find(query string) []Match {
	if query == "" {
		return nil
	}
	q := strings.ToLower(query)
	var out []Match
	for i, l := range r.Plain {
		low := strings.ToLower(l)
		for off := 0; ; {
			j := strings.Index(low[off:], q)
			if j < 0 {
				break
			}
			out = append(out, Match{Line: i, Col: ansi.StringWidth(l[:min(off+j, len(l))])})
			off += j + len(q)
		}
	}
	return out
}
