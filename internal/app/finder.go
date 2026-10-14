package app

import (
	"path"
	"unicode/utf8"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/fuzzy"
	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/workspace"
)

// finderLimits bound the path enumeration (DESIGN 6.2, ADR-0001 3.5).
var finderLimits = workspace.Limits{MaxEntries: 200_000, MaxDepth: 64}

// maxFinderResults is how many ranked paths are kept; the list shows a
// window of them.
const maxFinderResults = 500

// finder holds the workspace path list and the current ranking. The list
// survives closing the finder; `r` discards it.
type finder struct {
	paths       []string
	enumerating bool
	reason      string // workspace.LimitedEntries etc. when partial
	err         string

	query   string
	results []fuzzy.Result
	cursor  int
}

type finderBatchMsg struct {
	ticket Ticket
	paths  []string
	next   tea.Cmd // reads the following batch; nil when done
	reason string
	err    error
}

type finderRankedMsg struct {
	ticket  Ticket
	query   string
	results []fuzzy.Result
}

// openFinder shows the finder, starting the enumeration on first use.
func (m *Model) openFinder() tea.Cmd {
	m.overlay = overlayFinder
	f := &m.finder
	f.cursor = 0
	if f.paths == nil && !f.enumerating {
		return tea.Batch(m.enumerate(), m.rank())
	}
	return m.rank()
}

// enumerate walks the workspace in the background and streams batches as
// messages, so the finder can show results while the walk continues.
func (m *Model) enumerate() tea.Cmd {
	ticket, ctx := m.issue("finder")
	m.finder = finder{enumerating: true, paths: []string{}, query: m.finder.query}
	ch := make(chan finderBatchMsg, 4)
	ws := m.ws
	go func() {
		defer close(ch)
		reason, err := ws.Enumerate(ctx, finderLimits, func(rel string) bool { return path.Base(rel) == ".git" }, func(b []string) {
			select {
			case ch <- finderBatchMsg{ticket: ticket, paths: b}:
			case <-ctx.Done():
			}
		})
		if ctx.Err() == nil {
			ch <- finderBatchMsg{ticket: ticket, reason: reason, err: err}
		}
	}()
	var next tea.Cmd
	next = func() tea.Msg {
		msg, ok := <-ch
		if !ok {
			return nil
		}
		if msg.err == nil && msg.paths != nil {
			msg.next = next
		}
		return msg
	}
	return next
}

func (m *Model) applyFinderBatch(msg finderBatchMsg) tea.Cmd {
	// The ticket stays current until the next enumeration (Accept would
	// close it after the first batch).
	if !m.latest.Current(msg.ticket) {
		return nil
	}
	f := &m.finder
	f.paths = append(f.paths, msg.paths...)
	if msg.next == nil {
		f.enumerating = false
		f.reason = msg.reason
		if msg.err != nil {
			f.err = msg.err.Error()
		}
		m.latest.Accept(msg.ticket)
	}
	cmds := []tea.Cmd{msg.next}
	// Re-rank as the list grows, but not on every batch of a large walk.
	if m.overlay == overlayFinder && (msg.next == nil || len(f.paths) < 5000 || len(f.paths)%(16*512) < len(msg.paths)) {
		cmds = append(cmds, m.rank())
	}
	return tea.Batch(cmds...)
}

// rank re-ranks the current path list for the query off the UI loop.
func (m *Model) rank() tea.Cmd {
	ticket, ctx := m.issue("finder:rank")
	query, paths := m.finder.query, m.finder.paths
	return func() tea.Msg {
		res := fuzzy.Rank(ctx, query, paths, maxFinderResults)
		return finderRankedMsg{ticket: ticket, query: query, results: res}
	}
}

func (m *Model) applyRanked(msg finderRankedMsg) {
	if !m.latest.Accept(msg.ticket) {
		return
	}
	f := &m.finder
	f.results = msg.results
	f.cursor = min(f.cursor, max(len(f.results)-1, 0))
}

func (m *Model) finderKey(k tea.KeyPressMsg) tea.Cmd {
	f := &m.finder
	switch k.String() {
	case "esc":
		m.overlay = overlayNone
	case "enter":
		if len(f.results) == 0 {
			return nil
		}
		p := f.paths[f.results[f.cursor].Index]
		m.overlay = overlayNone
		return m.open(Target{Path: p}, preview.Position{}, nil, true)
	case "down", "ctrl+n":
		f.cursor = min(f.cursor+1, max(len(f.results)-1, 0))
	case "up", "ctrl+p":
		f.cursor = max(f.cursor-1, 0)
	case "backspace":
		if f.query == "" {
			return nil
		}
		_, size := utf8.DecodeLastRuneInString(f.query)
		f.query = f.query[:len(f.query)-size]
		f.cursor = 0
		return m.rank()
	default:
		if k.Text == "" {
			return nil
		}
		f.query += k.Text
		f.cursor = 0
		return m.rank()
	}
	return nil
}

// invalidateFinder drops the path list (after `r`); an open finder starts
// a new enumeration, cancelling the old one.
func (m *Model) invalidateFinder() tea.Cmd {
	if m.overlay == overlayFinder {
		return tea.Batch(m.enumerate(), m.rank())
	}
	m.latest.Cancel("finder")
	m.finder = finder{query: m.finder.query}
	return nil
}
