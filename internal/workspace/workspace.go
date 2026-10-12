// Package workspace fixes the read boundary of a session and gives access to
// files only through it.
package workspace

import (
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path"
	"path/filepath"
	"strings"
)

// ErrOutside is returned for any path that leaves the workspace root,
// lexically or through a symbolic link.
var ErrOutside = errors.New("path is outside the workspace")

// ErrTooLarge is returned by ReadFile when the file exceeds the given limit.
var ErrTooLarge = errors.New("file exceeds the read limit")

// Workspace is the directory the session was started on. Its boundary never
// widens after Open.
type Workspace struct {
	// Root is absolute with symbolic links resolved.
	Root string
	// Selected is the workspace-relative slash path of a file given on the
	// command line, or "".
	Selected string

	root *os.Root
}

// Open resolves arg (a directory, or a file whose parent becomes the root).
func Open(arg string) (*Workspace, error) {
	if arg == "" {
		arg = "."
	}
	abs, err := filepath.Abs(arg)
	if err != nil {
		return nil, err
	}
	resolved, err := filepath.EvalSymlinks(abs)
	if err != nil {
		return nil, err
	}
	info, err := os.Stat(resolved)
	if err != nil {
		return nil, err
	}
	w := &Workspace{Root: resolved}
	if !info.IsDir() {
		// The selection is taken from the name as given, not from the link
		// target, so `replor link.md` selects link.md next to it.
		w.Root, err = filepath.EvalSymlinks(filepath.Dir(abs))
		if err != nil {
			return nil, err
		}
		w.Selected = filepath.Base(abs)
	}
	w.root, err = os.OpenRoot(w.Root)
	if err != nil {
		return nil, err
	}
	if w.Selected != "" {
		if _, err := w.Stat(w.Selected); err != nil {
			w.root.Close()
			return nil, err
		}
	}
	return w, nil
}

// Close releases the root handle.
func (w *Workspace) Close() error { return w.root.Close() }

// Clean validates a workspace-relative slash path and returns its canonical
// form ("." for the root). Absolute paths, ".." escapes, backslashes and NUL
// are rejected lexically before any file system access.
func Clean(rel string) (string, error) {
	if rel == "" || strings.ContainsAny(rel, "\x00\\") || path.IsAbs(rel) {
		return "", fmt.Errorf("%w: %q", ErrOutside, rel)
	}
	c := path.Clean(rel)
	if !filepath.IsLocal(filepath.FromSlash(c)) && c != "." {
		return "", fmt.Errorf("%w: %q", ErrOutside, rel)
	}
	return c, nil
}

func (w *Workspace) check(rel string) (string, error) {
	c, err := Clean(rel)
	if err != nil {
		return "", err
	}
	return filepath.FromSlash(c), nil
}

// Stat follows symbolic links that stay inside the root.
func (w *Workspace) Stat(rel string) (fs.FileInfo, error) {
	p, err := w.check(rel)
	if err != nil {
		return nil, err
	}
	info, err := w.root.Stat(p)
	return info, wrap(err)
}

// ReadDir lists a directory inside the root.
func (w *Workspace) ReadDir(rel string) ([]fs.DirEntry, error) {
	p, err := w.check(rel)
	if err != nil {
		return nil, err
	}
	f, err := w.root.Open(p)
	if err != nil {
		return nil, wrap(err)
	}
	defer f.Close()
	entries, err := f.ReadDir(-1)
	return entries, wrap(err)
}

// ReadFile reads at most max bytes; larger files return ErrTooLarge without
// being read.
func (w *Workspace) ReadFile(rel string, max int64) ([]byte, error) {
	p, err := w.check(rel)
	if err != nil {
		return nil, err
	}
	// Opening a FIFO or device can block or have side effects, so the type is
	// checked before open; the check after open guards against a swap.
	if info, err := w.root.Stat(p); err != nil {
		return nil, wrap(err)
	} else if !info.Mode().IsRegular() {
		return nil, fmt.Errorf("%s: not a regular file", rel)
	}
	f, err := w.root.Open(p)
	if err != nil {
		return nil, wrap(err)
	}
	defer f.Close()
	info, err := f.Stat()
	if err != nil {
		return nil, err
	}
	if !info.Mode().IsRegular() {
		return nil, fmt.Errorf("%s: not a regular file", rel)
	}
	if info.Size() > max {
		return nil, fmt.Errorf("%w: %s (%d > %d bytes)", ErrTooLarge, rel, info.Size(), max)
	}
	// The file may grow after Stat; never read past the limit.
	data, err := io.ReadAll(io.LimitReader(f, max+1))
	if err != nil {
		return nil, err
	}
	if int64(len(data)) > max {
		return nil, fmt.Errorf("%w: %s", ErrTooLarge, rel)
	}
	return data, nil
}

// os.Root reports escapes as a *PathError whose message is "path escapes
// from parent"; there is no exported sentinel, so it is mapped here.
func wrap(err error) error {
	if err == nil {
		return nil
	}
	var pe *fs.PathError
	if errors.As(err, &pe) && strings.Contains(pe.Err.Error(), "escapes") {
		return fmt.Errorf("%w: %s", ErrOutside, pe.Path)
	}
	return err
}
