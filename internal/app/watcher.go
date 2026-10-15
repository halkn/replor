package app

import (
	"path/filepath"
	"slices"
	"time"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/watch"
)

// watchInterval is how often the watched set is scanned; a change shows up
// within about this long. A variable so that tests do not wait.
var watchInterval = time.Second

// watchMaxEntries bounds the directory entries looked at per scan; beyond
// it the header shows that watching is partial and `r` is the fallback.
var watchMaxEntries = 5000

type watchState struct {
	enabled bool
	last    *watch.Snapshot
	limited bool
}

type watchTickMsg struct{}

type watchScannedMsg struct {
	ticket Ticket
	snap   *watch.Snapshot
}

func (m *Model) scheduleWatch() tea.Cmd {
	if !m.watch.enabled {
		return nil
	}
	return tea.Tick(watchInterval, func(time.Time) tea.Msg { return watchTickMsg{} })
}

// scan stats the watched set off the UI loop. The next tick is scheduled
// only after the result, so scans never overlap.
func (m *Model) scan() tea.Cmd {
	ticket, _ := m.issue("watch")
	ws, set := m.ws, m.watchSet()
	return func() tea.Msg {
		return watchScannedMsg{ticket: ticket, snap: watch.Scan(ws, set, watchMaxEntries)}
	}
}

// watchSet is what is on screen plus git's metadata: the file in the viewer,
// the listed directories, and the index, HEAD and refs that a commit,
// checkout or `git add` touches.
func (m *Model) watchSet() watch.Set {
	var set watch.Set
	if t := m.viewer.target; t != nil {
		set.Files = []string{t.Path}
	}
	set.Dirs = []string{"."}
	for _, r := range m.tree.rows {
		if r.expanded {
			set.Dirs = append(set.Dirs, r.node.Rel)
		}
	}
	gs := &m.git
	if gs.gitDir != "" {
		set.Git = []string{
			filepath.Join(gs.gitDir, "index"),
			filepath.Join(gs.gitDir, "HEAD"),
			filepath.Join(gs.gitDir, "MERGE_HEAD"),
			filepath.Join(gs.commonDir, "packed-refs"),
		}
		if gs.snap != nil && gs.snap.Branch != "" {
			set.Git = append(set.Git, filepath.Join(gs.commonDir, "refs", "heads", filepath.FromSlash(gs.snap.Branch)))
		}
	}
	return set
}

func (m *Model) applyScan(msg watchScannedMsg) tea.Cmd {
	cmds := []tea.Cmd{m.scheduleWatch()}
	if !m.latest.Accept(msg.ticket) {
		return tea.Batch(cmds...)
	}
	ch := watch.Compare(m.watch.last, msg.snap)
	m.watch.last, m.watch.limited = msg.snap, msg.snap.Limited
	if ch.Empty() {
		return tea.Batch(cmds...)
	}
	// Any change in the work tree may change the git status.
	if m.git.repo != nil {
		cmds = append(cmds, m.loadGit())
	}
	if t := m.viewer.target; t != nil && t.Mode != ModeDiff && slices.Contains(ch.Files, t.Path) {
		m.viewer.savePos() // a diff is reloaded with the new snapshot
		cmds = append(cmds, m.load())
	}
	for _, d := range ch.Dirs {
		if d == "." || m.tree.isExpanded(d) {
			cmds = append(cmds, m.listDir(d))
		}
	}
	if len(ch.Dirs) > 0 {
		cmds = append(cmds, m.invalidateFinder())
	}
	return tea.Batch(cmds...)
}
