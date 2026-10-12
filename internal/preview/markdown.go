// Package preview turns documents into terminal lines while keeping the
// mapping back to document blocks needed for navigation.
package preview

import (
	"fmt"
	"regexp"
	"sort"
	"strings"

	"charm.land/glamour/v2"
	gansi "charm.land/glamour/v2/ansi"
	"charm.land/glamour/v2/styles"
	"github.com/charmbracelet/x/ansi"

	"github.com/halkn/replor/internal/document"
)

// Rendered is a document laid out for one width. Lines carry ANSI styling;
// Plain holds the same lines without it for searching.
type Rendered struct {
	Width     int
	Lines     []string
	Plain     []string
	BlockLine []int // first line of each rendered document block
	// Complete is false when only the leading blocks were rendered
	// (Options.Blocks); BlockLine is then shorter than the document.
	Complete bool
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

// Options controls RenderMarkdown.
type Options struct {
	Width int
	Hook  BlockHook
	// Blocks limits rendering to the first n blocks (0 = all). Blocks are
	// rendered independently, so a partial result is an exact prefix of the
	// full one and positions within it stay valid.
	Blocks int
}

// hrefOpen/hrefClose wrap the URL part that glamour prints after a link or
// image text, so that it can be told apart from the text after rendering.
// They are private-use characters and are removed from the input first.
const (
	hrefOpen  = "\uE000"
	hrefClose = "\uE001"
)

var markerScrub = strings.NewReplacer(hrefOpen, "\uFFFD", hrefClose, "\uFFFD")

func styleConfig() gansi.StyleConfig {
	st := styles.DarkStyleConfig
	st.Link.Prefix, st.Link.Suffix = hrefOpen, hrefClose
	st.Image.Prefix, st.Image.Suffix = hrefOpen, hrefClose
	return st
}

// RenderMarkdown renders each top-level block separately so that line ranges
// map back to blocks exactly. Reference definitions are appended to every
// block so reference-style links still resolve.
func RenderMarkdown(d *document.Doc, opt Options) (*Rendered, error) {
	tr, err := glamour.NewTermRenderer(
		glamour.WithStyles(styleConfig()),
		glamour.WithWordWrap(opt.Width),
	)
	if err != nil {
		return nil, err
	}
	n := len(d.Blocks)
	if opt.Blocks > 0 && opt.Blocks < n {
		n = opt.Blocks
	}
	r := &Rendered{Width: opt.Width, BlockLine: make([]int, n), Complete: n == len(d.Blocks)}
	for i := range n {
		r.BlockLine[i] = len(r.Lines)
		var lines []string
		if opt.Hook != nil {
			lines, _ = opt.Hook(i, opt.Width)
		}
		if lines == nil {
			out, err := tr.Render(markerScrub.Replace(blockMarkdown(d, i)) + "\n" + d.RefDefs)
			if err != nil {
				return nil, err
			}
			lines = trimBlank(strings.Split(filterHyperlinks(stripLocalHrefs(out)), "\n"))
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

var alertLabels = map[string]string{
	"NOTE": "Note", "TIP": "Tip", "IMPORTANT": "Important", "WARNING": "Warning", "CAUTION": "Caution",
}

// blockMarkdown returns the Markdown to give glamour for block i, rewriting
// the constructs glamour does not support: alert markers become a bold label
// and the footnote list is regenerated (glamour would read the definitions
// as link references and drop them).
func blockMarkdown(d *document.Doc, i int) string {
	b := d.Blocks[i]
	src := d.BlockSource(i)
	switch {
	case b.Alert != "":
		// The first line is "> [!KIND]" (document.alertKind checked it).
		first, rest, _ := strings.Cut(src, "\n")
		// An empty quote line keeps the label in its own paragraph.
		return first[:strings.Index(first, "[!")] + "**" + alertLabels[b.Alert] + "**\n>\n" + rest
	case len(b.Footnotes) > 0:
		var md strings.Builder
		md.WriteString("**Footnotes**\n\n")
		for _, f := range b.Footnotes {
			fmt.Fprintf(&md, "- \\[^%s\\] %s\n", escapeMarkdown(f.Ref), escapeMarkdown(f.Text))
		}
		return md.String()
	}
	return src
}

var escapeMarkdown = strings.NewReplacer(
	"\\", "\\\\", "`", "\\`", "*", "\\*", "_", "\\_", "[", "\\[", "]", "\\]", "<", "\\<", ">", "\\>", "#", "\\#",
).Replace

// hrefPart matches one URL part (with the space glamour puts before it,
// possibly wrapped in SGR codes).
var hrefPart = regexp.MustCompile("(?s)(?: ((?:\x1b\\[[0-9;]*m)*))?" + hrefOpen + "(.*?)" + hrefClose)

// stripLocalHrefs drops the URL glamour prints after a link or image unless
// it is an http(s) URL; local paths are noise next to the link text and are
// shown in the status line when the link is selected.
func stripLocalHrefs(s string) string {
	return hrefPart.ReplaceAllStringFunc(s, func(m string) string {
		sub := hrefPart.FindStringSubmatch(m)
		inner := sub[2]
		u := ansi.Strip(inner)
		if o := osc8.FindStringSubmatch(inner); o != nil && o[2] != "" {
			u = o[2]
		}
		if isWebURL(u) {
			return strings.TrimSuffix(strings.Replace(m, hrefOpen, "", 1), hrefClose)
		}
		// Keep line structure if glamour wrapped inside the URL.
		return sub[1] + strings.Repeat("\n", strings.Count(m, "\n"))
	})
}

func isWebURL(u string) bool {
	u = strings.ToLower(strings.TrimSpace(u))
	return strings.HasPrefix(u, "http://") || strings.HasPrefix(u, "https://")
}

// glamour emits an OSC 8 hyperlink for any parseable URL, which would let a
// terminal click open file:, javascript: or custom-scheme URLs. Only http(s)
// targets keep their hyperlink; the visible text is untouched.
var osc8 = regexp.MustCompile("\x1b\\]8;([^;\x07\x1b]*);([^\x07\x1b]*)(\x07|\x1b\\\\)")

func filterHyperlinks(s string) string {
	return osc8.ReplaceAllStringFunc(s, func(m string) string {
		sub := osc8.FindStringSubmatch(m)
		if sub[2] == "" || isWebURL(sub[2]) {
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
