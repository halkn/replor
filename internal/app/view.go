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
	bar      = lipgloss.NewStyle().Background(lipgloss.Color("236")).Foreground(lipgloss.Color("252"))
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

func (m *Model) header() string {
	var b strings.Builder
	b.WriteString(" " + clean(path.Base(m.ws.Root)))
	v := &m.viewer
	if v.target != nil {
		b.WriteString("  " + clean(v.target.Path))
		if v.res.Kind == preview.KindMarkdown || v.target.Mode == ModeSource {
			pv, src := " Preview ", " Source "
			if v.target.Mode == ModeSource {
				src = tabOn.Render(src)
			} else {
				pv = tabOn.Render(pv)
			}
			b.WriteString("  " + pv + src)
		}
	}
	if v.pending {
		b.WriteString("  ⟳")
	}
	return m.line(b.String())
}

func (m *Model) footer() string {
	v := &m.viewer
	var s string
	switch {
	case m.overlay == overlaySearch:
		s = " /" + clean(m.input) + "▏"
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
	text := fit(strings.Repeat("  ", r.depth)+icon+name+suffix, w)
	switch {
	case i == m.tree.cursor && m.focus == paneFiles:
		return selected.Render(text)
	case i == m.tree.cursor:
		return dim.Render(text)
	case r.node.Kind == workspace.NodeBroken:
		return dim.Render(text)
	}
	return text
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

func (m *Model) helpLines(w int) []string {
	out := []string{accent.Render("Keys") + dim.Render("  (Esc close)"), ""}
	for _, kv := range keyHelp {
		out = append(out, ansi.Truncate(fmt.Sprintf("  %-20s %s", kv[0], kv[1]), w, "…"))
	}
	return out
}
