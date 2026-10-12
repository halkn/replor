package workspace

import (
	"errors"
	"io/fs"
	"path"
	"sort"
)

// NodeKind is what a directory entry resolves to inside the workspace.
type NodeKind int

const (
	NodeFile NodeKind = iota
	NodeDir
	// NodeBroken cannot be opened: a dangling, escaping or absolute
	// symbolic link (os.Root refuses those), or a special file.
	NodeBroken
)

// Node is one entry of a directory listing.
type Node struct {
	Rel     string // workspace-relative slash path
	Name    string // raw file name; sanitize before display
	Kind    NodeKind
	Symlink bool
	Reason  string // why a NodeBroken entry cannot be opened
}

// ListDir lists a directory for the tree: ".git" is hidden, symbolic links
// are resolved within the boundary, directories come first, then names in
// byte order. max > 0 caps the number of entries; truncated reports a cut.
func (w *Workspace) ListDir(rel string, max int) (nodes []Node, truncated bool, err error) {
	entries, err := w.ReadDir(rel)
	if err != nil {
		return nil, false, err
	}
	for _, e := range entries {
		if e.Name() == ".git" {
			continue
		}
		n := Node{Rel: path.Join(rel, e.Name()), Name: e.Name()}
		switch {
		case e.Type()&fs.ModeSymlink != 0:
			n.Symlink = true
			info, err := w.Stat(n.Rel)
			switch {
			case err == nil && info.IsDir():
				n.Kind = NodeDir
			case err == nil && info.Mode().IsRegular():
				n.Kind = NodeFile
			case err == nil:
				n.Kind, n.Reason = NodeBroken, "link target is not a regular file"
			case errors.Is(err, ErrOutside):
				n.Kind, n.Reason = NodeBroken, "link points outside the workspace"
			default:
				n.Kind, n.Reason = NodeBroken, "link cannot be followed"
			}
		case e.IsDir():
			n.Kind = NodeDir
		case e.Type().IsRegular():
			n.Kind = NodeFile
		default:
			// Devices, sockets and FIFOs: reading them could block or have
			// side effects.
			n.Kind, n.Reason = NodeBroken, "not a regular file"
		}
		nodes = append(nodes, n)
	}
	sort.SliceStable(nodes, func(i, j int) bool {
		if (nodes[i].Kind == NodeDir) != (nodes[j].Kind == NodeDir) {
			return nodes[i].Kind == NodeDir
		}
		return nodes[i].Name < nodes[j].Name
	})
	if max > 0 && len(nodes) > max {
		return nodes[:max], true, nil
	}
	return nodes, false, nil
}
