package app

import (
	"context"
	"errors"
	"fmt"
	"path"
	"slices"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"golang.org/x/text/unicode/norm"

	"github.com/halkn/replor/internal/git"
	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/watch"
)

type gitPhase int

const (
	gitLoading gitPhase = iota
	gitReady
	gitNone        // not a repository: file browsing only
	gitUnavailable // git missing, too old, refused (reason)
	gitFailed      // a command failed (reason)
)

type gitState struct {
	phase  gitPhase
	reason string
	repo   *git.Repo
	snap   *git.Snapshot
	// inFlight is set while a read runs; queued asks for one more read when
	// it completes (refreshGit), so that a stream of changes cannot cancel
	// every read before it finishes.
	inFlight, queued bool

	// The maps are keyed by NFC paths: macOS can return decomposed names
	// from the file system while git prints composed ones.
	marks   map[string]byte // workspace path -> status mark
	dirty   map[string]bool // directories containing changes
	ignored map[string]bool // untracked ignored entries; directories end with "/"
	counts  map[byte]int    // changes per mark, for the header
	// scopes lists the diffs of each path in the order `d` cycles them;
	// conflicts holds unmerged paths. Both are derived once per snapshot.
	scopes    map[string][]git.Scope
	conflicts map[string]bool

	// branch is set once Branch Changes are asked for; from then on every
	// snapshot is followed by a comparison with the base (cmp, or cmpErr
	// when that failed), read separately so that the snapshot does not wait
	// for it. cmpRev is the snapshot revision cmp was built from. The
	// comparison is not built before: it walks the history.
	branch bool
	cmp    *git.Comparison
	cmpErr error
	cmpRev uint64
	// As inFlight and queued, for the comparison.
	cmpInFlight, cmpQueued bool
}

// index derives the per-path lookups from a snapshot and its comparison.
func (gs *gitState) index(s *git.Snapshot, cmp *git.Comparison) {
	gs.marks, gs.dirty, gs.counts = map[string]byte{}, map[string]bool{}, map[byte]int{}
	gs.scopes, gs.conflicts = map[string][]git.Scope{}, map[string]bool{}
	markDirs := func(p string) {
		for d := path.Dir(nfc(p)); d != "." && d != "/"; d = path.Dir(d) {
			gs.dirty[d] = true
		}
	}
	for _, c := range s.Changes {
		gs.counts[mark(c)]++
		if c.OrigPath != "" {
			markDirs(c.OrigPath) // a file left this directory
		}
		if c.Path == "" {
			continue // renamed out of the workspace; listed in Changes
		}
		gs.marks[nfc(c.Path)] = mark(c)
		markDirs(c.Path)
	}
	for _, e := range buildChanges(s) {
		p := nfc(e.path())
		if e.section == sectionConflicts {
			gs.conflicts[p] = true
		} else if e.change.Path != "" {
			gs.scopes[p] = append(gs.scopes[p], e.scope)
		}
	}
	// An untracked file already has its untracked diff.
	for _, e := range buildBranch(cmp) {
		if e.scope == git.ScopeBranch && e.change.Path != "" {
			p := nfc(e.path())
			gs.scopes[p] = append(gs.scopes[p], e.scope)
		}
	}
	// Unstaged first: it is what the reader is editing.
	for _, sc := range gs.scopes {
		slices.SortFunc(sc, func(a, b git.Scope) int { return scopeRank[a] - scopeRank[b] })
	}
}

var scopeRank = map[git.Scope]int{git.ScopeUnstaged: 0, git.ScopeStaged: 1, git.ScopeUntracked: 2, git.ScopeBranch: 3}

func nfc(s string) string { return norm.NFC.String(s) }

func (gs *gitState) markOf(rel string) byte { return gs.marks[nfc(rel)] }

func (gs *gitState) isDirty(rel string) bool { return gs.dirty[nfc(rel)] }

type gitMsg struct {
	ticket Ticket
	repo   *git.Repo
	snap   *git.Snapshot
	seed   *watch.Snapshot // the metadata files as seen before the read
	err    error
}

// loadGit reads a snapshot off the UI loop, locating git and the repository
// on first use.
func (m *Model) loadGit() tea.Cmd {
	ticket, ctx := m.issue("git")
	m.git.inFlight = true
	repo, root, ws := m.git.repo, m.ws.Root, m.ws
	branch, refs := m.git.headBranch(), m.baseRefs()
	return func() tea.Msg {
		msg := gitMsg{ticket: ticket, repo: repo}
		if repo == nil {
			msg.repo, msg.err = openRepo(ctx, root)
			if msg.err != nil {
				return msg
			}
		}
		msg.seed = watch.Scan(ws, watch.Set{Git: gitWatchFiles(msg.repo.GitDir, msg.repo.CommonDir, branch, refs)}, 0)
		msg.snap, msg.err = msg.repo.Snapshot(ctx)
		return msg
	}
}

func (gs *gitState) headBranch() string {
	if gs.snap == nil {
		return ""
	}
	return gs.snap.Branch
}

// refreshGit is loadGit for watch events: a read already running is not
// cancelled; one more is queued behind it.
func (m *Model) refreshGit() tea.Cmd {
	switch {
	case m.git.repo == nil:
		return nil
	case m.git.inFlight:
		m.git.queued = true
		return nil
	}
	return m.loadGit()
}

func openRepo(ctx context.Context, root string) (*git.Repo, error) {
	g, err := git.Detect(ctx)
	if err != nil {
		return nil, err
	}
	return g.Open(ctx, root)
}

func (m *Model) applyGit(msg gitMsg) tea.Cmd {
	if !m.latest.Accept(msg.ticket) {
		return nil
	}
	m.git.inFlight = false
	cmd := m.applyGitResult(msg)
	if m.git.queued && m.git.repo != nil {
		m.git.queued = false
		cmd = tea.Batch(cmd, m.loadGit())
	}
	return cmd
}

func (m *Model) applyGitResult(msg gitMsg) tea.Cmd {
	gs := &m.git
	gs.repo = msg.repo
	switch {
	case errors.Is(msg.err, git.ErrNotRepository):
		*gs = gitState{phase: gitNone}
		return nil
	case errors.Is(msg.err, git.ErrUnavailable):
		*gs = gitState{phase: gitUnavailable, reason: msg.err.Error()}
		return nil
	case msg.err != nil:
		// Marks from an older snapshot would contradict the error. The
		// ignored set is kept: it only filters, and dropping it would flood
		// the tree with build output on a transient failure.
		gs.phase, gs.reason, gs.snap, gs.cmp, gs.cmpErr, gs.cmpRev = gitFailed, msg.err.Error(), nil, nil, nil, 0
		gs.marks, gs.dirty, gs.counts, gs.scopes, gs.conflicts = nil, nil, nil, nil, nil
		m.rebuildChanges()
		// A shown diff belonged to the snapshot that is gone (DESIGN 7.2).
		if t := m.viewer.target; t != nil && t.Mode == ModeDiff {
			v := &m.viewer
			v.res, v.err = preview.Result{}, fmt.Errorf("git status failed: %s", gs.reason)
			return tea.Sequence(m.dropImages()...)
		}
		return nil
	}
	oldIgnored := gs.ignored
	gs.phase, gs.reason, gs.snap = gitReady, "", msg.snap
	m.watch.seed(msg.seed)
	gs.index(msg.snap, gs.cmp)
	if msg.snap.IgnoredErr == nil {
		gs.ignored = map[string]bool{}
		for _, p := range msg.snap.Ignored {
			gs.ignored[nfc(p)] = true
		}
	}
	m.rebuildChanges()
	var cmds []tea.Cmd
	// A shown diff belongs to the previous snapshot; a branch diff to the
	// previous comparison, reloaded when the next one arrives.
	if t := m.viewer.target; t != nil && t.Mode == ModeDiff && t.Scope != git.ScopeBranch {
		m.viewer.savePos()
		cmds = append(cmds, m.load())
	}
	if gs.branch {
		cmds = append(cmds, m.refreshBranch())
	}
	// The tree and the finder filter with the ignored set; redo them when
	// it changes.
	if !m.showIgnored && !sameSet(oldIgnored, gs.ignored) {
		cmds = append(cmds, m.refreshTree(), m.invalidateFinder())
	}
	return tea.Batch(cmds...)
}

// mark is the letter shown next to a changed file: the work tree state when
// it differs, otherwise the staged one.
func mark(c git.FileChange) byte {
	switch c.Kind {
	case git.ChangeUnmerged:
		return 'U'
	case git.ChangeUntracked:
		return '?'
	}
	if c.Worktree != '.' {
		return c.Worktree
	}
	return c.Index
}

var markStyles = map[byte]lipgloss.Style{
	'M': lipgloss.NewStyle().Foreground(lipgloss.Color("214")),
	'T': lipgloss.NewStyle().Foreground(lipgloss.Color("214")),
	'A': lipgloss.NewStyle().Foreground(lipgloss.Color("78")),
	'?': lipgloss.NewStyle().Foreground(lipgloss.Color("78")),
	'D': lipgloss.NewStyle().Foreground(lipgloss.Color("203")),
	'R': lipgloss.NewStyle().Foreground(lipgloss.Color("75")),
	'C': lipgloss.NewStyle().Foreground(lipgloss.Color("75")),
	'U': lipgloss.NewStyle().Foreground(lipgloss.Color("170")).Bold(true),
}

func markStyle(b byte) lipgloss.Style {
	if st, ok := markStyles[b]; ok {
		return st
	}
	return lipgloss.NewStyle()
}

func styledMark(b byte) string { return markStyle(b).Render(string(b)) }

// sameSet compares sets; a missing set is empty.
func sameSet(a, b map[string]bool) bool {
	if len(a) != len(b) {
		return false
	}
	for k := range a {
		if !b[k] {
			return false
		}
	}
	return true
}

// movedTo returns the new workspace path of a file renamed away from p, or
// "" (also when it moved outside the workspace).
func (gs *gitState) movedTo(p string) string {
	if gs.snap == nil {
		return ""
	}
	p = nfc(p)
	for _, c := range gs.snap.Changes {
		if c.OrigPath != "" && nfc(c.OrigPath) == p && c.Kind == git.ChangeRenamed {
			return c.Path
		}
	}
	return ""
}

// isIgnored reports whether rel or one of its parent directories is an
// untracked ignored entry.
func (gs *gitState) isIgnored(rel string) bool {
	if len(gs.ignored) == 0 {
		return false
	}
	rel = nfc(rel)
	if gs.ignored[rel] || gs.ignored[rel+"/"] {
		return true
	}
	for d := path.Dir(rel); d != "."; d = path.Dir(d) {
		if gs.ignored[d+"/"] {
			return true
		}
	}
	return false
}

// headerGit summarizes the repository as header parts styled on the bar.
func (m *Model) headerGit() []string {
	gs := &m.git
	switch gs.phase {
	case gitLoading:
		return []string{onBar(dim).Render("git …")}
	case gitNone:
		return []string{onBar(dim).Render("no git")}
	case gitUnavailable, gitFailed:
		return []string{onBar(warn).Render("git: " + clean(strings.TrimPrefix(gs.reason, "git unavailable: ")))}
	}
	s := gs.snap
	head := clean(s.Branch)
	switch {
	case s.Detached:
		head = fmt.Sprintf("detached %.7s", s.HeadOID)
	case s.HeadOID == "":
		head += " (no commits)"
	}
	parts := []string{bar.Render(head)}
	for _, k := range []byte("MADRCTU?") {
		if n := gs.counts[k]; n > 0 {
			parts = append(parts, bar.Render(" "), onBar(markStyle(k)).Render(string(k)), bar.Render(fmt.Sprintf(":%d", n)))
		}
	}
	return parts
}

func (m *Model) toggleIgnored() tea.Cmd {
	m.showIgnored = !m.showIgnored
	if m.showIgnored {
		m.status = "showing ignored files"
	} else {
		m.status = "hiding ignored files"
	}
	return tea.Batch(m.refreshTree(), m.invalidateFinder())
}
