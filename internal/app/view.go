package app

import (
	"fmt"
	"path"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"

	"github.com/halkn/replor/internal/document"
	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/workspace"
)

var (
	accent   = lipgloss.NewStyle().Foreground(lipgloss.Color("63"))
	dim      = lipgloss.NewStyle().Foreground(lipgloss.Color("243"))
	warn     = lipgloss.NewStyle().Foreground(lipgloss.Color("203"))
	selected = lipgloss.NewStyle().Reverse(true)
	barBG    = lipgloss.Color("236")
	bar      = lipgloss.NewStyle().Background(barBG).Foreground(lipgloss.Color("252"))
	tabOn    = lipgloss.NewStyle().Background(lipgloss.Color("63")).Foreground(lipgloss.Color("230"))
)

func (m *Model) View() tea.View {
	if m.width == 0 {
		return tea.NewView("")
	}
	h := m.bodyHeight()
	viewer := m.viewerLines(h)
	var side []string
	if m.sideVisible() {
		side = m.treeLines(h)
	}
	sep := "│"
	if m.focus == paneViewer {
		sep = accent.Render(sep)
	}
	// Rows are composed and truncated one by one instead of with
	// lipgloss.JoinHorizontal, which pads every row to the widest one and so
	// amplifies disagreements on emoji widths (ADR-0001 3.1).
	rows := make([]string, 0, h+2)
	rows = append(rows, m.header())
	for i := range h {
		row := " " + viewer[i]
		if side != nil {
			row = side[i] + sep + row
		}
		rows = append(rows, ansi.Truncate(row, m.width, ""))
	}
	rows = append(rows, m.footer())
	v := tea.NewView(strings.Join(rows, "\n"))
	v.AltScreen = true
	return v
}

// line fits one bar line to the width. Callers pass text from file names,
// paths, errors and input through clean; the bar's own styling must not be.
func (m *Model) line(s string) string {
	return bar.Width(m.width).Render(ansi.Truncate(s, m.width, "…"))
}

// clean makes untrusted text safe to draw (document.Sanitize).
func clean(s string) string { return document.Sanitize(s) }

// onBar gives a style the bar background. Every styled part ends with a
// full reset, so each part of a bar line carries the background itself.
func onBar(st lipgloss.Style) lipgloss.Style { return st.Background(barBG) }

// barLine joins parts styled on the bar and pads them to the width.
func (m *Model) barLine(parts ...string) string {
	s := ansi.Truncate(strings.Join(parts, ""), m.width, bar.Render("…"))
	return s + bar.Render(strings.Repeat(" ", max(0, m.width-ansi.StringWidth(s))))
}

func (m *Model) header() string {
	parts := []string{bar.Render(" " + clean(path.Base(m.ws.Root)) + "  ")}
	parts = append(parts, m.headerGit()...)
	v := &m.viewer
	if v.target != nil {
		parts = append(parts, bar.Render("  "+clean(v.target.Path)))
		if v.res.Kind == preview.KindMarkdown || v.target.Mode == ModeSource {
			pv, src := bar.Render(" Preview "), bar.Render(" Source ")
			if v.target.Mode == ModeSource {
				src = tabOn.Render(" Source ")
			} else {
				pv = tabOn.Render(" Preview ")
			}
			parts = append(parts, bar.Render("  "), pv, src)
		}
	}
	if v.pending {
		parts = append(parts, bar.Render("  ⟳"))
	}
	return m.barLine(parts...)
}

func (m *Model) footer() string {
	v := &m.viewer
	var s string
	switch {
	case m.overlay == overlaySearch:
		s = " /" + clean(m.input) + "▏"
	case m.overlay == overlayFinder:
		s = " Enter open · ↑/↓ choose · Esc close"
	case m.status != "":
		s = " " + clean(m.status)
	case v.linkIdx >= 0 && v.doc() != nil:
		l := v.doc().Links[v.linkIdx]
		s = fmt.Sprintf(" link %d/%d: %s → %s  (Enter to follow)", v.linkIdx+1, len(v.doc().Links), clean(l.Text), clean(l.Dest))
	case len(v.matches) > 0:
		s = fmt.Sprintf(" %q %d/%d  (n/N)", clean(v.query), v.matchIdx+1, len(v.matches))
	default:
		s = " Tab focus · Enter open · / search · t TOC · > link · ⌫ back · ? help · q quit"
	}
	return m.line(s)
}

func (m *Model) treeLines(h int) []string {
	w := m.sideWidth()
	t := &m.tree
	out := make([]string, 0, h)
	switch {
	case t.rootErr != "":
		out = append(out, warn.Render(fit(clean(t.rootErr), w)))
	case t.rootLoading && len(t.rows) == 0:
		out = append(out, dim.Render(fit("loading…", w)))
	case len(t.rows) == 0:
		out = append(out, dim.Render(fit("(empty directory)", w)))
	}
	// Keep the cursor visible.
	if t.cursor < t.top {
		t.top = t.cursor
	}
	if t.cursor >= t.top+h {
		t.top = t.cursor - h + 1
	}
	for i := t.top; i < len(t.rows) && len(out) < h; i++ {
		out = append(out, m.treeRow(i, w))
	}
	if t.rootCut && len(out) < h {
		out = append(out, dim.Render(fit(fmt.Sprintf("(first %d entries)", maxDirEntries), w)))
	}
	for len(out) < h {
		out = append(out, strings.Repeat(" ", w))
	}
	return out
}

func (m *Model) treeRow(i, w int) string {
	r := m.tree.rows[i]
	icon := "  "
	switch r.node.Kind {
	case workspace.NodeDir:
		icon = "▸ "
		if r.expanded {
			icon = "▾ "
		}
	case workspace.NodeBroken:
		icon = "⚠ "
	}
	name := clean(r.node.Name)
	if r.node.Symlink {
		name += " →"
	}
	suffix := ""
	switch {
	case r.loading:
		suffix = " …"
	case r.err != "":
		suffix = " (unreadable)"
	case r.truncated:
		suffix = fmt.Sprintf(" (first %d)", maxDirEntries)
	}
	// The last two cells hold the git mark: a status letter for files, a
	// dot for directories with changes below.
	gitMark := " "
	switch {
	case m.git.isIgnored(r.node.Rel):
		gitMark = dim.Render("!")
	case m.git.markOf(r.node.Rel) != 0:
		gitMark = styledMark(m.git.markOf(r.node.Rel))
	case m.git.isDirty(r.node.Rel):
		gitMark = markStyles['M'].Render("•")
	}
	text := fit(strings.Repeat("  ", r.depth)+icon+name+suffix, w-2)
	switch {
	case i == m.tree.cursor && m.focus == paneFiles:
		text = selected.Render(text)
	case i == m.tree.cursor, r.node.Kind == workspace.NodeBroken, m.git.isIgnored(r.node.Rel):
		text = dim.Render(text)
	}
	return text + " " + gitMark
}

// fit truncates or pads s to exactly w cells.
func fit(s string, w int) string {
	s = ansi.Truncate(s, w, "…")
	return s + strings.Repeat(" ", max(0, w-ansi.StringWidth(s)))
}

func (m *Model) viewerLines(h int) []string {
	w := m.viewerWidth()
	v := &m.viewer
	var out []string
	switch {
	case m.overlay == overlayFinder:
		out = m.finderLines(w, h)
	case m.overlay == overlayHelp:
		out = m.helpLines(w)
	case m.overlay == overlayTOC && v.doc() != nil:
		out = m.tocLines(w, h)
	case v.target == nil:
		out = []string{dim.Render("Select a file in the tree (Enter), or press ? for help.")}
	case v.err != nil:
		out = []string{warn.Render(ansi.Truncate(clean("Cannot show "+v.target.Path+": "+v.err.Error()), w, "…"))}
	case v.rendered() == nil && v.pending:
		out = []string{dim.Render("Loading…")}
	case v.res.Kind == preview.KindEmpty:
		out = []string{dim.Render("(empty file)")}
	case v.res.Kind == preview.KindBinary:
		out = []string{dim.Render(fmt.Sprintf("Binary file (%d bytes) — not shown.", v.res.Size))}
	case v.rendered() != nil:
		out = m.documentLines(w, h)
	}
	for len(out) < h {
		out = append(out, "")
	}
	return out[:h]
}

func (m *Model) documentLines(w, h int) []string {
	v := &m.viewer
	r := v.rendered()
	matchLine, focusBlock := -1, -1
	if len(v.matches) > 0 {
		matchLine = v.matches[v.matchIdx].Line
	}
	if v.linkIdx >= 0 && v.doc() != nil {
		focusBlock = v.doc().Links[v.linkIdx].Block
	}
	end := min(v.top+h, len(r.Lines))
	out := make([]string, 0, h)
	for i := v.top; i < end; i++ {
		gutter := " "
		switch {
		case i == matchLine:
			gutter = accent.Render("▶")
		case focusBlock >= 0 && r.BlockAt(i) == focusBlock:
			gutter = accent.Render("┃")
		}
		out = append(out, gutter+ansi.Truncate(r.Lines[i], w, ""))
	}
	if !r.Complete && end == len(r.Lines) && len(out) < h {
		out = append(out, dim.Render(" rendering…"))
	}
	return out
}

func (m *Model) tocLines(w, h int) []string {
	hs := m.viewer.doc().Headings
	out := []string{accent.Render("Contents") + dim.Render("  (Enter jump · Esc close)")}
	start := max(0, m.tocCursor-(h-2))
	for i := start; i < len(hs) && len(out) < h; i++ {
		l := fit(strings.Repeat("  ", hs[i].Level-1)+clean(hs[i].Text), w)
		if i == m.tocCursor {
			l = selected.Render(l)
		}
		out = append(out, l)
	}
	return out
}

func (m *Model) finderLines(w, h int) []string {
	f := &m.finder
	out := []string{accent.Render("Find: ") + clean(f.query) + "▏"}
	status := fmt.Sprintf("%d of %d files", len(f.results), len(f.paths))
	switch {
	case f.err != "":
		status += " · " + warn.Render("error: "+clean(f.err))
	case f.enumerating:
		status += " · scanning…"
	case f.reason != "":
		status += " · partial: " + f.reason
	}
	out = append(out, dim.Render(status))
	rows := h - len(out)
	start := max(0, f.cursor-rows+1)
	for i := start; i < len(f.results) && len(out) < h; i++ {
		r := f.results[i]
		line := fit(highlight(f.paths[r.Index], r.Positions), w)
		if i == f.cursor {
			line = selected.Render(ansi.Strip(line))
		}
		out = append(out, line)
	}
	return out
}

// highlight marks the matched runes of a path; each rune is sanitized on
// its own because positions refer to the raw bytes.
func highlight(p string, positions []int) string {
	hit := make(map[int]bool, len(positions))
	for _, i := range positions {
		hit[i] = true
	}
	var b strings.Builder
	for i, r := range p {
		if hit[i] {
			b.WriteString(accent.Render(clean(string(r))))
		} else {
			b.WriteString(clean(string(r)))
		}
	}
	return b.String()
}

func (m *Model) helpLines(w int) []string {
	out := []string{accent.Render("Keys") + dim.Render("  (Esc close)"), ""}
	for _, kv := range keyHelp {
		out = append(out, ansi.Truncate(fmt.Sprintf("  %-20s %s", kv[0], kv[1]), w, "…"))
	}
	return out
}
