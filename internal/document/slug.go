package document

import (
	"strconv"
	"strings"
	"unicode"

	"github.com/yuin/goldmark/ast"
)

// gitHubIDs generates heading anchors the way GitHub does (github-slugger):
// lower-case, drop everything except letters, marks, numbers, "_", "-" and
// spaces, turn spaces into "-", and suffix duplicates with -1, -2, ...
// goldmark's default generator drops non-ASCII letters, which breaks
// Japanese headings.
type gitHubIDs struct{ used map[string]bool }

func newGitHubIDs() *gitHubIDs { return &gitHubIDs{used: map[string]bool{}} }

// Slug returns the GitHub anchor for a heading text without deduplication.
func Slug(s string) string {
	var b strings.Builder
	for _, r := range strings.ToLower(s) {
		switch {
		case r == ' ':
			b.WriteByte('-')
		case r == '-' || r == '_' || unicode.IsLetter(r) || unicode.IsNumber(r) || unicode.IsMark(r):
			b.WriteRune(r)
		}
	}
	return b.String()
}

func (g *gitHubIDs) Generate(value []byte, _ ast.NodeKind) []byte {
	base := Slug(string(value))
	id := base
	for i := 1; g.used[id]; i++ {
		id = base + "-" + strconv.Itoa(i)
	}
	g.used[id] = true
	return []byte(id)
}

func (g *gitHubIDs) Put(value []byte) { g.used[string(value)] = true }
