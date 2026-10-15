// Package watch detects changes by comparing stat signatures between scans.
//
// Polling a small set (what is on screen and git's metadata files) needs no
// file descriptors or kernel watch limits and works on any file system,
// including network mounts such as WSL's /mnt/c. Changes outside the set
// are not seen; the UI offers an explicit reload for those (DESIGN 9.1).
package watch

import (
	"io/fs"
	"os"

	"github.com/halkn/replor/internal/workspace"
)

// Set is what one scan looks at.
type Set struct {
	Files []string // workspace paths of files shown in the viewer
	Dirs  []string // workspace paths of listed (expanded) directories
	Git   []string // absolute paths of git metadata files (index, HEAD, refs)
}

type sig struct {
	exists bool
	dir    bool
	size   int64
	mtime  int64
}

func sigOf(info fs.FileInfo, err error) sig {
	if err != nil {
		return sig{}
	}
	return sig{exists: true, dir: info.IsDir(), size: info.Size(), mtime: info.ModTime().UnixNano()}
}

type dirSig struct {
	self    sig
	entries map[string]sig
}

// Snapshot is the result of one scan.
type Snapshot struct {
	files map[string]sig
	dirs  map[string]dirSig
	git   map[string]sig
	// Limited reports that the entry limit cut the scan: entries beyond it
	// are not compared, so edits there go unnoticed.
	Limited bool
}

// Scan stats the set. Directory entries are not followed (Lstat), and at
// most maxEntries entries are looked at in total.
func Scan(ws *workspace.Workspace, set Set, maxEntries int) *Snapshot {
	s := &Snapshot{files: map[string]sig{}, dirs: map[string]dirSig{}, git: map[string]sig{}}
	for _, f := range set.Files {
		s.files[f] = sigOf(ws.Stat(f))
	}
	budget := maxEntries
	for _, d := range set.Dirs {
		ds := dirSig{self: sigOf(ws.Stat(d)), entries: map[string]sig{}}
		entries, _ := ws.ReadDir(d)
		for _, e := range entries {
			if budget <= 0 {
				s.Limited = true
				break
			}
			budget--
			ds.entries[e.Name()] = sigOf(e.Info())
		}
		s.dirs[d] = ds
	}
	for _, g := range set.Git {
		s.git[g] = sigOf(os.Stat(g))
	}
	return s
}

// Changes lists what differs between two scans. Paths watched in only one
// of them (the set changed) are not reported.
type Changes struct {
	Files []string
	Dirs  []string
	Git   bool
}

func (c Changes) Empty() bool { return len(c.Files) == 0 && len(c.Dirs) == 0 && !c.Git }

// Compare returns the changes from old to cur.
func Compare(old, cur *Snapshot) Changes {
	var ch Changes
	if old == nil || cur == nil {
		return ch
	}
	for f, s := range cur.files {
		if o, ok := old.files[f]; ok && o != s {
			ch.Files = append(ch.Files, f)
		}
	}
	for d, s := range cur.dirs {
		o, ok := old.dirs[d]
		if ok && (o.self != s.self || entriesDiffer(o.entries, s.entries, old.Limited || cur.Limited)) {
			ch.Dirs = append(ch.Dirs, d)
		}
	}
	for g, s := range cur.git {
		if o, ok := old.git[g]; ok && o != s {
			ch.Git = true
		}
	}
	return ch
}

// entriesDiffer compares directory entries. When a scan was cut short the
// sets of names differ by the cut, so only common names are compared.
func entriesDiffer(a, b map[string]sig, limited bool) bool {
	if !limited && len(a) != len(b) {
		return true
	}
	for name, s := range b {
		o, ok := a[name]
		if ok && o != s || !ok && !limited {
			return true
		}
	}
	return false
}
