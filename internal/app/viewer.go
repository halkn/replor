package app

import (
	"strings"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/document"
	"github.com/halkn/replor/internal/preview"
)

// firstBlocks is how many Markdown blocks past the reading position are
// rendered before the first paint; the rest follows in a second pass
// (ROADMAP P1-04: 1 MiB can take seconds on slow machines).
const firstBlocks = 80

type viewer struct {
	target  *Target
	res     preview.Result
	err     error
	pending bool

	top   int
	pos   preview.Position
	focus *preview.Position
	// sourceLine carries the reading position across a Preview/Source
	// switch, where block numbering differs; -1 when unused.
	sourceLine int

	kittyIDs []uint32

	linkIdx  int
	query    string
	matches  []preview.Match
	matchIdx int
}

func (v *viewer) rendered() *preview.Rendered { return v.res.Rendered }

func (v *viewer) doc() *document.Doc { return v.res.Doc }

// savePos records the reading position before a re-layout.
func (v *viewer) savePos() {
	if r := v.rendered(); r != nil {
		v.pos = r.PositionAt(v.top)
	}
}

type loadedMsg struct {
	ticket Ticket
	target Target
	res    preview.Result
	err    error
}

// load renders the current target at the current width. Markdown is first
// rendered up to a little past the reading position; applyLoad then asks for
// the rest (loadFull). Partial renderings are exact prefixes of the full one,
// so positions stay valid in between.
func (m *Model) load() tea.Cmd { return m.request(m.viewer.pos.Block + firstBlocks) }

func (m *Model) loadFull() tea.Cmd { return m.request(0) }

func (m *Model) request(blocks int) tea.Cmd {
	t := m.viewer.target
	if t == nil || m.width == 0 {
		return nil
	}
	ticket, ctx := m.issue("preview")
	m.viewer.pending = true
	req := preview.Request{WS: m.ws, Path: t.Path, Source: t.Mode == ModeSource, Width: m.viewerWidth(), Images: m.opts.Images, Blocks: blocks}
	target := *t
	return func() tea.Msg {
		res, err := preview.Load(ctx, req)
		return loadedMsg{ticket: ticket, target: target, res: res, err: err}
	}
}

func (m *Model) applyLoad(msg loadedMsg) tea.Cmd {
	if !m.latest.Accept(msg.ticket) {
		return nil // superseded by a newer selection, resize or mode switch
	}
	v := &m.viewer
	v.pending = false
	cmds := m.dropImages()
	if msg.err != nil {
		v.err, v.res = msg.err, preview.Result{}
		v.linkIdx, v.matches, v.matchIdx = -1, nil, 0
		if m.overlay == overlayTOC {
			m.overlay = overlayNone
		}
		return tea.Sequence(cmds...)
	}
	v.err, v.res = nil, msg.res
	for _, tx := range msg.res.Transmit {
		cmds = append(cmds, tea.Raw(tx))
	}
	v.kittyIDs = msg.res.KittyIDs

	r := v.rendered()
	if r != nil {
		if v.sourceLine >= 0 {
			v.pos = positionForLine(msg, v.sourceLine)
			v.sourceLine = -1
		}
		v.top = r.LineOf(v.pos)
		m.clampTop()
		m.keepFocusVisible()
		if v.query != "" {
			v.matches = r.Find(v.query)
		}
	}
	// The file may have changed since the last load: drop selections that
	// no longer exist.
	if d := v.doc(); d == nil || v.linkIdx >= len(d.Links) {
		v.linkIdx = -1
	}
	if v.matchIdx >= len(v.matches) {
		v.matchIdx = 0
	}
	if m.overlay == overlayTOC && (v.doc() == nil || m.tocCursor >= len(v.doc().Headings)) {
		m.overlay = overlayNone
	}
	if msg.res.Note != "" {
		m.status = msg.res.Note
	}
	if r != nil && !r.Complete {
		cmds = append(cmds, m.loadFull())
	}
	return tea.Sequence(cmds...)
}

// positionForLine maps a source line to a position in msg's rendering:
// identity for source, the block containing the line for Markdown.
func positionForLine(msg loadedMsg, line int) preview.Position {
	d := msg.res.Doc
	if d == nil {
		return preview.Position{Block: line}
	}
	b := 0
	for i := range d.Blocks {
		if strings.Count(d.Source[:d.Blocks[i].Start], "\n") > line {
			break
		}
		b = i
	}
	return preview.Position{Block: b}
}

// sourceLineOf is the inverse of positionForLine for the current rendering.
func (v *viewer) sourceLineOf() int {
	r := v.rendered()
	if r == nil {
		return 0
	}
	p := r.PositionAt(v.top)
	d := v.doc()
	if d == nil {
		return p.Block
	}
	return strings.Count(d.Source[:d.Blocks[p.Block].Start], "\n")
}

// dropImages frees the Kitty images of the current rendering.
func (m *Model) dropImages() []tea.Cmd {
	var cmds []tea.Cmd
	for _, id := range m.viewer.kittyIDs {
		cmds = append(cmds, tea.Raw(preview.KittyDelete(id)))
	}
	m.viewer.kittyIDs = nil
	return cmds
}

// open shows target at pos. record pushes the current view onto the back
// history and clears the forward history.
func (m *Model) open(t Target, pos preview.Position, focus *preview.Position, record bool) tea.Cmd {
	v := &m.viewer
	if record && v.target != nil {
		m.back = append(m.back, m.here())
		m.fwd = nil
	}
	v.target = &t
	v.pos, v.focus, v.top = pos, focus, 0
	v.res, v.err = preview.Result{}, nil
	v.linkIdx, v.matches, v.matchIdx = -1, nil, 0
	return m.load()
}

// here is the current view as a history entry.
func (m *Model) here() HistoryEntry {
	v := &m.viewer
	e := HistoryEntry{Target: *v.target, Pos: v.pos, Focus: v.focus}
	if r := v.rendered(); r != nil {
		e.Pos = r.PositionAt(v.top)
	}
	return e
}

// jumpTo scrolls so that p is at the top (as far as the end allows) and
// keeps it as the focus to hold on screen across re-layouts.
func (m *Model) jumpTo(p preview.Position) {
	v := &m.viewer
	r := v.rendered()
	if r == nil {
		return
	}
	v.top = r.LineOf(p)
	m.clampTop()
	v.pos = r.PositionAt(v.top)
	v.focus = &p
}

// jumpInDoc records history before an in-document jump (link, TOC).
func (m *Model) jumpInDoc(p preview.Position) {
	m.back = append(m.back, m.here())
	m.fwd = nil
	m.jumpTo(p)
}

func (m *Model) clampTop() {
	v := &m.viewer
	r := v.rendered()
	if r == nil {
		v.top = 0
		return
	}
	v.top = max(0, min(v.top, len(r.Lines)-m.bodyHeight()))
}

// keepFocusVisible scrolls minimally so that the focus line stays on screen
// after a re-layout moved it (DESIGN 4.3).
func (m *Model) keepFocusVisible() {
	v := &m.viewer
	r := v.rendered()
	if r == nil || v.focus == nil {
		return
	}
	line := r.LineOf(*v.focus)
	h := m.bodyHeight()
	switch {
	case line < v.top:
		v.top = line
	case line >= v.top+h:
		v.top = line - h + 1
	}
	m.clampTop()
	v.pos = r.PositionAt(v.top)
}

// scroll moves the view by lines; manual scrolling releases the focus.
func (m *Model) scroll(delta int) {
	v := &m.viewer
	r := v.rendered()
	if r == nil {
		return
	}
	v.top += delta
	m.clampTop()
	v.pos = r.PositionAt(v.top)
	v.focus = nil
}

func (m *Model) restore(e HistoryEntry) tea.Cmd {
	v := &m.viewer
	// Without a rendering (e.g. after a load error) the same document is
	// reopened rather than scrolled.
	if v.target != nil && *v.target == e.Target && v.rendered() != nil {
		v.top = v.rendered().LineOf(e.Pos)
		m.clampTop()
		v.pos, v.focus = e.Pos, e.Focus
		m.keepFocusVisible()
		return nil
	}
	return m.open(e.Target, e.Pos, e.Focus, false)
}

func (m *Model) goBack() tea.Cmd {
	n := len(m.back)
	if n == 0 || m.viewer.target == nil {
		m.status = "no earlier page"
		return nil
	}
	e := m.back[n-1]
	m.back = m.back[:n-1]
	m.fwd = append(m.fwd, m.here())
	return m.restore(e)
}

func (m *Model) goForward() tea.Cmd {
	n := len(m.fwd)
	if n == 0 || m.viewer.target == nil {
		m.status = "no later page"
		return nil
	}
	e := m.fwd[n-1]
	m.fwd = m.fwd[:n-1]
	m.back = append(m.back, m.here())
	return m.restore(e)
}

// toggleSource switches a Markdown file between Preview and Source, keeping
// the reading position by source line.
func (m *Model) toggleSource() tea.Cmd {
	v := &m.viewer
	if v.target == nil || v.res.Kind != preview.KindMarkdown {
		m.status = "Preview/Source applies to Markdown files"
		return nil
	}
	line := v.sourceLineOf()
	t := *v.target
	if t.Mode == ModeSource {
		t.Mode = ModePreview
	} else {
		t.Mode = ModeSource
	}
	cmd := m.open(t, preview.Position{}, nil, false)
	v.sourceLine = line
	return cmd
}

func (m *Model) search(query string) {
	v := &m.viewer
	v.query = query
	v.matches, v.matchIdx = nil, 0
	r := v.rendered()
	if r == nil || query == "" {
		return
	}
	v.matches = r.Find(query)
	if len(v.matches) == 0 {
		m.status = "not found: " + query
		return
	}
	// Start from the first hit at or below the current view.
	for i, mt := range v.matches {
		if mt.Line >= v.top {
			v.matchIdx = i
			break
		}
	}
	m.jumpToMatch()
}

func (m *Model) nextMatch(delta int) {
	v := &m.viewer
	if len(v.matches) == 0 {
		m.status = "no search results"
		return
	}
	v.matchIdx = (v.matchIdx + delta + len(v.matches)) % len(v.matches)
	m.jumpToMatch()
}

func (m *Model) jumpToMatch() {
	v := &m.viewer
	p := v.rendered().PositionAt(v.matches[v.matchIdx].Line)
	m.jumpTo(p)
}

func (m *Model) selectLink(delta int) {
	v := &m.viewer
	d := v.doc()
	if d == nil || len(d.Links) == 0 {
		m.status = "no links in this view"
		return
	}
	v.linkIdx = (v.linkIdx + delta + len(d.Links)) % len(d.Links)
	p := preview.Position{Block: d.Links[v.linkIdx].Block}
	r := v.rendered()
	line := r.LineOf(p)
	if line < v.top || line >= v.top+m.bodyHeight() {
		m.jumpTo(p)
	} else {
		v.focus = &p
	}
}

func (m *Model) followLink() tea.Cmd {
	v := &m.viewer
	l := v.doc().Links[v.linkIdx]
	target, err := document.ResolveLink(m.ws, v.target.Path, ".", l.Dest)
	if err != nil {
		m.status = "link: " + err.Error()
		return nil
	}
	switch target.Kind {
	case document.LinkExternal:
		m.status = "external link (not opened): " + target.URL
		return nil
	case document.LinkAnchor:
		h, ok := v.doc().HeadingByID(target.Fragment)
		if !ok {
			m.status = "anchor not found: #" + target.Fragment
			return nil
		}
		m.jumpInDoc(preview.Position{Block: h.Block})
		return nil
	}
	return m.open(Target{Path: target.Path}, preview.Position{}, nil, true)
}

func (m *Model) reload() tea.Cmd {
	m.viewer.savePos()
	return tea.Batch(m.refreshTree(), m.load(), m.invalidateFinder(), m.loadGit())
}
