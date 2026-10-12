package document

import (
	"bytes"
	"fmt"
	"sort"
	"strings"

	"github.com/yuin/goldmark"
	"github.com/yuin/goldmark/ast"
	"github.com/yuin/goldmark/extension"
	"github.com/yuin/goldmark/parser"
	"github.com/yuin/goldmark/text"
)

// Block is a top-level Markdown block. Start/End cover whole source lines, so
// concatenating every block's source reproduces the document after the first
// block.
type Block struct {
	Start, End int
	Kind       ast.NodeKind
	// Lang and Code are set for fenced code blocks.
	Lang, Code string
	// Image is the destination when the block is a paragraph holding only
	// one image.
	Image string
}

// Heading is a heading anywhere in the document with its GitHub-style ID.
type Heading struct {
	Block int
	Level int
	ID    string
	Text  string
}

// DocLink is an inline link in document order.
type DocLink struct {
	Block int
	Dest  string
	Text  string
}

// Doc is the navigable structure of a Markdown document. It is the source of
// truth for headings, anchors and links; rendered output is derived from it.
type Doc struct {
	Source   string
	Blocks   []Block
	Headings []Heading
	Links    []DocLink
	// RefDefs re-declares every link reference definition so that a block can
	// be rendered on its own without losing reference-style links.
	RefDefs string
}

var md = goldmark.New(
	goldmark.WithExtensions(extension.GFM, extension.Footnote),
	goldmark.WithParserOptions(parser.WithAutoHeadingID()),
)

// Parse builds a Doc. The source is sanitized first; all offsets refer to
// Doc.Source.
func Parse(src string) *Doc {
	src = Sanitize(src)
	source := []byte(src)
	ctx := parser.NewContext(parser.WithIDs(newGitHubIDs()))
	root := md.Parser().Parse(text.NewReader(source), parser.WithContext(ctx))

	// The footnote extension moves definitions to a list at the end of the
	// AST, so top-level nodes are ordered by source position, not AST order.
	type top struct {
		node  ast.Node
		start int
	}
	var tops []top
	for n := root.FirstChild(); n != nil; n = n.NextSibling() {
		if start := blockStart(n, source); start >= 0 {
			tops = append(tops, top{n, lineStart(source, start)})
		}
	}
	sort.SliceStable(tops, func(i, j int) bool { return tops[i].start < tops[j].start })

	d := &Doc{Source: src}
	for i, t := range tops {
		b := Block{Start: t.start, End: len(source), Kind: t.node.Kind()}
		switch v := t.node.(type) {
		case *ast.FencedCodeBlock:
			b.Lang = string(v.Language(source))
			var code strings.Builder
			for j := 0; j < v.Lines().Len(); j++ {
				seg := v.Lines().At(j)
				code.Write(seg.Value(source))
			}
			b.Code = code.String()
		case *ast.Paragraph:
			if img, ok := v.FirstChild().(*ast.Image); ok && v.ChildCount() == 1 {
				b.Image = string(img.Destination)
			}
		}
		if i == 0 {
			b.Start = 0
		}
		if i+1 < len(tops) {
			b.End = tops[i+1].start
		}
		d.Blocks = append(d.Blocks, b)
	}

	for bi, t := range tops {
		_ = ast.Walk(t.node, func(c ast.Node, entering bool) (ast.WalkStatus, error) {
			if !entering {
				return ast.WalkContinue, nil
			}
			switch v := c.(type) {
			case *ast.Heading:
				id, _ := v.AttributeString("id")
				idb, _ := id.([]byte)
				d.Headings = append(d.Headings, Heading{Block: bi, Level: v.Level, ID: string(idb), Text: plainText(v, source)})
			case *ast.Link:
				d.Links = append(d.Links, DocLink{Block: bi, Dest: string(v.Destination), Text: plainText(v, source)})
			case *ast.AutoLink:
				d.Links = append(d.Links, DocLink{Block: bi, Dest: string(v.URL(source)), Text: string(v.Label(source))})
			}
			return ast.WalkContinue, nil
		})
	}

	var refs strings.Builder
	for _, r := range ctx.References() {
		fmt.Fprintf(&refs, "[%s]: <%s>", r.Label(), r.Destination())
		if len(r.Title()) > 0 {
			fmt.Fprintf(&refs, " %q", r.Title())
		}
		refs.WriteByte('\n')
	}
	d.RefDefs = refs.String()
	return d
}

// BlockSource returns the source of block i.
func (d *Doc) BlockSource(i int) string { return d.Source[d.Blocks[i].Start:d.Blocks[i].End] }

// HeadingByID finds a heading by its anchor ID.
func (d *Doc) HeadingByID(id string) (Heading, bool) {
	for _, h := range d.Headings {
		if h.ID == id {
			return h, true
		}
	}
	return Heading{}, false
}

// blockStart returns the source offset of n, falling back to the first line
// segment of the node or its descendants (some block kinds carry no Pos).
func blockStart(n ast.Node, source []byte) int {
	if p := n.Pos(); p >= 0 {
		return p
	}
	start := -1
	_ = ast.Walk(n, func(c ast.Node, entering bool) (ast.WalkStatus, error) {
		if !entering || c.Type() != ast.TypeBlock {
			return ast.WalkContinue, nil
		}
		if l := c.Lines(); l != nil && l.Len() > 0 {
			start = l.At(0).Start
			return ast.WalkStop, nil
		}
		return ast.WalkContinue, nil
	})
	return start
}

func lineStart(source []byte, off int) int {
	if off > len(source) {
		off = len(source)
	}
	return bytes.LastIndexByte(source[:off], '\n') + 1
}

func plainText(n ast.Node, source []byte) string {
	var b strings.Builder
	_ = ast.Walk(n, func(c ast.Node, entering bool) (ast.WalkStatus, error) {
		if !entering {
			return ast.WalkContinue, nil
		}
		switch v := c.(type) {
		case *ast.Text:
			b.Write(v.Segment.Value(source))
			if v.SoftLineBreak() {
				b.WriteByte(' ')
			}
		case *ast.String:
			b.Write(v.Value)
		case *ast.CodeSpan:
			for t := v.FirstChild(); t != nil; t = t.NextSibling() {
				if tx, ok := t.(*ast.Text); ok {
					b.Write(tx.Segment.Value(source))
				}
			}
			return ast.WalkSkipChildren, nil
		}
		return ast.WalkContinue, nil
	})
	return b.String()
}
