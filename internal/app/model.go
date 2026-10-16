package app

import (
	"context"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/git"
	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/workspace"
)

// Mode is how the viewer shows a file.
type Mode int

const (
	ModePreview Mode = iota // rendered Markdown, image, or source for other text
	ModeSource              // Markdown as source
	ModeDiff                // a git diff of the file (Scope)
)

// Target is what the viewer shows. It keys history entries and async loads.
type Target struct {
	Path  string
	Mode  Mode
	Scope git.Scope // ModeDiff only
}

// HistoryEntry is one back/forward step.
type HistoryEntry struct {
	Target Target
	Pos    preview.Position
	// Focus is the position the reader was attending to (search hit,
	// selected link, jump target), kept on screen when restored.
	Focus *preview.Position
}

// Options are the command-line choices that affect the UI.
type Options struct {
	Images preview.ImageMode
	// Watch reloads what is on screen when files or git state change.
	Watch bool
	// Base is the branch Branch Changes compare with; "" looks it up
	// (git.DefaultBaseRef).
	Base string
}

type pane int

const (
	paneFiles pane = iota
	paneViewer
)

type overlay int

const (
	overlayNone overlay = iota
	overlaySearch
	overlayTOC
	overlayHelp
	overlayFinder
)

// Model is the Bubble Tea model of the application.
type Model struct {
	ws     *workspace.Workspace
	opts   Options
	latest *Latest

	width, height int
	focus         pane
	hideSide      bool

	tree    tree
	viewer  viewer
	finder  finder
	git     gitState
	left    leftMode
	changes changesState
	watch   watchState
	// showIgnored also lists untracked files that git ignores.
	showIgnored bool

	back, fwd []HistoryEntry

	overlay   overlay
	input     string
	tocCursor int

	status string
}

// New creates the model for a workspace. A file given on the command line
// (ws.Selected) is opened once the terminal size is known.
func New(ws *workspace.Workspace, opts Options) *Model {
	m := &Model{ws: ws, opts: opts, latest: NewLatest()}
	m.viewer.linkIdx, m.viewer.sourceLine = -1, -1
	if ws.Selected != "" {
		m.viewer.target = &Target{Path: ws.Selected}
	}
	return m
}

func (m *Model) Init() tea.Cmd {
	return tea.Batch(m.listDir("."), m.loadGit(), m.scheduleWatch())
}

func (m *Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = msg.Width, msg.Height
		return m, m.relayout()
	case dirListedMsg:
		return m, m.applyDir(msg)
	case loadedMsg:
		return m, m.applyLoad(msg)
	case finderBatchMsg:
		return m, m.applyFinderBatch(msg)
	case gitMsg:
		return m, m.applyGit(msg)
	case branchMsg:
		return m, m.applyBranch(msg)
	case watchTickMsg:
		return m, m.scan()
	case watchScannedMsg:
		return m, m.applyScan(msg)
	case finderRankedMsg:
		m.applyRanked(msg)
		return m, nil
	case tea.KeyPressMsg:
		return m.key(msg)
	}
	return m, nil
}

// issue starts an async request for a display target; see Latest.
func (m *Model) issue(key string) (Ticket, context.Context) {
	return m.latest.Issue(context.Background(), key)
}

// Layout geometry. The side pane hides itself on narrow terminals.

const (
	minWidthForSide = 70
	chromeRows      = 2 // header + footer
)

func (m *Model) sideVisible() bool { return !m.hideSide && m.width >= minWidthForSide }

func (m *Model) sideWidth() int { return min(32, m.width/3) }

func (m *Model) bodyHeight() int { return max(m.height-chromeRows, 1) }

// viewerWidth is the text width given to renderers: the viewer column minus
// the left padding and the gutter.
func (m *Model) viewerWidth() int {
	w := m.width
	if m.sideVisible() {
		w -= m.sideWidth() + 1
	}
	return max(w-2, 10)
}
