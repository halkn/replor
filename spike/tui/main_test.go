package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/charmbracelet/x/ansi"

	"github.com/halkn/replor/internal/app"
	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/workspace"
)

// drive applies msg and synchronously runs the render commands it returns,
// standing in for the Bubble Tea runtime.
func drive(t *testing.T, m *model, msg tea.Msg) {
	t.Helper()
	_, cmd := m.Update(msg)
	run(t, m, cmd)
}

func run(t *testing.T, m *model, cmd tea.Cmd) {
	t.Helper()
	if cmd == nil {
		return
	}
	if r, ok := cmd().(renderedMsg); ok {
		drive(t, m, r)
	}
}

func key(s string) tea.KeyPressMsg {
	switch s {
	case "enter":
		return tea.KeyPressMsg{Code: tea.KeyEnter}
	case "tab":
		return tea.KeyPressMsg{Code: tea.KeyTab}
	case "backspace":
		return tea.KeyPressMsg{Code: tea.KeyBackspace}
	case "esc":
		return tea.KeyPressMsg{Code: tea.KeyEscape}
	}
	r := []rune(s)[0]
	return tea.KeyPressMsg{Code: r, Text: s}
}

func newModel(t *testing.T, target string) *model {
	t.Helper()
	ws, err := workspace.Open(target)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ws.Close() })
	m := &model{ws: ws, latest: app.NewLatest(), imgMode: preview.ImageOff, showTree: true, linkIdx: -1}
	m.nodes = m.list(".", 0)
	m.path = ws.Selected
	return m
}

func screen(t *testing.T, m *model) []string {
	t.Helper()
	lines := strings.Split(m.View().Content, "\n")
	if len(lines) != m.height {
		t.Fatalf("view has %d lines, want %d", len(lines), m.height)
	}
	for i, l := range lines {
		// Bubble Tea measures with wcwidth unless the terminal negotiates
		// grapheme clustering (mode 2027), so both must fit.
		for _, method := range []ansi.Method{ansi.GraphemeWidth, ansi.WcWidth} {
			if w := method.StringWidth(l); w > m.width {
				t.Errorf("method %v: line %d is %d cells wide (> %d): %q", method, i, w, m.width, ansi.Strip(l))
			}
		}
	}
	return lines
}

func topLine(m *model) string { return m.rendered.Plain[m.top] }

func onScreen(m *model, text string) bool {
	for i := m.top; i < min(m.top+m.bodyHeight(), len(m.rendered.Plain)); i++ {
		if strings.Contains(m.rendered.Plain[i], text) {
			return true
		}
	}
	return false
}

func TestLayoutAcrossResizes(t *testing.T) {
	m := newModel(t, filepath.Join("..", "..", "testdata", "markdown", "gfm.md"))
	for _, sz := range [][2]int{{120, 40}, {80, 24}, {60, 15}, {40, 10}, {200, 60}} {
		drive(t, m, tea.WindowSizeMsg{Width: sz[0], Height: sz[1]})
		screen(t, m)
	}
	drive(t, m, key("tab"))
	if m.focus != paneDoc {
		t.Error("tab did not move focus to the document")
	}
	drive(t, m, key("tab"))
	if m.focus != paneTree {
		t.Error("tab did not move focus back to the tree")
	}
	drive(t, m, key("j"))
	screen(t, m)
}

func TestLinkTOCSearchHistoryResize(t *testing.T) {
	m := newModel(t, filepath.Join("..", "..", "testdata", "markdown", "gfm.md"))
	drive(t, m, tea.WindowSizeMsg{Width: 100, Height: 30})
	m.focus = paneDoc

	// Select the "#data-flow--api-v2" link (selection may scroll), then
	// move to a distinct position so "back" has something to restore.
	for range len(m.doc.Links) {
		drive(t, m, key(">"))
		if m.doc.Links[m.linkIdx].Dest == "#data-flow--api-v2" {
			break
		}
	}
	for range 3 {
		drive(t, m, key("k"))
	}
	before := topLine(m)
	drive(t, m, key("enter"))
	if !onScreen(m, "Data Flow") {
		t.Fatalf("after link: heading not on screen (top %q)", topLine(m))
	}
	drive(t, m, key("backspace"))
	if topLine(m) != before {
		t.Fatalf("back: top = %q, want %q", topLine(m), before)
	}
	drive(t, m, key("]"))
	if !onScreen(m, "Data Flow") {
		t.Fatalf("forward: heading not on screen (top %q)", topLine(m))
	}

	// TOC jump to the second duplicate heading.
	drive(t, m, key("t"))
	for m.doc.Headings[m.tocCursor].ID != "重複見出し-1" {
		drive(t, m, key("j"))
	}
	drive(t, m, key("enter"))
	want := m.rendered.LineOf(preview.Position{Block: m.doc.Headings[m.tocCursor].Block})
	if want < m.top || want >= m.top+m.bodyHeight() {
		t.Fatalf("TOC target line %d not in view [%d,%d)", want, m.top, m.top+m.bodyHeight())
	}

	// Search, then shrink the terminal: the hit stays on screen.
	drive(t, m, key("/"))
	for _, r := range "書生" {
		drive(t, m, key(string(r)))
	}
	drive(t, m, key("enter"))
	drive(t, m, tea.WindowSizeMsg{Width: 60, Height: 20})
	visible := false
	for _, mt := range m.rendered.Find("書生") {
		if mt.Line >= m.top && mt.Line < m.top+m.bodyHeight() {
			visible = true
		}
	}
	if !visible {
		t.Error("search hit scrolled out of view after resize")
	}
	screen(t, m)
}

func TestCrossDocumentLinkAndBack(t *testing.T) {
	m := newModel(t, filepath.Join("..", "..", "testdata", "azure-wiki", "Home.md"))
	drive(t, m, tea.WindowSizeMsg{Width: 100, Height: 30})
	m.focus = paneDoc
	for range len(m.doc.Links) {
		drive(t, m, key(">"))
		if m.doc.Links[m.linkIdx].Dest == "/Design/Architecture" {
			break
		}
	}
	drive(t, m, key("enter"))
	if m.path != "Design/Architecture.md" {
		t.Fatalf("path = %q", m.path)
	}
	drive(t, m, key("backspace"))
	if m.path != "Home.md" {
		t.Fatalf("back path = %q", m.path)
	}
	// A rejected link must not navigate.
	for range len(m.doc.Links) {
		drive(t, m, key(">"))
		if m.doc.Links[m.linkIdx].Dest == "/../secret.md" {
			break
		}
	}
	drive(t, m, key("enter"))
	if m.path != "Home.md" || !strings.Contains(m.status, "outside") {
		t.Errorf("escape link: path=%q status=%q", m.path, m.status)
	}
}

// Results delivered out of order: only the newest selection is shown.
func TestStaleRenderIgnored(t *testing.T) {
	m := newModel(t, filepath.Join("..", "..", "testdata", "azure-wiki", "Home.md"))
	drive(t, m, tea.WindowSizeMsg{Width: 100, Height: 30})
	cmdA := m.open("Getting-Started.md", preview.Position{}, false, true)
	cmdB := m.open("Unlisted.md", preview.Position{}, false, true)
	resB, resA := cmdB(), cmdA()
	drive(t, m, resB)
	drive(t, m, resA)
	if !strings.Contains(strings.Join(m.rendered.Plain, "\n"), "Unlisted") {
		t.Errorf("stale result replaced the current document")
	}
}

func TestImagesHalfblocksAndFallback(t *testing.T) {
	m := newModel(t, filepath.Join("..", "..", "testdata", "markdown", "media.md"))
	m.imgMode = preview.ImageHalfblocks
	drive(t, m, tea.WindowSizeMsg{Width: 100, Height: 40})
	all := strings.Join(m.rendered.Plain, "\n")
	if !strings.Contains(all, "▀") {
		t.Error("no half-block image rendered")
	}
	// The broken diagram falls back to its source and the error is reported
	// without stopping the rest of the document.
	if !strings.Contains(all, "A -->") || !strings.Contains(m.status, "mermaid block") || !strings.Contains(all, "後続の段落") {
		t.Errorf("fallback missing: status=%q", m.status)
	}
	for _, sz := range [][2]int{{60, 20}, {140, 50}} {
		drive(t, m, tea.WindowSizeMsg{Width: sz[0], Height: sz[1]})
		screen(t, m)
	}
}

func TestImagesKittyPlaceholders(t *testing.T) {
	m := newModel(t, filepath.Join("..", "..", "testdata", "markdown", "media.md"))
	m.imgMode = preview.ImageKitty
	_, cmd := m.Update(tea.WindowSizeMsg{Width: 100, Height: 40})
	msg := cmd().(renderedMsg)
	if len(msg.transmit) != 2 || len(msg.kittyIDs) != 2 {
		t.Fatalf("transmits=%d ids=%v", len(msg.transmit), msg.kittyIDs)
	}
	drive(t, m, msg)
	first := append([]uint32(nil), m.kittyIDs...)
	if !strings.Contains(strings.Join(m.rendered.Lines, "\n"), "\U0010EEEE") {
		t.Error("no placeholders in content")
	}
	// Scrolling moves placeholder rows like text; the screen stays in bounds.
	m.focus = paneDoc
	for range 10 {
		drive(t, m, key("j"))
		screen(t, m)
	}
	// Resize re-renders with new image IDs; the old ones are freed.
	drive(t, m, tea.WindowSizeMsg{Width: 70, Height: 30})
	for _, id := range m.kittyIDs {
		for _, old := range first {
			if id == old {
				t.Errorf("image id %d reused after resize", id)
			}
		}
	}
	screen(t, m)
}

// After the current document fails to load, history, link and search keys
// must reopen or no-op instead of touching the missing rendering.
func TestHistoryAfterLoadError(t *testing.T) {
	dir := t.TempDir()
	doc := filepath.Join(dir, "a.md")
	if err := os.WriteFile(doc, []byte("# A\n\n[x](#b)\n\n## B\n\ntext 書生\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	m := newModel(t, doc)
	drive(t, m, tea.WindowSizeMsg{Width: 80, Height: 20})
	m.focus = paneDoc
	drive(t, m, key("/"))
	for _, r := range "書生" {
		drive(t, m, key(string(r)))
	}
	drive(t, m, key("enter"))
	drive(t, m, key(">"))
	drive(t, m, key("enter")) // anchor jump: history now holds a.md

	if err := os.Remove(doc); err != nil {
		t.Fatal(err)
	}
	drive(t, m, tea.WindowSizeMsg{Width: 70, Height: 20}) // reload fails
	if m.rendered != nil || !strings.Contains(m.status, "error") {
		t.Fatalf("expected load error, status=%q", m.status)
	}
	for _, k := range []string{"enter", "n", "N", "t", ">"} {
		drive(t, m, key(k))
	}

	if err := os.WriteFile(doc, []byte("# A again\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	drive(t, m, key("backspace"))
	if m.rendered == nil || !strings.Contains(m.rendered.Plain[0], "A again") {
		t.Fatalf("back did not reload the document: %v", m.status)
	}
	drive(t, m, key("]"))
	if m.rendered == nil {
		t.Fatal("forward lost the document")
	}
}

func TestHeaderAndFooterSanitizeNames(t *testing.T) {
	dir := t.TempDir()
	name := "x\x1b]8;;file:evil\x07y.md"
	if err := os.WriteFile(filepath.Join(dir, name), []byte("# x\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	m := newModel(t, filepath.Join(dir, name))
	drive(t, m, tea.WindowSizeMsg{Width: 120, Height: 20})
	m.status = "error: " + name
	if v := m.View().Content; strings.Contains(v, "\x1b]8;;file") || strings.Contains(v, "\x07") {
		t.Errorf("control sequence from a file name reached the screen: %q", v)
	}
}

func TestEmptyTreeKeys(t *testing.T) {
	m := newModel(t, t.TempDir())
	drive(t, m, tea.WindowSizeMsg{Width: 80, Height: 20})
	for _, k := range []string{"j", "enter", "l", "h", "k", "j", "enter"} {
		drive(t, m, key(k))
	}
	screen(t, m)
}

// Reloading a document that lost links, headings or matches must not leave
// selections pointing past the new lists.
func TestSelectionsSurviveShrinkingReload(t *testing.T) {
	dir := t.TempDir()
	doc := filepath.Join(dir, "a.md")
	if err := os.WriteFile(doc, []byte("# A x\n\n[1](#a-x) [2](#b-x) [3](#c-x)\n\n# B x\n\n# C x\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	m := newModel(t, doc)
	drive(t, m, tea.WindowSizeMsg{Width: 80, Height: 20})
	m.focus = paneDoc
	for range 3 {
		drive(t, m, key(">"))
	}
	drive(t, m, key("/"))
	drive(t, m, key("x"))
	drive(t, m, key("enter"))
	drive(t, m, key("N"))
	drive(t, m, key("t"))
	drive(t, m, key("j"))
	drive(t, m, key("j"))

	if err := os.WriteFile(doc, []byte("plain\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	drive(t, m, tea.WindowSizeMsg{Width: 70, Height: 20}) // reload
	screen(t, m)
	for _, k := range []string{"enter", "esc", "enter", "n", "N", ">"} {
		drive(t, m, key(k))
		screen(t, m)
	}
}
