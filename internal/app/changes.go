package app

import (
	"context"
	"errors"
	"fmt"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/git"
	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/workspace"
)

type leftMode int

const (
	leftFiles leftMode = iota
	leftChanges
)

// changeEntry is one row of the Changes pane. A path with staged and
// unstaged changes has two rows with separate diffs (DESIGN 7.1).
type changeEntry struct {
	section string
	scope   git.Scope
	change  git.FileChange
}

// path is the workspace path shown and opened; a file renamed out of the
// workspace is known by its original path.
func (e changeEntry) path() string {
	if e.change.Path != "" {
		return e.change.Path
	}
	return e.change.OrigPath
}

func (e changeEntry) key() string { return e.section + "\x00" + e.path() }

type changesState struct {
	entries  []changeEntry
	cursor   int
	selected string // key under the cursor, restored when the list is rebuilt
	top      int
}

const (
	sectionConflicts = "Conflicts"
	sectionStaged    = "Staged"
	sectionUnstaged  = "Unstaged"
	sectionUntracked = "Untracked"
)

// buildChanges groups a snapshot into the Changes sections.
func buildChanges(s *git.Snapshot) []changeEntry {
	var conflicts, staged, unstaged, untracked []changeEntry
	for _, c := range s.Changes {
		switch c.Kind {
		case git.ChangeUnmerged:
			conflicts = append(conflicts, changeEntry{sectionConflicts, git.ScopeUnstaged, c})
			continue
		case git.ChangeUntracked:
			untracked = append(untracked, changeEntry{sectionUntracked, git.ScopeUntracked, c})
			continue
		}
		if c.Index != '.' {
			staged = append(staged, changeEntry{sectionStaged, git.ScopeStaged, c})
		}
		if c.Worktree != '.' {
			unstaged = append(unstaged, changeEntry{sectionUnstaged, git.ScopeUnstaged, c})
		}
	}
	return append(append(append(conflicts, staged...), unstaged...), untracked...)
}

// scopesFor lists the diffs available for a workspace path, in the order
// `d` cycles through them.
func (gs *gitState) scopesFor(p string) (scopes []git.Scope, conflict bool) {
	p = nfc(p)
	return gs.scopes[p], gs.conflicts[p]
}

func findChange(s *git.Snapshot, scope git.Scope, p string) (git.FileChange, bool) {
	p = nfc(p)
	for _, e := range buildChanges(s) {
		if e.scope == scope && nfc(e.path()) == p && e.section != sectionConflicts {
			return e.change, true
		}
	}
	return git.FileChange{}, false
}

func (m *Model) rebuildChanges() {
	c := &m.changes
	c.entries = nil
	if m.git.snap != nil {
		c.entries = buildChanges(m.git.snap)
	}
	for i, e := range c.entries {
		if e.key() == c.selected {
			c.cursor = i
			return
		}
	}
	// The selected entry is gone: select what the cursor now points at, so
	// that it does not jump back if the entry reappears later.
	c.setCursor(c.cursor)
}

func (c *changesState) setCursor(i int) {
	c.cursor = min(max(i, 0), max(len(c.entries)-1, 0))
	c.selected = ""
	if c.cursor < len(c.entries) {
		c.selected = c.entries[c.cursor].key()
	}
}

func (m *Model) toggleLeft() tea.Cmd {
	if m.left == leftChanges {
		m.left = leftFiles
		return nil
	}
	switch m.git.phase {
	case gitLoading:
		m.status = "git status is loading"
		return nil
	case gitNone, gitUnavailable:
		m.status = "Changes need a git repository"
		return nil
	}
	m.left, m.focus = leftChanges, paneFiles
	m.rebuildChanges()
	if m.hideSide {
		m.hideSide = false
		return m.relayout() // the viewer just became narrower
	}
	return nil
}

func (m *Model) changesKey(s string) tea.Cmd {
	c := &m.changes
	if len(c.entries) == 0 {
		return nil
	}
	switch s {
	case "j", "down":
		c.setCursor(c.cursor + 1)
	case "k", "up":
		c.setCursor(c.cursor - 1)
	case "g", "home":
		c.setCursor(0)
	case "G", "end":
		c.setCursor(len(c.entries) - 1)
	case "pgdown":
		c.setCursor(c.cursor + m.bodyHeight())
	case "pgup":
		c.setCursor(c.cursor - m.bodyHeight())
	case "enter":
		return m.openChange(c.entries[c.cursor])
	}
	return nil
}

// openChange shows the diff of an entry. A conflict has no two-way diff in
// v1 (DESIGN 7.1): the work tree file is shown instead. A file renamed out
// of the workspace has its content outside, which is not shown (6.1).
func (m *Model) openChange(e changeEntry) tea.Cmd {
	switch {
	case e.section == sectionConflicts:
		m.status = "unresolved conflict: showing the work tree file"
		return m.open(Target{Path: e.path(), Mode: sourceModeFor(e.path())}, preview.Position{}, nil, true)
	case e.change.Path == "":
		m.status = clean(e.path()) + " was moved outside the workspace; its content is not shown"
		return nil
	}
	return m.open(Target{Path: e.path(), Mode: ModeDiff, Scope: e.scope}, preview.Position{}, nil, true)
}

func sourceModeFor(p string) Mode {
	if preview.IsMarkdown(p) {
		return ModeSource
	}
	return ModePreview
}

// toggleDiff cycles a file through its diffs and back to Preview.
func (m *Model) toggleDiff() tea.Cmd {
	v := &m.viewer
	if v.target == nil {
		return nil
	}
	t := *v.target
	scopes, conflict := m.git.scopesFor(t.Path)
	switch {
	case conflict && t.Mode == ModeDiff:
		m.status = "unresolved conflict: showing the work tree file"
		return m.open(Target{Path: t.Path, Mode: sourceModeFor(t.Path)}, preview.Position{}, nil, false)
	case conflict:
		m.status = "unresolved conflict: no diff in this version"
		return nil
	case len(scopes) == 0 && t.Mode != ModeDiff:
		m.status = "no changes in " + clean(t.Path)
		return nil
	}
	next := 0
	if t.Mode == ModeDiff {
		next = len(scopes) // back to Preview unless the scope is found
		for i, s := range scopes {
			if s == t.Scope {
				next = i + 1
			}
		}
	}
	if next >= len(scopes) {
		return m.open(Target{Path: t.Path, Mode: ModePreview}, preview.Position{}, nil, false)
	}
	return m.open(Target{Path: t.Path, Mode: ModeDiff, Scope: scopes[next]}, preview.Position{}, nil, false)
}

// errNoLongerChanged is shown when a diff target left the snapshot.
var errNoLongerChanged = errors.New("no longer changed")

// loadDiff produces the diff of a target against the snapshot current when
// requested; the result carries its revision so that a diff from an older
// snapshot is not shown next to a newer list.
func loadDiff(ctx context.Context, ws *workspace.Workspace, repo *git.Repo, snap *git.Snapshot, t Target, width int) (preview.Result, uint64, error) {
	if snap == nil || repo == nil {
		return preview.Result{}, 0, errors.New("git status is not available")
	}
	c, ok := findChange(snap, t.Scope, t.Path)
	if !ok {
		return preview.Result{}, snap.Revision, fmt.Errorf("%s: %w (%v)", t.Path, errNoLongerChanged, t.Scope)
	}
	var text []byte
	var err error
	if t.Scope == git.ScopeUntracked {
		text, err = untrackedDiff(ws, c.Path)
	} else {
		// The same limit as reading a file for Source (DESIGN 6.1, 9.1).
		text, err = repo.Diff(ctx, t.Scope, c, preview.MaxSource)
	}
	if err != nil {
		return preview.Result{}, snap.Revision, err
	}
	r := preview.DiffLines(string(text), width)
	return preview.Result{Kind: preview.KindDiff, Size: len(text), Rendered: r}, snap.Revision, nil
}

// untrackedDiff shows an untracked entry the way git would: a symbolic link
// as its target, a directory (a nested repository) as such.
func untrackedDiff(ws *workspace.Workspace, p string) ([]byte, error) {
	if target, ok, err := ws.Readlink(p); err != nil {
		return nil, err
	} else if ok {
		return preview.UntrackedLinkDiff(p, target), nil
	}
	info, err := ws.Stat(p)
	if err != nil {
		return nil, err
	}
	if info.IsDir() {
		return []byte(fmt.Sprintf("diff --git a/%s b/%s\nuntracked directory (a nested repository); its files are not listed\n", p, p)), nil
	}
	data, err := ws.ReadFile(p, preview.MaxSource)
	if err != nil {
		return nil, err
	}
	return preview.UntrackedDiff(p, data), nil
}
