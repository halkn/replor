package app

import (
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/charmbracelet/x/ansi"

	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/workspace"
)

// The helpers drive the model without a terminal: commands run synchronously
// and their messages are fed back, standing in for the Bubble Tea runtime.

func drive(t *testing.T, m *Model, msg tea.Msg) {
	t.Helper()
	_, cmd := m.Update(msg)
	run(t, m, cmd)
}

func run(t *testing.T, m *Model, cmd tea.Cmd) {
	t.Helper()
	if cmd == nil {
		return
	}
	msg := cmd()
	// tea.Batch and tea.Sequence return slices of commands.
	if v := reflect.ValueOf(msg); v.Kind() == reflect.Slice {
		for i := range v.Len() {
			if c, ok := v.Index(i).Interface().(tea.Cmd); ok {
				run(t, m, c)
			}
		}
		return
	}
	switch msg.(type) {
	case dirListedMsg, loadedMsg, finderBatchMsg, finderRankedMsg, gitMsg:
		drive(t, m, msg)
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

func typeText(t *testing.T, m *Model, s string) {
	t.Helper()
	for _, r := range s {
		drive(t, m, key(string(r)))
	}
}

func start(t *testing.T, target string, w, h int) *Model {
	t.Helper()
	isolateGit(t)
	ws, err := workspace.Open(target)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ws.Close() })
	m := New(ws, Options{Images: preview.ImageOff})
	run(t, m, m.Init())
	drive(t, m, tea.WindowSizeMsg{Width: w, Height: h})
	return m
}

func fixture(parts ...string) string {
	return filepath.Join(append([]string{"..", "..", "testdata"}, parts...)...)
}

// screen renders the view and checks it fits the terminal with both width
// methods Bubble Tea may use.
func screen(t *testing.T, m *Model) []string {
	t.Helper()
	lines := strings.Split(m.View().Content, "\n")
	if len(lines) != m.height {
		t.Fatalf("view has %d lines, want %d", len(lines), m.height)
	}
	for i, l := range lines {
		for _, method := range []ansi.Method{ansi.GraphemeWidth, ansi.WcWidth} {
			if w := method.StringWidth(l); w > m.width {
				t.Errorf("method %v: line %d is %d cells wide (> %d): %q", method, i, w, m.width, ansi.Strip(l))
			}
		}
	}
	return lines
}

func plainScreen(t *testing.T, m *Model) string {
	return ansi.Strip(strings.Join(screen(t, m), "\n"))
}

func topLine(m *Model) string { return m.viewer.rendered().Plain[m.viewer.top] }

func onScreen(m *Model, text string) bool {
	r := m.viewer.rendered()
	for i := m.viewer.top; i < min(m.viewer.top+m.bodyHeight(), len(r.Plain)); i++ {
		if strings.Contains(r.Plain[i], text) {
			return true
		}
	}
	return false
}

func writeFiles(t *testing.T, files map[string]string) string {
	t.Helper()
	dir := t.TempDir()
	for name, content := range files {
		p := filepath.Join(dir, name)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

func TestStartupOpensReadmeAndListsTree(t *testing.T) {
	dir := writeFiles(t, map[string]string{
		"README.md":     "# Hello\n\ntext\n",
		"docs/guide.md": "# Guide\n",
		"src/main.go":   "package main\n",
	})
	m := start(t, dir, 100, 20)
	if m.viewer.target == nil || m.viewer.target.Path != "README.md" || !onScreen(m, "Hello") {
		t.Fatalf("README not opened: %+v", m.viewer.target)
	}
	s := plainScreen(t, m)
	for _, want := range []string{"▸ docs", "▸ src", "README.md", " Preview ", " Source "} {
		if !strings.Contains(s, want) {
			t.Errorf("screen lacks %q:\n%s", want, s)
		}
	}

	// Expand docs asynchronously and open guide.md.
	m.tree.setCursor(m.tree.indexOf("docs"))
	drive(t, m, key("l"))
	if i := m.tree.indexOf("docs/guide.md"); i < 0 {
		t.Fatal("docs not expanded")
	} else {
		m.tree.setCursor(i)
	}
	drive(t, m, key("enter"))
	if m.viewer.target.Path != "docs/guide.md" || !onScreen(m, "Guide") {
		t.Errorf("guide not opened: %+v", m.viewer.target)
	}
	// h on a child moves to its parent; h again collapses.
	drive(t, m, key("h"))
	drive(t, m, key("h"))
	if m.tree.indexOf("docs/guide.md") >= 0 || m.tree.rows[m.tree.cursor].node.Rel != "docs" {
		t.Errorf("collapse: cursor %d rows %+v", m.tree.cursor, m.tree.rows)
	}
	drive(t, m, key("backspace"))
	if m.viewer.target.Path != "README.md" {
		t.Errorf("back: %+v", m.viewer.target)
	}
}

func TestFileGivenOnCommandLine(t *testing.T) {
	m := start(t, fixture("markdown", "gfm.md"), 100, 30)
	if m.viewer.target.Path != "gfm.md" || !onScreen(m, "GFM 表示評価") {
		t.Fatalf("target %+v", m.viewer.target)
	}
}

func TestLayoutAcrossResizes(t *testing.T) {
	m := start(t, fixture("markdown", "gfm.md"), 120, 40)
	for _, sz := range [][2]int{{80, 24}, {60, 15}, {40, 10}, {200, 60}} {
		drive(t, m, tea.WindowSizeMsg{Width: sz[0], Height: sz[1]})
		screen(t, m)
	}
	drive(t, m, key("tab"))
	if m.focus != paneViewer {
		t.Error("tab did not move focus to the viewer")
	}
	drive(t, m, key("tab"))
	if m.focus != paneFiles {
		t.Error("tab did not move focus back to the tree")
	}
	drive(t, m, key("e"))
	if m.sideVisible() || m.focus != paneViewer {
		t.Error("e did not hide the tree")
	}
	screen(t, m)
	for _, o := range []string{"?", "t", "/"} {
		drive(t, m, key(o))
		screen(t, m)
		drive(t, m, key("esc"))
	}
}

func TestLinkTOCSearchHistoryResize(t *testing.T) {
	m := start(t, fixture("markdown", "gfm.md"), 100, 30)
	m.focus = paneViewer
	links := m.viewer.doc().Links

	for range len(links) {
		drive(t, m, key(">"))
		if links[m.viewer.linkIdx].Dest == "#data-flow--api-v2" {
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

	// TOC preselects the current section and jumps to the second duplicate.
	drive(t, m, key("t"))
	if m.overlay != overlayTOC {
		t.Fatal("TOC not open")
	}
	target := -1
	for i, h := range m.viewer.doc().Headings {
		if h.ID == "重複見出し-1" {
			target = i
		}
	}
	for m.tocCursor != target {
		if m.tocCursor < target {
			drive(t, m, key("j"))
		} else {
			drive(t, m, key("k"))
		}
	}
	drive(t, m, key("enter"))
	want := m.viewer.rendered().LineOf(preview.Position{Block: m.viewer.doc().Headings[m.tocCursor].Block})
	if want < m.viewer.top || want >= m.viewer.top+m.bodyHeight() {
		t.Fatalf("TOC target line %d not in view", want)
	}

	// Search; the hit stays visible after shrinking the terminal.
	drive(t, m, key("/"))
	typeText(t, m, "書生")
	drive(t, m, key("enter"))
	if len(m.viewer.matches) == 0 {
		t.Fatal("no matches")
	}
	drive(t, m, tea.WindowSizeMsg{Width: 60, Height: 20})
	hit := m.viewer.matches[m.viewer.matchIdx].Line
	if hit < m.viewer.top || hit >= m.viewer.top+m.bodyHeight() {
		t.Errorf("search hit line %d not in view [%d,%d)", hit, m.viewer.top, m.viewer.top+m.bodyHeight())
	}
	if !strings.Contains(plainScreen(t, m), "書生") {
		t.Error("search hit not drawn")
	}
}

func TestSearchInputDoesNotTriggerKeys(t *testing.T) {
	m := start(t, fixture("markdown", "gfm.md"), 100, 30)
	drive(t, m, key("/"))
	typeText(t, m, "qtse?") // all are global keys outside the input
	if m.overlay != overlaySearch || m.input != "qtse?" || m.viewer.target.Mode != ModePreview {
		t.Fatalf("overlay %v input %q", m.overlay, m.input)
	}
	drive(t, m, key("backspace"))
	if m.input != "qtse" {
		t.Errorf("backspace: %q", m.input)
	}
	drive(t, m, key("esc"))
	if m.overlay != overlayNone {
		t.Error("esc did not close the search")
	}
}

func TestPreviewSourceToggleKeepsPosition(t *testing.T) {
	m := start(t, fixture("markdown", "gfm.md"), 100, 20)
	m.focus = paneViewer
	h, _ := m.viewer.doc().HeadingByID("日本語長文")
	m.jumpTo(preview.Position{Block: h.Block})
	drive(t, m, key("s"))
	if m.viewer.target.Mode != ModeSource || !onScreen(m, "## 日本語長文") {
		t.Fatalf("source: top %q", topLine(m))
	}
	drive(t, m, key("s"))
	if m.viewer.target.Mode != ModePreview || !onScreen(m, "日本語長文") {
		t.Fatalf("preview: top %q", topLine(m))
	}
}

func TestCrossDocumentLinkAndBoundary(t *testing.T) {
	m := start(t, fixture("azure-wiki", "Home.md"), 100, 30)
	m.focus = paneViewer
	follow := func(dest string) {
		t.Helper()
		for range len(m.viewer.doc().Links) {
			drive(t, m, key(">"))
			if m.viewer.doc().Links[m.viewer.linkIdx].Dest == dest {
				drive(t, m, key("enter"))
				return
			}
		}
		t.Fatalf("link %q not found", dest)
	}
	follow("/Design/Architecture")
	if m.viewer.target.Path != "Design/Architecture.md" {
		t.Fatalf("path = %q", m.viewer.target.Path)
	}
	drive(t, m, key("backspace"))
	follow("/../secret.md")
	if m.viewer.target.Path != "Home.md" || !strings.Contains(m.status, "outside") {
		t.Errorf("escape link: path=%q status=%q", m.viewer.target.Path, m.status)
	}
	follow("https://example.com/wiki")
	if m.viewer.target.Path != "Home.md" || !strings.Contains(m.status, "not opened") {
		t.Errorf("external link: status=%q", m.status)
	}
}

// Results delivered out of order: only the newest selection is shown.
func TestStaleLoadIgnored(t *testing.T) {
	m := start(t, fixture("azure-wiki", "Home.md"), 100, 30)
	cmdA := m.open(Target{Path: "Getting-Started.md"}, preview.Position{}, nil, true)
	cmdB := m.open(Target{Path: "Unlisted.md"}, preview.Position{}, nil, true)
	resB, resA := cmdB(), cmdA()
	drive(t, m, resB)
	drive(t, m, resA)
	if !strings.Contains(strings.Join(m.viewer.rendered().Plain, "\n"), "Unlisted") {
		t.Error("stale result replaced the current document")
	}
}

// After the current document fails to load, history, link and search keys
// reopen or no-op instead of touching the missing rendering.
func TestKeysAfterLoadError(t *testing.T) {
	dir := writeFiles(t, map[string]string{"a.md": "# A\n\n[x](#b)\n\n## B\n\ntext 書生\n"})
	m := start(t, filepath.Join(dir, "a.md"), 80, 20)
	m.focus = paneViewer
	drive(t, m, key("/"))
	typeText(t, m, "書生")
	drive(t, m, key("enter"))
	drive(t, m, key(">"))
	drive(t, m, key("enter")) // anchor jump: history now holds a.md

	if err := os.Remove(filepath.Join(dir, "a.md")); err != nil {
		t.Fatal(err)
	}
	drive(t, m, tea.WindowSizeMsg{Width: 70, Height: 20}) // reload fails
	if m.viewer.err == nil || !strings.Contains(plainScreen(t, m), "Cannot show a.md") {
		t.Fatal("expected a load error on screen")
	}
	for _, k := range []string{"enter", "n", "N", "t", ">", "s", "/"} {
		drive(t, m, key(k))
		drive(t, m, key("esc"))
	}
	if err := os.WriteFile(filepath.Join(dir, "a.md"), []byte("# A again\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	drive(t, m, key("backspace"))
	if m.viewer.rendered() == nil || !strings.Contains(m.viewer.rendered().Plain[0], "A again") {
		t.Fatalf("back did not reload: %v", m.viewer.err)
	}
	drive(t, m, key("]"))
	if m.viewer.rendered() == nil {
		t.Fatal("forward lost the document")
	}
}

func TestFileStatesOnScreen(t *testing.T) {
	dir := writeFiles(t, map[string]string{
		"empty.txt":           "",
		"bin.dat":             "a\x00b",
		"big.md":              "# big\n\n" + strings.Repeat("word ", preview.MaxPreview/5+10),
		"evil\x1b[2Jname.txt": "x",
	})
	m := start(t, dir, 100, 20)
	open := func(p string) string {
		t.Helper()
		run(t, m, m.open(Target{Path: p}, preview.Position{}, nil, true))
		return plainScreen(t, m)
	}
	if s := open("empty.txt"); !strings.Contains(s, "(empty file)") {
		t.Errorf("empty:\n%s", s)
	}
	if s := open("bin.dat"); !strings.Contains(s, "Binary file (3 bytes)") {
		t.Errorf("binary:\n%s", s)
	}
	if s := open("big.md"); !strings.Contains(s, "shown as source") || !strings.Contains(s, "# big") {
		t.Errorf("big markdown:\n%s", s)
	}
	if s := strings.Join(screen(t, m), "\n"); strings.Contains(s, "\x1b[2J") {
		t.Error("file name escape reached the screen")
	}
	if s := open("missing.md"); !strings.Contains(s, "Cannot show missing.md") {
		t.Errorf("missing:\n%s", s)
	}
}

func TestProgressiveRendering(t *testing.T) {
	var src strings.Builder
	for i := range 400 {
		src.WriteString("## Section " + strings.Repeat("x", i%7) + "\n\nparagraph text\n\n")
	}
	dir := writeFiles(t, map[string]string{"long.md": src.String()})
	ws, err := workspace.Open(filepath.Join(dir, "long.md"))
	if err != nil {
		t.Fatal(err)
	}
	defer ws.Close()
	m := New(ws, Options{Images: preview.ImageOff})
	run(t, m, m.Init())

	// Deliver only the first (partial) result, then check the full pass.
	_, cmd := m.Update(tea.WindowSizeMsg{Width: 80, Height: 20})
	first := cmd().(loadedMsg)
	if first.res.Rendered.Complete {
		t.Fatal("first pass rendered everything")
	}
	_, follow := m.Update(first)
	if !strings.Contains(plainScreen(t, m), "Section") {
		t.Error("partial result not shown")
	}
	run(t, m, follow)
	if !m.viewer.rendered().Complete || len(m.viewer.rendered().BlockLine) != len(m.viewer.doc().Blocks) {
		t.Error("full pass not applied")
	}
}

// A hit near the end cannot scroll to the top, so it sits low on the screen;
// narrowing the terminal lengthens the text above it. Restoring only the top
// position would push it off screen.
func TestFocusStaysVisibleWhenTextAboveGrows(t *testing.T) {
	src := "# T\n\n" + strings.Repeat("長い段落の文章です。", 120) + "\n\nNEEDLE at the end\n"
	dir := writeFiles(t, map[string]string{"a.md": src})
	m := start(t, filepath.Join(dir, "a.md"), 120, 12)
	m.focus = paneViewer
	drive(t, m, key("/"))
	typeText(t, m, "NEEDLE")
	drive(t, m, key("enter"))
	drive(t, m, tea.WindowSizeMsg{Width: 50, Height: 12})
	if !onScreen(m, "NEEDLE") {
		t.Fatalf("hit scrolled away; top %q", topLine(m))
	}
}

func startWithImages(t *testing.T, target string, mode preview.ImageMode, w, h int) (*Model, tea.Cmd) {
	t.Helper()
	isolateGit(t)
	ws, err := workspace.Open(target)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ws.Close() })
	m := New(ws, Options{Images: mode})
	run(t, m, m.Init())
	_, cmd := m.Update(tea.WindowSizeMsg{Width: w, Height: h})
	return m, cmd
}

func TestImagesHalfblocks(t *testing.T) {
	m, cmd := startWithImages(t, fixture("markdown", "media.md"), preview.ImageHalfblocks, 100, 40)
	run(t, m, cmd)
	all := strings.Join(m.viewer.rendered().Plain, "\n")
	if !strings.ContainsAny(all, "▀▄█") {
		t.Error("no half-block image rendered")
	}
	// Mermaid is rendered as code until P2-04.
	if !strings.Contains(all, "sequenceDiagram") || !strings.Contains(all, "後続の段落") {
		t.Error("mermaid source or following text missing")
	}
	for _, sz := range [][2]int{{60, 20}, {140, 50}} {
		drive(t, m, tea.WindowSizeMsg{Width: sz[0], Height: sz[1]})
		screen(t, m)
	}
}

func TestImagesKittyLifecycle(t *testing.T) {
	m, cmd := startWithImages(t, fixture("markdown", "media.md"), preview.ImageKitty, 100, 40)
	msg := cmd().(loadedMsg)
	if len(msg.res.Transmit) != 1 || len(msg.res.KittyIDs) != 1 {
		t.Fatalf("transmits=%d ids=%v", len(msg.res.Transmit), msg.res.KittyIDs)
	}
	drive(t, m, msg)
	first := append([]uint32(nil), m.viewer.kittyIDs...)
	if !strings.Contains(strings.Join(m.viewer.rendered().Lines, "\n"), "\U0010EEEE") {
		t.Error("no placeholders in content")
	}
	m.focus = paneViewer
	for range 10 {
		drive(t, m, key("j"))
		screen(t, m)
	}
	drive(t, m, tea.WindowSizeMsg{Width: 70, Height: 30})
	for _, id := range m.viewer.kittyIDs {
		if id == first[0] {
			t.Errorf("image id %d reused after resize", id)
		}
	}
	// Quitting deletes the images still on screen.
	_, quit := m.Update(key("q"))
	if quit == nil || len(m.viewer.kittyIDs) != 0 {
		t.Error("images not released on quit")
	}
}

// Only text that comes from files, paths and errors is sanitized; the UI's
// own styling must reach the terminal as escape sequences.
func TestChromeKeepsStylingAndSanitizesInput(t *testing.T) {
	dir := writeFiles(t, map[string]string{"evil\x1b[2J.md": "# x\n"})
	m := start(t, filepath.Join(dir, "evil\x1b[2J.md"), 100, 10)
	lines := screen(t, m)
	header := ansi.Strip(lines[0])
	if strings.Contains(header, "␛[4") || strings.Contains(header, "␛[m") {
		t.Errorf("styling shown as text: %q", header)
	}
	if !strings.Contains(header, "evil␛[2J.md") || strings.Contains(lines[0], "\x1b[2J") {
		t.Errorf("file name not sanitized: %q", lines[0])
	}
}
