package app

import (
	"context"
	"errors"
	"fmt"
	"slices"

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
func scopesFor(s *git.Snapshot, p string) (scopes []git.Scope, conflict bool) {
	if s == nil {
		return nil, false
	}
	for _, e := range buildChanges(s) {
		if e.path() != p {
			continue
		}
		if e.section == sectionConflicts {
			return nil, true
		}
		scopes = append(scopes, e.scope)
	}
	// Unstaged first: it is what the reader is editing.
	rank := map[git.Scope]int{git.ScopeUnstaged: 0, git.ScopeStaged: 1, git.ScopeUntracked: 2}
	slices.SortFunc(scopes, func(a, b git.Scope) int { return rank[a] - rank[b] })
	return scopes, false
}

func findChange(s *git.Snapshot, scope git.Scope, p string) (git.FileChange, bool) {
	for _, e := range buildChanges(s) {
		if e.scope == scope && e.path() == p && e.section != sectionConflicts {
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
	c.cursor = min(c.cursor, max(len(c.entries)-1, 0))
	for i, e := range c.entries {
		if e.key() == c.selected {
			c.cursor = i
		}
	}
}

func (c *changesState) setCursor(i int) {
	c.cursor = min(max(i, 0), max(len(c.entries)-1, 0))
	if c.cursor < len(c.entries) {
		c.selected = c.entries[c.cursor].key()
	}
}

func (m *Model) toggleLeft() {
	if m.left == leftChanges {
		m.left = leftFiles
		return
	}
	if m.git.phase != gitReady {
		m.status = "Changes need a git repository"
		return
	}
	m.left, m.hideSide, m.focus = leftChanges, false, paneFiles
	m.rebuildChanges()
	if c := &m.changes; c.selected == "" && len(c.entries) > 0 {
		c.setCursor(0)
	}
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
// v1 (DESIGN 7.1): the work tree file is shown instead.
func (m *Model) openChange(e changeEntry) tea.Cmd {
	if e.section == sectionConflicts {
		m.status = "unresolved conflict: showing the work tree file"
		return m.open(Target{Path: e.path(), Mode: sourceModeFor(e.path())}, preview.Position{}, nil, true)
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
	scopes, conflict := scopesFor(m.git.snap, t.Path)
	switch {
	case conflict:
		m.status = "unresolved conflict: no diff in this version"
		return nil
	case len(scopes) == 0 && t.Mode != ModeDiff:
		m.status = "no changes in " + clean(t.Path)
		return nil
	}
	next := -1
	if t.Mode == ModeDiff {
		for i, s := range scopes {
			if s == t.Scope {
				next = i + 1
			}
		}
	} else {
		next = 0
	}
	if next < 0 || next >= len(scopes) {
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
		var data []byte
		if data, err = ws.ReadFile(c.Path, preview.MaxSource); err == nil {
			text = preview.UntrackedDiff(c.Path, data)
		}
	} else {
		text, err = repo.Diff(ctx, t.Scope, c)
	}
	if err != nil {
		return preview.Result{}, snap.Revision, err
	}
	r := preview.DiffLines(string(text), width)
	return preview.Result{Kind: preview.KindDiff, Size: len(text), Rendered: r}, snap.Revision, nil
}
