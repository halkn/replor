package preview

import (
	"fmt"
	"strings"

	"github.com/alecthomas/chroma/v2"
	"github.com/alecthomas/chroma/v2/formatters"
	"github.com/alecthomas/chroma/v2/lexers"
	"github.com/alecthomas/chroma/v2/styles"
	"github.com/charmbracelet/x/ansi"

	"github.com/halkn/replor/internal/document"
)

// maxHighlight bounds syntax highlighting; lexing megabytes of minified code
// is slow and the colors add little there.
const maxHighlight = 1 << 20

const tabWidth = 4

// SourceLines renders text with line numbers. Every line is its own block, so
// positions and search work the same way as for Markdown. Lines are not
// wrapped; the view truncates them.
func SourceLines(name, text string, width int) *Rendered {
	text = expandTabs(document.Sanitize(text))
	text = strings.TrimSuffix(text, "\n")
	raw := strings.Split(text, "\n")
	styled := raw
	if len(text) <= maxHighlight {
		if h := highlight(name, text); len(h) == len(raw) {
			styled = h
		}
	}
	digits := len(fmt.Sprint(len(raw)))
	r := &Rendered{Width: width, Lines: make([]string, len(raw)), Plain: make([]string, len(raw)), BlockLine: make([]int, len(raw)), Complete: true}
	for i := range raw {
		gutter := fmt.Sprintf("%*d │ ", digits, i+1)
		r.Lines[i] = gutterStyle + gutter + "\x1b[m" + styled[i]
		r.Plain[i] = gutter + raw[i]
		r.BlockLine[i] = i
	}
	return r
}

const gutterStyle = "\x1b[38;5;243m"

// highlight returns one styled string per line. Tokens are split at line
// breaks before formatting so that every line carries its own color codes.
func highlight(name, text string) []string {
	lexer := lexers.Match(name)
	if lexer == nil {
		return nil
	}
	it, err := chroma.Coalesce(lexer).Tokenise(nil, text)
	if err != nil {
		return nil
	}
	style := styles.Get("monokai")
	var out []string
	for _, line := range chroma.SplitTokensIntoLines(it.Tokens()) {
		var b strings.Builder
		if err := formatters.TTY256.Format(&b, style, chroma.Literator(line...)); err != nil {
			return nil
		}
		out = append(out, strings.TrimRight(b.String(), "\n"))
	}
	// SplitTokensIntoLines yields no entry for a trailing empty line.
	for len(out) < strings.Count(text, "\n")+1 {
		out = append(out, "")
	}
	return out
}

func expandTabs(s string) string {
	if !strings.Contains(s, "\t") {
		return s
	}
	var b strings.Builder
	col := 0
	for _, r := range s {
		switch r {
		case '\t':
			n := tabWidth - col%tabWidth
			b.WriteString(strings.Repeat(" ", n))
			col += n
		case '\n':
			b.WriteRune(r)
			col = 0
		default:
			b.WriteRune(r)
			col += ansi.StringWidth(string(r))
		}
	}
	return b.String()
}

func plainLines(lines []string) []string {
	out := make([]string, len(lines))
	for i, l := range lines {
		out[i] = ansi.Strip(l)
	}
	return out
}
