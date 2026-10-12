package app

import (
	"unicode/utf8"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/preview"
)

// keyHelp is shown by `?` and must match the bindings in key.
var keyHelp = [][2]string{
	{"Tab", "switch focus between the file tree and the viewer"},
	{"j / k, ↓ / ↑", "move in the tree / scroll the viewer"},
	{"PgDn / PgUp, g / G", "page / jump to the start or end"},
	{"h / l, ← / →", "collapse / expand a directory (h moves to the parent)"},
	{"Enter", "open the selected file or directory, follow the selected link"},
	{"f", "find a file by name (type to filter, ↑/↓ to choose, Enter to open)"},
	{"> / <", "select the next / previous link in the document"},
	{"Backspace / ]", "go back / forward in history"},
	{"/", "search in the document (Enter to search, Esc to cancel)"},
	{"n / N", "next / previous search result"},
	{"t", "table of contents"},
	{"s", "switch a Markdown file between Preview and Source"},
	{"e", "show / hide the file tree"},
	{"r", "reload the tree and the document"},
	{"Esc", "close a panel, clear the link selection and search"},
	{"?", "this help"},
	{"q", "quit (closes a panel first)"},
}

func (m *Model) key(k tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	s := k.String()
	if s == "ctrl+c" {
		return m, m.quit()
	}
	m.status = ""
	switch m.overlay {
	case overlaySearch:
		m.searchKey(k)
		return m, nil
	case overlayTOC:
		return m, m.tocKey(s)
	case overlayFinder:
		return m, m.finderKey(k)
	case overlayHelp:
		if s == "esc" || s == "q" || s == "?" {
			m.overlay = overlayNone
		}
		return m, nil
	}

	v := &m.viewer
	switch s {
	case "q":
		return m, m.quit()
	case "esc":
		v.linkIdx, v.matches, v.query, v.focus = -1, nil, "", nil
	case "tab":
		if m.focus == paneFiles || !m.sideVisible() {
			m.focus = paneViewer
		} else {
			m.focus = paneFiles
		}
	case "e":
		m.hideSide = !m.hideSide
		if !m.sideVisible() {
			m.focus = paneViewer
		}
		v.savePos()
		return m, m.load()
	case "?":
		m.overlay = overlayHelp
	case "f":
		return m, m.openFinder()
	case "/":
		if v.rendered() == nil {
			m.status = "nothing to search"
			return m, nil
		}
		m.overlay, m.input = overlaySearch, ""
	case "n":
		m.nextMatch(1)
	case "N":
		m.nextMatch(-1)
	case "t":
		if d := v.doc(); d != nil && len(d.Headings) > 0 {
			m.overlay, m.tocCursor = overlayTOC, m.currentHeading()
		} else {
			m.status = "no headings in this view"
		}
	case "s":
		return m, m.toggleSource()
	case ">":
		m.selectLink(1)
	case "<":
		m.selectLink(-1)
	case "backspace":
		return m, m.goBack()
	case "]":
		return m, m.goForward()
	case "r":
		return m, m.reload()
	default:
		if m.focus == paneFiles && m.sideVisible() {
			return m, m.treeKey(s)
		}
		return m, m.viewerKey(s)
	}
	return m, nil
}

func (m *Model) viewerKey(s string) tea.Cmd {
	h := m.bodyHeight()
	switch s {
	case "j", "down":
		m.scroll(1)
	case "k", "up":
		m.scroll(-1)
	case "pgdown", "space":
		m.scroll(h)
	case "pgup":
		m.scroll(-h)
	case "g", "home":
		m.scroll(-1 << 30)
	case "G", "end":
		m.scroll(1 << 30)
	case "enter":
		if m.viewer.linkIdx >= 0 && m.viewer.doc() != nil {
			return m.followLink()
		}
	}
	return nil
}

func (m *Model) searchKey(k tea.KeyPressMsg) {
	switch k.String() {
	case "enter":
		m.overlay = overlayNone
		m.search(m.input)
	case "esc":
		m.overlay = overlayNone
	case "backspace":
		_, size := utf8.DecodeLastRuneInString(m.input)
		m.input = m.input[:len(m.input)-size]
	default:
		// Text is what the key types; control keys have none.
		m.input += k.Text
	}
}

func (m *Model) tocKey(s string) tea.Cmd {
	hs := m.viewer.doc().Headings
	switch s {
	case "j", "down":
		m.tocCursor = min(m.tocCursor+1, len(hs)-1)
	case "k", "up":
		m.tocCursor = max(m.tocCursor-1, 0)
	case "g", "home":
		m.tocCursor = 0
	case "G", "end":
		m.tocCursor = len(hs) - 1
	case "enter":
		m.overlay = overlayNone
		m.jumpInDoc(preview.Position{Block: hs[m.tocCursor].Block})
	case "esc", "q", "t":
		m.overlay = overlayNone
	}
	return nil
}

// currentHeading is the last heading at or above the top of the view, to
// preselect in the TOC.
func (m *Model) currentHeading() int {
	v := &m.viewer
	block := v.rendered().BlockAt(v.top)
	cur := 0
	for i, h := range v.doc().Headings {
		if h.Block <= block {
			cur = i
		}
	}
	return cur
}

func (m *Model) quit() tea.Cmd {
	return tea.Sequence(append(m.dropImages(), tea.Quit)...)
}
