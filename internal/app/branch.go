package app

import (
	"path/filepath"
	"slices"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/git"
	"github.com/halkn/replor/internal/watch"
)

// This file reads the Branch Changes comparison after each snapshot.

type branchMsg struct {
	ticket Ticket
	rev    uint64 // the snapshot compared
	cmp    *git.Comparison
	err    error
	seed   *watch.Snapshot
}

// baseRefs are the refs the comparison depends on, watched like HEAD: a
// fetch or `git remote set-head` moves them. origin/HEAD only matters when
// no base was given.
func (m *Model) baseRefs() []string {
	gs := &m.git
	if !gs.branch {
		return nil
	}
	var refs []string
	if m.opts.Base == "" {
		refs = append(refs, git.DefaultBaseRef)
	}
	if gs.cmp != nil && gs.cmp.BaseRef != "" && !slices.Contains(refs, gs.cmp.BaseRef) {
		refs = append(refs, gs.cmp.BaseRef)
	}
	return refs
}

// loadBranch compares the current snapshot with the base off the UI loop.
// The previous comparison lends its merge base when the base and HEAD did
// not move.
func (m *Model) loadBranch() tea.Cmd {
	gs := &m.git
	ticket, ctx := m.issue("branch")
	gs.cmpInFlight = true
	ws, repo, snap, base, prev, refs := m.ws, gs.repo, gs.snap, m.opts.Base, gs.cmp, m.baseRefs()
	return func() tea.Msg {
		files := func(refs []string) []string {
			var out []string
			for _, r := range refs {
				out = append(out, filepath.Join(repo.CommonDir, filepath.FromSlash(r)))
			}
			return out
		}
		msg := branchMsg{ticket: ticket, rev: snap.Revision, seed: watch.Scan(ws, watch.Set{Git: files(refs)}, 0)}
		msg.cmp, msg.err = repo.Compare(ctx, base, snap, prev)
		// A ref resolved by this read was not seeded before it.
		if msg.cmp != nil && msg.cmp.BaseRef != "" && !slices.Contains(refs, msg.cmp.BaseRef) {
			msg.seed.Merge(watch.Unknown(files([]string{msg.cmp.BaseRef})))
		}
		return msg
	}
}

// refreshBranch is loadBranch for a new snapshot: a comparison already
// running is not cancelled; one more is queued behind it.
func (m *Model) refreshBranch() tea.Cmd {
	gs := &m.git
	switch {
	case gs.snap == nil || gs.repo == nil:
		return nil
	case gs.cmpInFlight:
		gs.cmpQueued = true
		return nil
	}
	return m.loadBranch()
}

// diffRevision is the revision a diff of t must be made from: a branch diff
// follows the comparison, the others the snapshot.
func (m *Model) diffRevision(t Target) uint64 {
	if t.Scope == git.ScopeBranch {
		return m.git.cmpRev
	}
	return m.git.snap.Revision
}

func (m *Model) applyBranch(msg branchMsg) tea.Cmd {
	if !m.latest.Accept(msg.ticket) {
		return nil
	}
	gs := &m.git
	gs.cmpInFlight = false
	if gs.phase != gitReady {
		gs.cmpQueued = false
		return nil
	}
	gs.cmp, gs.cmpErr, gs.cmpRev = msg.cmp, msg.err, msg.rev
	m.watch.seed(msg.seed)
	gs.index(gs.snap, gs.cmp)
	m.rebuildChanges()
	var cmds []tea.Cmd
	if t := m.viewer.target; t != nil && t.Mode == ModeDiff && t.Scope == git.ScopeBranch {
		m.viewer.savePos()
		cmds = append(cmds, m.load())
	}
	if gs.cmpQueued {
		gs.cmpQueued = false
		cmds = append(cmds, m.loadBranch())
	}
	return tea.Batch(cmds...)
}
