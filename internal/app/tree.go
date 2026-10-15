package app

import (
	"strings"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/watch"
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
	// selected is the path under the cursor. Re-listings replace rows (and
	// the ignored filter changes which exist), so the cursor is restored by
	// path, also once a subtree that holds it arrives later.
	selected string
	// expanded survives re-listings, which replace rows wholesale.
	expanded    map[string]bool
	cursor      int
	top         int
	rootLoading bool
	// defaulted is set after the first root listing, which may open the
	// README; later listings (watch, `r`) must not.
	defaulted bool
	rootErr   string
	rootCut   bool
}

type dirListedMsg struct {
	ticket    Ticket
	dir       string
	nodes     []workspace.Node
	truncated bool
	// deep re-lists the expanded directories below as well (`r`).
	deep bool
	seed *watch.Snapshot // the directory as seen before it was listed
	err  error
}

// listDir lists one directory off the UI loop. Each directory is its own
// display target, so expanding several directories does not cancel others.
func (m *Model) listDir(dir string) tea.Cmd { return m.listDirAt(dir, false) }

func (m *Model) listDirAt(dir string, deep bool) tea.Cmd {
	if dir == "." {
		m.tree.rootLoading = true
	}
	ticket, _ := m.issue("tree:" + dir)
	ws := m.ws
	return func() tea.Msg {
		seed := watch.Scan(ws, watch.Set{Dirs: []string{dir}}, watchMaxEntries)
		nodes, cut, err := ws.ListDir(dir, maxDirEntries)
		return dirListedMsg{ticket: ticket, dir: dir, nodes: nodes, truncated: cut, deep: deep, seed: seed, err: err}
	}
}

func (m *Model) applyDir(msg dirListedMsg) tea.Cmd {
	if !m.latest.Accept(msg.ticket) {
		return nil
	}
	m.watch.seed(msg.seed)
	viewing := ""
	if m.viewer.target != nil {
		viewing = m.viewer.target.Path
	}
	children := func(depth int) []treeRow {
		rows := make([]treeRow, 0, len(msg.nodes))
		for _, n := range msg.nodes {
			// The file being viewed stays reachable even when ignored
			// (DESIGN 6.2); it is marked as ignored instead.
			shown := viewing == n.Rel || strings.HasPrefix(viewing, n.Rel+"/")
			if !m.showIgnored && !shown && m.git.isIgnored(n.Rel) {
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
		rows, reload := m.tree.restoreExpanded(children(0), m.tree.rows, msg.deep)
		m.tree.rows = rows
		m.tree.restoreCursor()
		cmds := m.listDirs(reload, msg.deep)
		if !m.tree.defaulted {
			m.tree.defaulted = true
			cmds = append(cmds, m.openDefault())
		}
		return tea.Batch(cmds...)
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
	kids, reload := m.tree.restoreExpanded(children(r.depth+1), m.tree.rows[i+1:end], msg.deep)
	m.tree.rows = append(append(append([]treeRow{}, m.tree.rows[:i+1]...), kids...), m.tree.rows[end:]...)
	m.tree.restoreCursor()
	return tea.Batch(m.listDirs(reload, msg.deep)...)
}

func (m *Model) listDirs(dirs []string, deep bool) []tea.Cmd {
	cmds := make([]tea.Cmd, len(dirs))
	for i, d := range dirs {
		cmds[i] = m.listDirAt(d, deep)
	}
	return cmds
}

// restoreExpanded rebuilds a re-listed level. Directories expanded before
// keep their previous subtree rows (old) so that nothing below them
// disappears while waiting; those without one, or all of them when deep
// (`r`), are returned to be listed again.
func (t *tree) restoreExpanded(fresh, old []treeRow, deep bool) (rows []treeRow, reload []string) {
	subtree := map[string][]treeRow{}
	for i := 0; i < len(old); i++ {
		if !old[i].expanded {
			continue
		}
		j := i + 1
		for j < len(old) && old[j].depth > old[i].depth {
			j++
		}
		subtree[old[i].node.Rel] = old[i+1 : j]
		i = j - 1
	}
	for _, f := range fresh {
		if !t.expanded[f.node.Rel] || f.node.Kind != workspace.NodeDir {
			rows = append(rows, f)
			continue
		}
		kept, ok := subtree[f.node.Rel]
		f.expanded, f.loading = true, deep || !ok
		rows = append(rows, f)
		rows = append(rows, kept...)
		if deep || !ok {
			reload = append(reload, f.node.Rel)
		}
	}
	return rows, reload
}

func (t *tree) setCursor(i int) {
	t.cursor = min(max(i, 0), max(len(t.rows)-1, 0))
	if t.cursor < len(t.rows) {
		t.selected = t.rows[t.cursor].node.Rel
	}
}

// restoreCursor moves the cursor to the selected path if it is listed; if
// not (yet), the cursor only stays within the rows and selected is kept.
func (t *tree) restoreCursor() {
	if i := t.indexOf(t.selected); i >= 0 {
		t.cursor = i
		return
	}
	t.cursor = min(t.cursor, max(len(t.rows)-1, 0))
}

func (t *tree) isExpanded(rel string) bool {
	i := t.indexOf(rel)
	return i >= 0 && t.rows[i].expanded
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

// refreshTree re-lists the root and, as their parents arrive, every
// expanded directory (restoreExpanded), so results cannot apply out of
// order.
func (m *Model) refreshTree() tea.Cmd { return m.listDirAt(".", true) }

// openDefault opens README.md of the workspace root when nothing was
// selected on the command line.
func (m *Model) openDefault() tea.Cmd {
	if m.viewer.target != nil {
		return nil
	}
	for i, r := range m.tree.rows {
		if r.node.Kind == workspace.NodeFile && isReadme(r.node.Name) {
			m.tree.setCursor(i)
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
		t.setCursor(t.cursor + 1)
	case "k", "up":
		t.setCursor(t.cursor - 1)
	case "g", "home":
		t.setCursor(0)
	case "G", "end":
		t.setCursor(len(t.rows) - 1)
	case "pgdown":
		t.setCursor(t.cursor + m.bodyHeight())
	case "pgup":
		t.setCursor(t.cursor - m.bodyHeight())
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
				t.setCursor(i)
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
