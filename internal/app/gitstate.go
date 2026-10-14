package app

import (
	"context"
	"errors"
	"fmt"
	"path"
	"strings"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"

	"github.com/halkn/replor/internal/git"
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

	marks   map[string]byte // workspace path -> status mark
	dirty   map[string]bool // directories containing changes
	ignored map[string]bool // untracked ignored entries; directories end with "/"
}

type gitMsg struct {
	ticket Ticket
	repo   *git.Repo
	snap   *git.Snapshot
	err    error
}

// loadGit reads a snapshot off the UI loop, locating git and the repository
// on first use.
func (m *Model) loadGit() tea.Cmd {
	ticket, ctx := m.issue("git")
	repo, root := m.git.repo, m.ws.Root
	return func() tea.Msg {
		msg := gitMsg{ticket: ticket, repo: repo}
		if repo == nil {
			msg.repo, msg.err = openRepo(ctx, root)
			if msg.err != nil {
				return msg
			}
		}
		msg.snap, msg.err = msg.repo.Snapshot(ctx)
		return msg
	}
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
		gs.phase, gs.reason = gitFailed, msg.err.Error()
		return nil
	}
	oldIgnored := gs.ignored
	gs.phase, gs.reason, gs.snap = gitReady, "", msg.snap
	gs.marks, gs.dirty, gs.ignored = map[string]byte{}, map[string]bool{}, map[string]bool{}
	for _, c := range msg.snap.Changes {
		if c.Path == "" {
			continue // the inside side of the rename is OrigPath; shown in Changes
		}
		gs.marks[c.Path] = mark(c)
		for d := path.Dir(c.Path); d != "."; d = path.Dir(d) {
			gs.dirty[d] = true
		}
	}
	for _, p := range msg.snap.Ignored {
		gs.ignored[p] = true
	}
	// The tree and the finder filter with the ignored set; redo them when
	// it changes (always on the first snapshot).
	if !m.showIgnored && !sameSet(oldIgnored, gs.ignored) {
		return tea.Batch(m.refreshTree(), m.invalidateFinder())
	}
	return nil
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

func styledMark(b byte) string {
	if st, ok := markStyles[b]; ok {
		return st.Render(string(b))
	}
	return string(b)
}

func sameSet(a, b map[string]bool) bool {
	if a == nil || len(a) != len(b) {
		return false
	}
	for k := range a {
		if !b[k] {
			return false
		}
	}
	return true
}

// isIgnored reports whether rel or one of its parent directories is an
// untracked ignored entry.
func (gs *gitState) isIgnored(rel string) bool {
	if len(gs.ignored) == 0 {
		return false
	}
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

// headerGit summarizes the repository for the header line.
func (m *Model) headerGit() string {
	gs := &m.git
	switch gs.phase {
	case gitLoading:
		return dim.Render("git …")
	case gitNone:
		return dim.Render("no git")
	case gitUnavailable, gitFailed:
		return warn.Render("git: " + clean(strings.TrimPrefix(gs.reason, "git unavailable: ")))
	}
	s := gs.snap
	var b strings.Builder
	switch {
	case s.Detached:
		fmt.Fprintf(&b, "detached %.7s", s.HeadOID)
	default:
		b.WriteString(clean(s.Branch))
		if s.HeadOID == "" {
			b.WriteString(" (no commits)")
		}
	}
	counts := map[byte]int{}
	for _, c := range s.Changes {
		counts[mark(c)]++
	}
	for _, k := range []byte("MADRCTU?") {
		if n := counts[k]; n > 0 {
			fmt.Fprintf(&b, " %s:%d", styledMark(k), n)
		}
	}
	return b.String()
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
