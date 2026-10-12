package document

import (
	"errors"
	"path/filepath"
	"testing"

	"github.com/halkn/replor/internal/workspace"
)

func openWiki(t *testing.T) *workspace.Workspace {
	t.Helper()
	w, err := workspace.Open(filepath.Join("..", "..", "testdata", "azure-wiki"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { w.Close() })
	return w
}

func TestResolveLinkAzureWikiFixture(t *testing.T) {
	ws := openWiki(t)
	type want struct {
		kind LinkKind
		path string
		frag string
		err  error
	}
	cases := []struct {
		from, href string
		want       want
	}{
		{"Home.md", "/Design/Architecture", want{kind: LinkLocal, path: "Design/Architecture.md"}},
		{"Home.md", "./Getting-Started.md", want{kind: LinkLocal, path: "Getting-Started.md"}},
		{"Home.md", "Design/日本語ページ.md", want{kind: LinkLocal, path: "Design/日本語ページ.md"}},
		{"Home.md", "/Design/%E6%97%A5%E6%9C%AC%E8%AA%9E%E3%83%9A%E3%83%BC%E3%82%B8", want{kind: LinkLocal, path: "Design/日本語ページ.md"}},
		{"Home.md", "/Design/Architecture#data-flow", want{kind: LinkLocal, path: "Design/Architecture.md", frag: "data-flow"}},
		{"Home.md", "#links", want{kind: LinkAnchor, frag: "links"}},
		{"Home.md", "/.attachments/diagram-1.png", want{kind: LinkLocal, path: ".attachments/diagram-1.png"}},
		{"Home.md", "/Design", want{kind: LinkLocal, path: "Design.md"}},
		{"Home.md", "/Folder-Only", want{kind: LinkLocal, path: "Folder-Only"}},
		{"Design/Architecture.md", "../Design", want{kind: LinkLocal, path: "Design.md"}},
		{"Design/Architecture.md", "日本語ページ", want{kind: LinkLocal, path: "Design/日本語ページ.md"}},
		{"Home.md", "https://example.com/wiki", want{kind: LinkExternal}},

		{"Home.md", "/../secret.md", want{err: workspace.ErrOutside}},
		{"Home.md", "/%2E%2E/secret.md", want{err: workspace.ErrOutside}},
		{"Design/Architecture.md", "../../x.md", want{err: workspace.ErrOutside}},
		{"Home.md", "/Design/%252E%252E", want{err: ErrNotFound}},
		{"Home.md", "javascript:alert(1)", want{err: ErrScheme}},
		{"Home.md", "file:///etc/passwd", want{err: ErrScheme}},
		{"Home.md", "data:text/html,x", want{err: ErrScheme}},
		{"Home.md", "//evil.example/x", want{err: ErrScheme}},
		{"Home.md", "https://example.com/\x1b[2J", want{err: ErrScheme}},
		{"Home.md", "/Missing-Page", want{err: ErrNotFound}},
		// Azure stores "API : v2" as API-%3A-v2.md and hyphenates spaces; the
		// mapping is decided in P2, P0 only guarantees no guessing.
		{"Home.md", "/Getting%20Started", want{err: ErrNotFound}},
		{"Home.md", "/Design/API-%3A-v2", want{err: ErrNotFound}},
	}
	for _, c := range cases {
		got, err := ResolveLink(ws, c.from, ".", c.href)
		if c.want.err != nil {
			if !errors.Is(err, c.want.err) {
				t.Errorf("%s -> %q: err = %v, want %v", c.from, c.href, err, c.want.err)
			}
			continue
		}
		if err != nil {
			t.Errorf("%s -> %q: %v", c.from, c.href, err)
			continue
		}
		if got.Kind != c.want.kind || got.Path != c.want.path || got.Fragment != c.want.frag {
			t.Errorf("%s -> %q = %+v, want %+v", c.from, c.href, got, c.want)
		}
	}
}

func TestResolveLinkSubfolderWorkspace(t *testing.T) {
	w, err := workspace.Open(filepath.Join("..", "..", "testdata", "azure-wiki", "Design"))
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()
	// Started on a sub-folder, the wiki root is that folder: "/Home" from the
	// full wiki is not reachable and must not escape upward.
	if _, err := ResolveLink(w, "Architecture.md", ".", "/Home"); !errors.Is(err, ErrNotFound) {
		t.Errorf("/Home = %v", err)
	}
	if _, err := ResolveLink(w, "Architecture.md", ".", "../Home.md"); !errors.Is(err, workspace.ErrOutside) {
		t.Errorf("../Home.md = %v", err)
	}
}
