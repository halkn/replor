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
	// Sanitize drops a CR before LF; in a diff it can be the only change
	// (CRLF → LF), so it is made visible first.
	diff = strings.ReplaceAll(diff, "\r\n", "\u240d\n")
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
	case isBinary(data):
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

// UntrackedLinkDiff presents an untracked symbolic link like git does: the
// link target as the file content.
func UntrackedLinkDiff(path, target string) []byte {
	return []byte(fmt.Sprintf("diff --git a/%s b/%s\nnew file mode 120000 (untracked symbolic link)\n--- /dev/null\n+++ b/%s\n@@ -0,0 +1 @@\n+%s\n\\ No newline at end of file\n", path, path, path, target))
}

// UntrackedBaseDiff presents a file that the branch's merge base has and the
// index does not as a change from the base content: every base line removed
// and every current line added. git has no command that compares a blob
// with an untracked file without writing to the repository.
func UntrackedBaseDiff(path string, base, cur []byte) []byte {
	var b bytes.Buffer
	fmt.Fprintf(&b, "diff --git a/%s b/%s\nuntracked now; compared with the merge base\n", path, path)
	switch {
	case bytes.Equal(base, cur):
		b.WriteString("same content as in the merge base\n")
		return b.Bytes()
	case isBinary(base) || isBinary(cur):
		fmt.Fprintf(&b, "Binary files a/%s and b/%s differ\n", path, path)
		return b.Bytes()
	}
	was, now := diffSide(base), diffSide(cur)
	fmt.Fprintf(&b, "--- a/%s\n+++ b/%s\n@@ -%s +%s @@\n", path, path, hunkRange(len(was.lines)), hunkRange(len(now.lines)))
	was.write(&b, "-")
	now.write(&b, "+")
	return b.Bytes()
}

type side struct {
	lines     []string
	missingNL bool
}

func diffSide(data []byte) side {
	if len(data) == 0 {
		return side{}
	}
	text := string(data)
	return side{strings.Split(strings.TrimSuffix(text, "\n"), "\n"), !strings.HasSuffix(text, "\n")}
}

func (s side) write(b *bytes.Buffer, prefix string) {
	for _, l := range s.lines {
		b.WriteString(prefix + l + "\n")
	}
	if s.missingNL {
		b.WriteString("\\ No newline at end of file\n")
	}
}

// hunkRange formats a whole-file range the way git does.
func hunkRange(n int) string {
	switch n {
	case 0:
		return "0,0"
	case 1:
		return "1"
	}
	return fmt.Sprintf("1,%d", n)
}
