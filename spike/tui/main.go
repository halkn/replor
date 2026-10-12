// Command tui is the P0-01/03/05 spike: a two-pane tree + preview on Bubble
// Tea v2 exercising resize, focus, link/TOC/search navigation with history,
// and inline images (Kitty placeholders or half blocks).
//
//	go run ./spike/tui [-images auto|kitty|halfblocks|off] [PATH]
//
// Keys: Tab focus · j/k move/scroll · h/l collapse/expand · Enter open ·
// >/< select next/prev link · Backspace back · ] forward · t TOC ·
// / search, n/N next/prev · s source · b toggle tree · q quit
package main

import (
	"bytes"
	"context"
	"flag"
	"fmt"
	"image"
	_ "image/jpeg"
	"image/png"
	"os"
	"path"
	"sort"
	"strings"
	"sync/atomic"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
	mermaid "github.com/zkrebbekx/go-mermaid"
	"github.com/zkrebbekx/go-mermaid/raster"

	"github.com/halkn/replor/internal/app"
	"github.com/halkn/replor/internal/document"
	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/workspace"
)

const maxPreviewBytes = 4 << 20

type node struct {
	rel      string
	name     string
	depth    int
	dir      bool
	expanded bool
}

type histEntry struct {
	path   string
	pos    preview.Position
	source bool
}

type pane int

const (
	paneTree pane = iota
	paneDoc
)

type model struct {
	ws      *workspace.Workspace
	latest  *app.Latest
	imgMode preview.ImageMode

	width, height int
	focus         pane
	showTree      bool

	nodes  []node
	cursor int

	path     string
	doc      *document.Doc
	rendered *preview.Rendered
	source   bool
	pos      preview.Position
	top      int
	pending  bool
	status   string
	kittyIDs []uint32

	back, fwd []histEntry

	linkIdx int

	searching bool
	query     string
	matches   []preview.Match
	matchIdx  int

	toc       bool
	tocCursor int
}

var kittySeq atomic.Uint32

type renderedMsg struct {
	ticket   app.Ticket
	path     string
	doc      *document.Doc
	rendered *preview.Rendered
	transmit []string
	kittyIDs []uint32
	notes    []string
	err      error
}

func main() {
	imagesFlag := flag.String("images", "auto", "auto|kitty|halfblocks|off")
	flag.Parse()
	ws, err := workspace.Open(flag.Arg(0))
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	defer ws.Close()

	mode := preview.DetectImageMode(os.Getenv)
	switch *imagesFlag {
	case "kitty":
		mode = preview.ImageKitty
	case "halfblocks":
		mode = preview.ImageHalfblocks
	case "off":
		mode = preview.ImageOff
	}

	m := &model{ws: ws, latest: app.NewLatest(), imgMode: mode, showTree: true, linkIdx: -1}
	m.nodes = m.list(".", 0)
	if ws.Selected != "" {
		m.path = ws.Selected
	} else if _, err := ws.Stat("README.md"); err == nil {
		m.path = "README.md"
	}
	if _, err := tea.NewProgram(m).Run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

// list reads a directory synchronously. The spike accepts this; P1 moves it
// to an async command like rendering.
func (m *model) list(rel string, depth int) []node {
	entries, err := m.ws.ReadDir(rel)
	if err != nil {
		return nil
	}
	sort.SliceStable(entries, func(i, j int) bool {
		if entries[i].IsDir() != entries[j].IsDir() {
			return entries[i].IsDir()
		}
		return entries[i].Name() < entries[j].Name()
	})
	var out []node
	for _, e := range entries {
		if e.Name() == ".git" {
			continue
		}
		out = append(out, node{rel: path.Join(rel, e.Name()), name: document.Sanitize(e.Name()), depth: depth, dir: e.IsDir()})
	}
	return out
}

func (m *model) Init() tea.Cmd { return nil }

func (m *model) docWidth() int {
	w := m.width
	if m.showTree && m.width >= 70 {
		w -= m.treeWidth() + 1
	}
	return max(w-2, 10)
}

func (m *model) treeWidth() int { return min(32, m.width/3) }

func (m *model) bodyHeight() int { return max(m.height-2, 1) }

// request renders m.path at the current width. Reading, parsing, rendering
// and image conversion all happen off the update loop.
func (m *model) request() tea.Cmd {
	if m.path == "" || m.width == 0 {
		return nil
	}
	ticket, ctx := m.latest.Issue(context.Background(), "preview")
	m.pending = true
	p, width, source, mode, ws := m.path, m.docWidth(), m.source, m.imgMode, m.ws
	return func() tea.Msg {
		return load(ctx, ws, ticket, p, width, source, mode)
	}
}

func load(ctx context.Context, ws *workspace.Workspace, ticket app.Ticket, p string, width int, source bool, mode preview.ImageMode) renderedMsg {
	msg := renderedMsg{ticket: ticket, path: p}
	data, err := ws.ReadFile(p, maxPreviewBytes)
	if err != nil {
		msg.err = err
		return msg
	}
	if ctx.Err() != nil {
		msg.err = ctx.Err()
		return msg
	}
	if strings.HasSuffix(strings.ToLower(p), ".png") || strings.HasSuffix(strings.ToLower(p), ".jpg") {
		img, _, err := image.Decode(bytes.NewReader(data))
		if err != nil {
			msg.err = err
			return msg
		}
		lines, tx, id := picture(img, data, width, mode)
		msg.rendered = &preview.Rendered{Width: width, Lines: lines, Plain: lines, BlockLine: []int{0}}
		msg.doc = &document.Doc{Blocks: []document.Block{{}}}
		if tx != "" {
			msg.transmit, msg.kittyIDs = []string{tx}, []uint32{id}
		}
		return msg
	}
	isMD := strings.HasSuffix(strings.ToLower(p), ".md")
	if source || !isMD {
		text := document.Sanitize(string(data))
		lines := strings.Split(text, "\n")
		for i, l := range lines {
			lines[i] = fmt.Sprintf("%5d │ %s", i+1, strings.ReplaceAll(l, "\t", "    "))
		}
		msg.doc = &document.Doc{Blocks: make([]document.Block, len(lines))}
		bl := make([]int, len(lines))
		for i := range bl {
			bl[i] = i
		}
		msg.rendered = &preview.Rendered{Width: width, Lines: lines, Plain: lines, BlockLine: bl}
		return msg
	}

	d := document.Parse(string(data))
	hook := func(i, w int) ([]string, bool) {
		b := d.Blocks[i]
		var png []byte
		switch {
		case b.Lang == "mermaid":
			svg, err := mermaid.Render(b.Code)
			if err != nil {
				msg.notes = append(msg.notes, fmt.Sprintf("mermaid block %d: %v", i, err))
				return nil, false
			}
			png, err = raster.RasterizeSVG(svg, 2)
			if err != nil {
				msg.notes = append(msg.notes, fmt.Sprintf("mermaid raster %d: %v", i, err))
				return nil, false
			}
		case b.Image != "":
			l, err := document.ResolveLink(ws, p, ".", b.Image)
			if err != nil || l.Kind != document.LinkLocal {
				return nil, false
			}
			png, err = ws.ReadFile(l.Path, maxPreviewBytes)
			if err != nil {
				return nil, false
			}
		default:
			return nil, false
		}
		if mode == preview.ImageOff {
			return nil, false
		}
		img, _, err := image.Decode(bytes.NewReader(png))
		if err != nil {
			return nil, false
		}
		lines, tx, id := picture(img, png, w, mode)
		if tx != "" {
			msg.transmit = append(msg.transmit, tx)
			msg.kittyIDs = append(msg.kittyIDs, id)
		}
		return lines, true
	}
	r, err := preview.RenderMarkdown(d, width, hook)
	msg.doc, msg.rendered, msg.err = d, r, err
	return msg
}

func picture(img image.Image, data []byte, width int, mode preview.ImageMode) (lines []string, transmit string, id uint32) {
	cols, rows := preview.FitCells(img.Bounds(), min(width, img.Bounds().Dx()/8+1), 24)
	switch mode {
	case preview.ImageKitty:
		cols, rows = min(cols, preview.MaxKittyCells), min(rows, preview.MaxKittyCells)
		id = kittySeq.Add(1) + 100
		var buf bytes.Buffer
		if err := png.Encode(&buf, img); err == nil {
			data = buf.Bytes()
		}
		return preview.KittyPlaceholders(id, cols, rows), preview.KittyTransmit(data, id, cols, rows), id
	case preview.ImageHalfblocks:
		return preview.Halfblocks(img, cols, rows), "", 0
	}
	return []string{fmt.Sprintf("[image %dx%d]", img.Bounds().Dx(), img.Bounds().Dy())}, "", 0
}

func (m *model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		if m.rendered != nil {
			m.pos = m.rendered.PositionAt(m.top)
		}
		m.width, m.height = msg.Width, msg.Height
		return m, m.request()

	case renderedMsg:
		if !m.latest.Accept(msg.ticket) {
			return m, nil // stale: a newer selection, resize or mode switch won
		}
		m.pending = false
		if msg.err != nil {
			m.status = "error: " + msg.err.Error()
			// Everything derived from the lost rendering goes with it.
			m.rendered, m.doc = nil, nil
			m.linkIdx, m.matches, m.matchIdx, m.toc = -1, nil, 0, false
			return m, nil
		}
		var cmds []tea.Cmd
		for _, id := range m.kittyIDs {
			cmds = append(cmds, tea.Raw(preview.KittyDelete(id)))
		}
		for _, tx := range msg.transmit {
			cmds = append(cmds, tea.Raw(tx))
		}
		m.kittyIDs = msg.kittyIDs
		m.doc, m.rendered = msg.doc, msg.rendered
		m.top = m.rendered.LineOf(m.pos)
		m.clampTop()
		if m.query != "" {
			m.matches = m.rendered.Find(m.query)
		}
		// The file may have changed since the last load; selections into
		// the old lists are dropped or clamped.
		if m.linkIdx >= len(m.doc.Links) {
			m.linkIdx = -1
		}
		if m.tocCursor >= len(m.doc.Headings) {
			m.toc, m.tocCursor = false, 0
		}
		if m.matchIdx >= len(m.matches) {
			m.matchIdx = 0
		}
		m.status = strings.Join(msg.notes, "; ")
		return m, tea.Sequence(cmds...)

	case tea.KeyPressMsg:
		return m.key(msg)
	}
	return m, nil
}

func (m *model) clampTop() {
	if m.rendered == nil {
		m.top = 0
		return
	}
	m.top = max(0, min(m.top, len(m.rendered.Lines)-m.bodyHeight()))
}

func (m *model) open(p string, pos preview.Position, source bool, record bool) tea.Cmd {
	if record && m.path != "" {
		m.back = append(m.back, histEntry{m.path, m.currentPos(), m.source})
		m.fwd = nil
	}
	m.path, m.pos, m.source = p, pos, source
	m.linkIdx, m.matches, m.matchIdx = -1, nil, 0
	return m.request()
}

func (m *model) currentPos() preview.Position {
	if m.rendered == nil {
		return m.pos
	}
	return m.rendered.PositionAt(m.top)
}

func (m *model) jump(line int) {
	m.top = line
	m.clampTop()
	m.pos = m.rendered.PositionAt(m.top)
}

func (m *model) key(k tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	s := k.String()
	if m.searching {
		switch s {
		case "enter":
			m.searching = false
			if m.rendered != nil {
				m.matches, m.matchIdx = m.rendered.Find(m.query), 0
				if len(m.matches) > 0 {
					m.jump(m.matches[0].Line)
				}
			}
		case "esc":
			m.searching, m.query = false, ""
		case "backspace":
			if m.query != "" {
				_, size := lastRune(m.query)
				m.query = m.query[:len(m.query)-size]
			}
		default:
			if t := k.Text; t != "" {
				m.query += t
			}
		}
		return m, nil
	}
	if m.toc {
		switch s {
		case "j", "down":
			m.tocCursor = min(m.tocCursor+1, len(m.doc.Headings)-1)
		case "k", "up":
			m.tocCursor = max(m.tocCursor-1, 0)
		case "enter":
			h := m.doc.Headings[m.tocCursor]
			m.back = append(m.back, histEntry{m.path, m.currentPos(), m.source})
			m.fwd = nil
			m.jump(m.rendered.LineOf(preview.Position{Block: h.Block}))
			m.toc = false
		case "esc", "q", "t":
			m.toc = false
		}
		return m, nil
	}

	switch s {
	case "q", "ctrl+c":
		var cmds []tea.Cmd
		for _, id := range m.kittyIDs {
			cmds = append(cmds, tea.Raw(preview.KittyDelete(id)))
		}
		return m, tea.Sequence(append(cmds, tea.Quit)...)
	case "tab":
		if m.focus == paneTree {
			m.focus = paneDoc
		} else {
			m.focus = paneTree
		}
	case "b":
		m.showTree = !m.showTree
		return m, m.request()
	case "s":
		return m, m.open(m.path, m.currentPos(), !m.source, false)
	case "/":
		m.searching, m.query = true, ""
	case "n", "N":
		if len(m.matches) > 0 {
			d := 1
			if s == "N" {
				d = len(m.matches) - 1
			}
			m.matchIdx = (m.matchIdx + d) % len(m.matches)
			m.jump(m.matches[m.matchIdx].Line)
		}
	case "t":
		if m.doc != nil && len(m.doc.Headings) > 0 && !m.source {
			m.toc, m.tocCursor = true, 0
		}
	case ">", "<":
		if m.doc != nil && len(m.doc.Links) > 0 && !m.source {
			if s == ">" {
				m.linkIdx = (m.linkIdx + 1) % len(m.doc.Links)
			} else {
				m.linkIdx = (m.linkIdx - 1 + len(m.doc.Links)) % len(m.doc.Links)
			}
			l := m.doc.Links[m.linkIdx]
			line := m.rendered.LineOf(preview.Position{Block: l.Block})
			if line < m.top || line >= m.top+m.bodyHeight() {
				m.jump(line)
			}
		}
	case "backspace":
		if n := len(m.back); n > 0 {
			e := m.back[n-1]
			m.back = m.back[:n-1]
			m.fwd = append(m.fwd, histEntry{m.path, m.currentPos(), m.source})
			// Without a rendering (e.g. after a load error) the same
			// document is reopened rather than scrolled.
			if e.path == m.path && e.source == m.source && m.rendered != nil {
				m.jump(m.rendered.LineOf(e.pos))
				return m, nil
			}
			return m, m.open(e.path, e.pos, e.source, false)
		}
	case "]":
		if n := len(m.fwd); n > 0 {
			e := m.fwd[n-1]
			m.fwd = m.fwd[:n-1]
			m.back = append(m.back, histEntry{m.path, m.currentPos(), m.source})
			// Without a rendering (e.g. after a load error) the same
			// document is reopened rather than scrolled.
			if e.path == m.path && e.source == m.source && m.rendered != nil {
				m.jump(m.rendered.LineOf(e.pos))
				return m, nil
			}
			return m, m.open(e.path, e.pos, e.source, false)
		}
	case "enter":
		if m.focus == paneDoc && m.linkIdx >= 0 {
			return m, m.followLink()
		}
		if m.focus == paneTree && m.cursor >= 0 && m.cursor < len(m.nodes) {
			n := m.nodes[m.cursor]
			if n.dir {
				m.toggle(m.cursor)
				return m, nil
			}
			return m, m.open(n.rel, preview.Position{}, false, true)
		}
	case "j", "down", "k", "up", "pgdown", "pgup", "g", "G", "h", "left", "l", "right":
		m.move(s)
	}
	return m, nil
}

func (m *model) followLink() tea.Cmd {
	l := m.doc.Links[m.linkIdx]
	target, err := document.ResolveLink(m.ws, m.path, ".", l.Dest)
	if err != nil {
		m.status = "link: " + err.Error()
		return nil
	}
	switch target.Kind {
	case document.LinkExternal:
		m.status = "external (would open only with o): " + target.URL
		return nil
	case document.LinkAnchor:
		h, ok := m.doc.HeadingByID(target.Fragment)
		if !ok {
			m.status = "anchor not found: #" + target.Fragment
			return nil
		}
		m.back = append(m.back, histEntry{m.path, m.currentPos(), m.source})
		m.fwd = nil
		m.jump(m.rendered.LineOf(preview.Position{Block: h.Block}))
		return nil
	}
	// Fragments into other documents are resolved after load in P2; the
	// spike opens the top of the target.
	return m.open(target.Path, preview.Position{}, false, true)
}

func (m *model) toggle(i int) {
	n := &m.nodes[i]
	if n.expanded {
		j := i + 1
		for j < len(m.nodes) && m.nodes[j].depth > n.depth {
			j++
		}
		m.nodes = append(m.nodes[:i+1], m.nodes[j:]...)
		n.expanded = false
		return
	}
	children := m.list(n.rel, n.depth+1)
	m.nodes = append(m.nodes[:i+1], append(children, m.nodes[i+1:]...)...)
	m.nodes[i].expanded = true
}

func (m *model) move(s string) {
	if m.focus == paneTree {
		if len(m.nodes) == 0 {
			return
		}
		switch s {
		case "j", "down":
			m.cursor = min(m.cursor+1, len(m.nodes)-1)
		case "k", "up":
			m.cursor = max(m.cursor-1, 0)
		case "l", "right":
			if m.cursor < len(m.nodes) && m.nodes[m.cursor].dir && !m.nodes[m.cursor].expanded {
				m.toggle(m.cursor)
			}
		case "h", "left":
			if m.cursor < len(m.nodes) && m.nodes[m.cursor].expanded {
				m.toggle(m.cursor)
			}
		}
		return
	}
	if m.rendered == nil {
		return
	}
	switch s {
	case "j", "down":
		m.top++
	case "k", "up":
		m.top--
	case "pgdown":
		m.top += m.bodyHeight()
	case "pgup":
		m.top -= m.bodyHeight()
	case "g":
		m.top = 0
	case "G":
		m.top = len(m.rendered.Lines)
	}
	m.clampTop()
	m.pos = m.rendered.PositionAt(m.top)
}

var (
	focusBorder = lipgloss.NewStyle().Foreground(lipgloss.Color("63"))
	dim         = lipgloss.NewStyle().Foreground(lipgloss.Color("243"))
	selected    = lipgloss.NewStyle().Reverse(true)
	bar         = lipgloss.NewStyle().Background(lipgloss.Color("236")).Foreground(lipgloss.Color("252"))
)

func (m *model) View() tea.View {
	if m.width == 0 {
		return tea.NewView("")
	}
	h := m.bodyHeight()
	// Names, paths and messages come from the file system and the user;
	// they go through the same sanitizer as document text.
	header := bar.Width(m.width).Render(ansi.Truncate(document.Sanitize(fmt.Sprintf(" %s  %s  images=%s %s", path.Base(m.ws.Root), m.path, m.imgMode, pendingMark(m.pending))), m.width, "…"))

	// Lines are composed and truncated one by one instead of with
	// lipgloss.JoinHorizontal, which pads every line to the widest one and
	// so amplifies any disagreement on emoji widths.
	doc := strings.Split(m.docLines(h), "\n")
	var tree []string
	if m.showTree && m.width >= 70 {
		tree = strings.Split(m.treeLines(h), "\n")
	}
	sep := "│"
	if m.focus == paneDoc {
		sep = focusBorder.Render(sep)
	}
	rows := make([]string, h)
	for i := range h {
		row := " " + doc[i]
		if tree != nil {
			row = tree[i] + sep + row
		}
		rows[i] = ansi.Truncate(row, m.width, "")
	}
	body := strings.Join(rows, "\n")

	footer := document.Sanitize(m.footer())
	v := tea.NewView(header + "\n" + body + "\n" + bar.Width(m.width).Render(ansi.Truncate(footer, m.width, "…")))
	v.AltScreen = true
	return v
}

func pendingMark(p bool) string {
	if p {
		return "⟳"
	}
	return ""
}

func (m *model) footer() string {
	switch {
	case m.searching:
		return " /" + m.query + "▏"
	case m.status != "":
		return " " + m.status
	case m.linkIdx >= 0 && m.doc != nil:
		l := m.doc.Links[m.linkIdx]
		return fmt.Sprintf(" link %d/%d: %s → %s  (Enter follow)", m.linkIdx+1, len(m.doc.Links), l.Text, l.Dest)
	case len(m.matches) > 0:
		return fmt.Sprintf(" match %d/%d %q", m.matchIdx+1, len(m.matches), m.query)
	}
	return " Tab focus · Enter open · >/< link · ⌫ back · ] fwd · t TOC · / search · s source · b tree · q quit"
}

func (m *model) treeLines(h int) string {
	w := m.treeWidth()
	start := max(0, m.cursor-h+1)
	lines := make([]string, 0, h)
	for i := start; i < len(m.nodes) && len(lines) < h; i++ {
		n := m.nodes[i]
		icon := "  "
		if n.dir {
			icon = "▸ "
			if n.expanded {
				icon = "▾ "
			}
		}
		line := ansi.Truncate(strings.Repeat("  ", n.depth)+icon+n.name, w, "…")
		line += strings.Repeat(" ", max(0, w-ansi.StringWidth(line)))
		if i == m.cursor {
			if m.focus == paneTree {
				line = selected.Render(line)
			} else {
				line = dim.Render(line)
			}
		}
		lines = append(lines, line)
	}
	for len(lines) < h {
		lines = append(lines, strings.Repeat(" ", w))
	}
	return strings.Join(lines, "\n")
}

func (m *model) docLines(h int) string {
	w := m.docWidth()
	if m.toc && m.doc != nil {
		var out []string
		for i, hd := range m.doc.Headings {
			l := ansi.Truncate(strings.Repeat("  ", hd.Level-1)+hd.Text+"  #"+hd.ID, w, "…")
			if i == m.tocCursor {
				l = selected.Render(l)
			}
			out = append(out, l)
		}
		return pad(out, h)
	}
	if m.rendered == nil {
		return pad([]string{dim.Render("(no document)")}, h)
	}
	end := min(m.top+h, len(m.rendered.Lines))
	matchLine := -1
	if len(m.matches) > 0 {
		matchLine = m.matches[m.matchIdx].Line
	}
	linkBlock := -1
	if m.linkIdx >= 0 {
		linkBlock = m.doc.Links[m.linkIdx].Block
	}
	out := make([]string, 0, h)
	for i := m.top; i < end; i++ {
		gutter := " "
		switch {
		case i == matchLine:
			gutter = "▶"
		case linkBlock >= 0 && m.rendered.BlockAt(i) == linkBlock:
			gutter = focusBorder.Render("┃")
		}
		out = append(out, gutter+ansi.Truncate(m.rendered.Lines[i], w, ""))
	}
	return pad(out, h)
}

func pad(lines []string, h int) string {
	for len(lines) < h {
		lines = append(lines, "")
	}
	return strings.Join(lines[:h], "\n")
}

func lastRune(s string) (rune, int) {
	for i := len(s) - 1; i >= 0; i-- {
		if s[i]&0xC0 != 0x80 {
			r := []rune(s[i:])
			return r[0], len(s) - i
		}
	}
	return 0, 0
}
