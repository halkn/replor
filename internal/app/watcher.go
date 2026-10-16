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

// watchState holds the baseline the next scan is compared with. Loaders
// merge in what they saw just before reading (watch.Snapshot.Merge).
type watchState struct {
	last *watch.Snapshot
}

func (w *watchState) limited() bool { return w.last != nil && w.last.Limited }

// seed records a loader's view of its paths as the baseline.
func (w *watchState) seed(s *watch.Snapshot) {
	if s == nil {
		return
	}
	if w.last == nil {
		w.last = s
		return
	}
	w.last.Merge(s)
}

type watchTickMsg struct{}

type watchScannedMsg struct {
	ticket Ticket
	snap   *watch.Snapshot
}

func (m *Model) scheduleWatch() tea.Cmd {
	if !m.opts.Watch {
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
	if r := m.git.repo; r != nil {
		set.Git = gitWatchFiles(r.GitDir, r.CommonDir, m.git.headBranch(), m.baseRefs())
	}
	return set
}

// gitWatchFiles are the metadata files a commit, checkout, reset or
// `git add` touches, and the files of refs (full names). Refs live either as
// files or, with the reftable backend, in tables listed by
// reftable/tables.list.
func gitWatchFiles(gitDir, commonDir, branch string, refs []string) []string {
	files := []string{
		filepath.Join(gitDir, "index"),
		filepath.Join(gitDir, "HEAD"),
		filepath.Join(gitDir, "MERGE_HEAD"),
		filepath.Join(commonDir, "packed-refs"),
		filepath.Join(commonDir, "reftable", "tables.list"),
	}
	if branch != "" {
		files = append(files, filepath.Join(commonDir, "refs", "heads", filepath.FromSlash(branch)))
	}
	for _, ref := range refs {
		files = append(files, filepath.Join(commonDir, filepath.FromSlash(ref)))
	}
	if len(refs) > 0 {
		// A deepening fetch extends the history a merge base is searched in.
		files = append(files, filepath.Join(commonDir, "shallow"))
	}
	return files
}

func (m *Model) applyScan(msg watchScannedMsg) tea.Cmd {
	cmds := []tea.Cmd{m.scheduleWatch()}
	if !m.latest.Accept(msg.ticket) {
		return tea.Batch(cmds...)
	}
	ch := watch.Compare(m.watch.last, msg.snap)
	m.watch.last = msg.snap
	if ch.Empty() {
		return tea.Batch(cmds...)
	}
	// Any change in the work tree may change the git status.
	cmds = append(cmds, m.refreshGit())
	if t := m.viewer.target; t != nil && t.Mode != ModeDiff && slices.Contains(ch.Files, t.Path) {
		cmds = append(cmds, m.refreshViewer()) // a diff follows the new snapshot
	}
	// Only added or removed names change a listing.
	for _, d := range ch.Listed {
		if d == "." || m.tree.isExpanded(d) {
			cmds = append(cmds, m.listDir(d))
		}
	}
	if len(ch.Listed) > 0 {
		m.finderStale()
	}
	return tea.Batch(cmds...)
}
