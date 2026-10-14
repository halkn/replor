package app

import (
	"strings"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/workspace"
)

// maxDirEntries caps one directory listing so that a huge directory cannot
// stall the tree; the cut is shown on the directory row.
const maxDirEntries = 20_000

type treeRow struct {
	node      workspace.Node
	depth     int
	expanded  bool
	loading   bool
	truncated bool
	err       string
}

type tree struct {
	rows []treeRow
	// expanded survives re-listings, which replace rows wholesale.
	expanded    map[string]bool
	cursor      int
	top         int
	rootLoading bool
	rootErr     string
	rootCut     bool
}

type dirListedMsg struct {
	ticket    Ticket
	dir       string
	nodes     []workspace.Node
	truncated bool
	err       error
}

// listDir lists one directory off the UI loop. Each directory is its own
// display target, so expanding several directories does not cancel others.
func (m *Model) listDir(dir string) tea.Cmd {
	if dir == "." {
		m.tree.rootLoading = true
	}
	ticket, _ := m.issue("tree:" + dir)
	ws := m.ws
	return func() tea.Msg {
		nodes, cut, err := ws.ListDir(dir, maxDirEntries)
		return dirListedMsg{ticket: ticket, dir: dir, nodes: nodes, truncated: cut, err: err}
	}
}

func (m *Model) applyDir(msg dirListedMsg) tea.Cmd {
	if !m.latest.Accept(msg.ticket) {
		return nil
	}
	children := func(depth int) []treeRow {
		rows := make([]treeRow, 0, len(msg.nodes))
		for _, n := range msg.nodes {
			if !m.showIgnored && m.git.isIgnored(n.Rel) {
				continue
			}
			rows = append(rows, treeRow{node: n, depth: depth})
		}
		return rows
	}
	if msg.dir == "." {
		m.tree.rootLoading = false
		if msg.err != nil {
			m.tree.rootErr = msg.err.Error()
			return nil
		}
		m.tree.rootErr, m.tree.rootCut = "", msg.truncated
		rows, reload := m.tree.restoreExpanded(children(0))
		m.tree.rows = rows
		m.tree.cursor = min(m.tree.cursor, max(len(m.tree.rows)-1, 0))
		return tea.Batch(append(m.listDirs(reload), m.openDefault())...)
	}
	i := m.tree.indexOf(msg.dir)
	if i < 0 || !m.tree.rows[i].expanded {
		return nil // collapsed or gone while loading
	}
	r := &m.tree.rows[i]
	r.loading = false
	if msg.err != nil {
		r.err = msg.err.Error()
		return nil
	}
	r.err, r.truncated = "", msg.truncated
	end := m.tree.subtreeEnd(i)
	kids, reload := m.tree.restoreExpanded(children(r.depth + 1))
	m.tree.rows = append(append(append([]treeRow{}, m.tree.rows[:i+1]...), kids...), m.tree.rows[end:]...)
	return tea.Batch(m.listDirs(reload)...)
}

func (m *Model) listDirs(dirs []string) []tea.Cmd {
	cmds := make([]tea.Cmd, len(dirs))
	for i, d := range dirs {
		cmds[i] = m.listDir(d)
	}
	return cmds
}

// restoreExpanded marks freshly listed directories that were expanded before
// a re-listing (e.g. after `r`) and returns them to be listed again; nested
// ones follow level by level as their parents arrive.
func (t *tree) restoreExpanded(fresh []treeRow) (rows []treeRow, reload []string) {
	for i := range fresh {
		if t.expanded[fresh[i].node.Rel] && fresh[i].node.Kind == workspace.NodeDir {
			fresh[i].expanded, fresh[i].loading = true, true
			reload = append(reload, fresh[i].node.Rel)
		}
	}
	return fresh, reload
}

func (t *tree) indexOf(rel string) int {
	for i, r := range t.rows {
		if r.node.Rel == rel {
			return i
		}
	}
	return -1
}

// subtreeEnd returns the index just past the descendants of row i.
func (t *tree) subtreeEnd(i int) int {
	j := i + 1
	for j < len(t.rows) && t.rows[j].depth > t.rows[i].depth {
		j++
	}
	return j
}

func (m *Model) toggleDir(i int) tea.Cmd {
	r := &m.tree.rows[i]
	if r.node.Kind != workspace.NodeDir {
		return nil
	}
	if r.expanded {
		end := m.tree.subtreeEnd(i)
		m.tree.rows = append(m.tree.rows[:i+1], m.tree.rows[end:]...)
		r.expanded, r.loading = false, false
		delete(m.tree.expanded, r.node.Rel)
		return nil
	}
	r.expanded, r.loading = true, true
	if m.tree.expanded == nil {
		m.tree.expanded = map[string]bool{}
	}
	m.tree.expanded[r.node.Rel] = true
	return m.listDir(r.node.Rel)
}

// refreshTree re-lists the root; expanded directories are re-listed as
// their parents arrive (restoreExpanded), so results cannot apply out of
// order.
func (m *Model) refreshTree() tea.Cmd { return m.listDir(".") }

// openDefault opens README.md of the workspace root when nothing was
// selected on the command line.
func (m *Model) openDefault() tea.Cmd {
	if m.viewer.target != nil {
		return nil
	}
	for i, r := range m.tree.rows {
		if r.node.Kind == workspace.NodeFile && isReadme(r.node.Name) {
			m.tree.cursor = i
			return m.open(Target{Path: r.node.Rel}, preview.Position{}, nil, false)
		}
	}
	return nil
}

func isReadme(name string) bool {
	n := strings.ToLower(name)
	return n == "readme.md" || n == "readme.markdown"
}

func (m *Model) treeKey(s string) tea.Cmd {
	t := &m.tree
	if len(t.rows) == 0 {
		return nil
	}
	switch s {
	case "j", "down":
		t.cursor = min(t.cursor+1, len(t.rows)-1)
	case "k", "up":
		t.cursor = max(t.cursor-1, 0)
	case "g", "home":
		t.cursor = 0
	case "G", "end":
		t.cursor = len(t.rows) - 1
	case "pgdown":
		t.cursor = min(t.cursor+m.bodyHeight(), len(t.rows)-1)
	case "pgup":
		t.cursor = max(t.cursor-m.bodyHeight(), 0)
	case "l", "right":
		if r := t.rows[t.cursor]; r.node.Kind == workspace.NodeDir && !r.expanded {
			return m.toggleDir(t.cursor)
		}
	case "h", "left":
		r := t.rows[t.cursor]
		if r.expanded {
			return m.toggleDir(t.cursor)
		}
		// Move to the parent directory row.
		for i := t.cursor - 1; i >= 0; i-- {
			if t.rows[i].depth < r.depth {
				t.cursor = i
				break
			}
		}
	case "enter":
		r := t.rows[t.cursor]
		switch r.node.Kind {
		case workspace.NodeDir:
			return m.toggleDir(t.cursor)
		case workspace.NodeBroken:
			m.status = r.node.Name + ": " + r.node.Reason
		default:
			return m.open(Target{Path: r.node.Rel}, preview.Position{}, nil, true)
		}
	}
	return nil
}
