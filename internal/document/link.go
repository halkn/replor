// Package document holds Markdown document structure: links, anchors and
// dialect-specific navigation.
package document

import (
	"errors"
	"fmt"
	"io/fs"
	"net/url"
	"path"
	"strings"
	"unicode"

	"github.com/halkn/replor/internal/workspace"
)

// LinkKind classifies a resolved link target.
type LinkKind int

const (
	LinkLocal    LinkKind = iota // file inside the workspace
	LinkAnchor                   // fragment within the current document
	LinkExternal                 // http(s) URL, opened only on explicit request
)

// Link is a resolved link. Path is workspace-relative for LinkLocal.
type Link struct {
	Kind     LinkKind
	Path     string
	Fragment string
	URL      string
}

var (
	ErrScheme    = errors.New("link scheme is not allowed")
	ErrNotFound  = errors.New("link target does not exist")
	ErrAmbiguous = errors.New("link target is ambiguous")
)

// Statter is the subset of the workspace used for existence checks.
type Statter interface {
	Stat(rel string) (fs.FileInfo, error)
}

// ResolveLink resolves href found in the document fromDoc. wikiRoot is the
// base for root-relative ("/...") links; both are workspace-relative.
// The path is percent-decoded exactly once before the boundary check, so
// "%2E%2E" is treated as ".." and "%252E" stays a literal "%2E".
func ResolveLink(ws Statter, fromDoc, wikiRoot, href string) (Link, error) {
	if strings.IndexFunc(href, unicode.IsControl) >= 0 {
		return Link{}, fmt.Errorf("%w: control character", ErrScheme)
	}
	rawPath, rawFrag, _ := strings.Cut(href, "#")
	frag, err := url.PathUnescape(rawFrag)
	if err != nil {
		return Link{}, err
	}

	if scheme, _, ok := strings.Cut(rawPath, ":"); ok && isScheme(scheme) {
		switch strings.ToLower(scheme) {
		case "http", "https":
			u, err := url.Parse(href)
			if err != nil || u.Host == "" {
				return Link{}, fmt.Errorf("%w: %q", ErrScheme, href)
			}
			return Link{Kind: LinkExternal, URL: u.String()}, nil
		}
		return Link{}, fmt.Errorf("%w: %q", ErrScheme, scheme)
	}
	if strings.HasPrefix(rawPath, "//") {
		return Link{}, fmt.Errorf("%w: protocol-relative URL", ErrScheme)
	}

	rawPath, _, _ = strings.Cut(rawPath, "?")
	if rawPath == "" {
		return Link{Kind: LinkAnchor, Fragment: frag}, nil
	}
	p, err := url.PathUnescape(rawPath)
	if err != nil {
		return Link{}, err
	}
	var joined string
	if strings.HasPrefix(p, "/") {
		joined = path.Join(wikiRoot, p[1:])
	} else {
		joined = path.Join(path.Dir(fromDoc), p)
	}
	rel, err := workspace.Clean(joined)
	if err != nil {
		return Link{}, err
	}

	target, err := pickTarget(ws, rel)
	if err != nil {
		return Link{}, err
	}
	return Link{Kind: LinkLocal, Path: target, Fragment: frag}, nil
}

// pickTarget maps an extensionless wiki page link to Page.md. A directory
// with the same name holds the page's sub-pages, so Page.md wins over Page/;
// a regular file named exactly like the link alongside Page.md is ambiguous.
func pickTarget(ws Statter, rel string) (string, error) {
	info, err := ws.Stat(rel)
	exact := err == nil
	if err != nil && !errors.Is(err, fs.ErrNotExist) {
		return "", err
	}
	if path.Ext(rel) == ".md" {
		if !exact {
			return "", fmt.Errorf("%w: %s", ErrNotFound, rel)
		}
		return rel, nil
	}
	_, err = ws.Stat(rel + ".md")
	page := err == nil
	if err != nil && !errors.Is(err, fs.ErrNotExist) {
		return "", err
	}
	switch {
	case page && exact && !info.IsDir():
		return "", fmt.Errorf("%w: %s and %s.md", ErrAmbiguous, rel, rel)
	case page:
		return rel + ".md", nil
	case exact:
		return rel, nil
	}
	return "", fmt.Errorf("%w: %s", ErrNotFound, rel)
}

func isScheme(s string) bool {
	if s == "" || !isAlpha(s[0]) {
		return false
	}
	for i := 1; i < len(s); i++ {
		c := s[i]
		if !isAlpha(c) && !('0' <= c && c <= '9') && c != '+' && c != '-' && c != '.' {
			return false
		}
	}
	return true
}

func isAlpha(c byte) bool { return 'a' <= c|0x20 && c|0x20 <= 'z' }
