package preview

import (
	"bytes"
	"fmt"
	"strings"

	"github.com/halkn/replor/internal/document"
)

// SGR prefixes of the diff colors; each line is reset at its end.
const (
	diffMeta = "\x1b[38;5;243m" // headers
	diffHunk = "\x1b[38;5;75m"  // @@ … @@
	diffAdd  = "\x1b[38;5;78m"
	diffDel  = "\x1b[38;5;203m"
)

// DiffLines colors a unified diff. Every line is its own block so that
// positions and search work as in source view; lines are not wrapped.
func DiffLines(diff string, width int) *Rendered {
	text := strings.TrimSuffix(expandTabs(document.Sanitize(diff)), "\n")
	raw := strings.Split(text, "\n")
	r := &Rendered{Width: width, Lines: make([]string, len(raw)), Plain: raw, BlockLine: make([]int, len(raw)), Complete: true}
	header := true
	for i, l := range raw {
		r.BlockLine[i] = i
		style := ""
		switch {
		case strings.HasPrefix(l, "diff --git "):
			header, style = true, diffMeta
		case strings.HasPrefix(l, "@@"):
			header, style = false, diffHunk
		case header:
			style = diffMeta
		case strings.HasPrefix(l, "+"):
			style = diffAdd
		case strings.HasPrefix(l, "-"):
			style = diffDel
		case strings.HasPrefix(l, `\`):
			style = diffMeta
		}
		if style == "" {
			r.Lines[i] = l
		} else {
			r.Lines[i] = style + l + "\x1b[m"
		}
	}
	return r
}

// UntrackedDiff presents an untracked file as an addition, in the same
// format git uses for a new file. data comes from the workspace read, so the
// read limits and boundary checks apply.
func UntrackedDiff(path string, data []byte) []byte {
	var b bytes.Buffer
	fmt.Fprintf(&b, "diff --git a/%s b/%s\nnew file (untracked)\n", path, path)
	switch {
	case len(data) == 0:
		return b.Bytes()
	case bytes.IndexByte(data[:min(len(data), 8000)], 0) >= 0:
		fmt.Fprintf(&b, "Binary file %s (%d bytes)\n", path, len(data))
		return b.Bytes()
	}
	text := string(data)
	missingNL := !strings.HasSuffix(text, "\n")
	lines := strings.Split(strings.TrimSuffix(text, "\n"), "\n")
	fmt.Fprintf(&b, "--- /dev/null\n+++ b/%s\n@@ -0,0 +1,%d @@\n", path, len(lines))
	for _, l := range lines {
		b.WriteString("+" + l + "\n")
	}
	if missingNL {
		b.WriteString("\\ No newline at end of file\n")
	}
	return b.Bytes()
}
