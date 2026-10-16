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
	"time"

	"github.com/halkn/replor/internal/workspace"
)

// racyWindow: an entry modified this recently may be modified again within
// the same mtime tick (1-2 s on HFS+, FAT) without changing its size, so it
// is reported as changed until it is older (as git treats "racily clean"
// index entries).
const racyWindow = 2 * time.Second

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
	racy   bool
}

func sigOf(info fs.FileInfo, err error, now time.Time) sig {
	if err != nil {
		return sig{}
	}
	age := now.Sub(info.ModTime())
	return sig{exists: true, dir: info.IsDir(), size: info.Size(), mtime: info.ModTime().UnixNano(), racy: age >= 0 && age < racyWindow}
}

// changed compares two signatures of one path. A racy old signature cannot
// prove that nothing changed.
func changed(old, cur sig) bool {
	return old.racy || old.exists != cur.exists || old.dir != cur.dir || old.size != cur.size || old.mtime != cur.mtime
}

type dirSig struct {
	exists   bool
	entries  map[string]sig
	complete bool // all entries were read (not cut by the limit)
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

// Scan stats the set. Directory entries are not followed (Lstat), ".git" is
// skipped like in the tree, and at most maxEntries entries are read in
// total; directories after that are not read at all.
func Scan(ws *workspace.Workspace, set Set, maxEntries int) *Snapshot {
	now := time.Now()
	s := &Snapshot{files: map[string]sig{}, dirs: map[string]dirSig{}, git: map[string]sig{}}
	for _, f := range set.Files {
		info, err := ws.Stat(f)
		s.files[f] = sigOf(info, err, now)
	}
	budget := maxEntries
	for _, d := range set.Dirs {
		ds := dirSig{entries: map[string]sig{}}
		if budget <= 0 {
			s.Limited = true
			s.dirs[d] = ds
			continue
		}
		// One more than the budget tells a cut from an exact fit.
		entries, err := ws.ReadDirN(d, budget+1)
		ds.exists = err == nil
		ds.complete = len(entries) <= budget
		for _, e := range entries {
			if budget <= 0 {
				s.Limited = true
				break
			}
			if e.Name() == ".git" {
				continue
			}
			budget--
			info, err := e.Info()
			ds.entries[e.Name()] = sigOf(info, err, now)
		}
		s.dirs[d] = ds
	}
	for _, g := range set.Git {
		info, err := os.Stat(g)
		s.git[g] = sigOf(info, err, now)
	}
	return s
}

// Merge records seed's signatures as the baseline for their paths. Loaders
// take the seed just before reading, so a change after the read shows up in
// the next Compare even if the path was not watched before.
func (s *Snapshot) Merge(seed *Snapshot) {
	for k, v := range seed.files {
		s.files[k] = v
	}
	for k, v := range seed.dirs {
		s.dirs[k] = v
	}
	for k, v := range seed.git {
		s.git[k] = v
	}
}

// Changes lists what differs between two scans. Paths watched in only one
// of them are not reported.
type Changes struct {
	Files []string
	// Listed are directories whose entry names changed (to be listed
	// again); Modified are directories where only entry contents changed
	// (which may change the git status, not the listing).
	Listed, Modified []string
	Git              bool
}

func (c Changes) Empty() bool {
	return len(c.Files) == 0 && len(c.Listed) == 0 && len(c.Modified) == 0 && !c.Git
}

// Compare returns the changes from old to cur.
func Compare(old, cur *Snapshot) Changes {
	var ch Changes
	if old == nil || cur == nil {
		return ch
	}
	for f, s := range cur.files {
		if o, ok := old.files[f]; ok && changed(o, s) {
			ch.Files = append(ch.Files, f)
		}
	}
	for d, s := range cur.dirs {
		o, ok := old.dirs[d]
		if !ok {
			continue
		}
		names, contents := diffEntries(o, s)
		switch {
		case o.exists != s.exists || names:
			ch.Listed = append(ch.Listed, d)
		case contents:
			ch.Modified = append(ch.Modified, d)
		}
	}
	for g, s := range cur.git {
		if o, ok := old.git[g]; ok && changed(o, s) {
			ch.Git = true
		}
	}
	return ch
}

// diffEntries reports whether names were added or removed and whether a
// common entry changed. Names are only compared when both reads were
// complete; a cut read differs by the cut.
func diffEntries(a, b dirSig) (names, contents bool) {
	both := a.complete && b.complete
	if both && len(a.entries) != len(b.entries) {
		names = true
	}
	for name, s := range b.entries {
		o, ok := a.entries[name]
		switch {
		case !ok && both:
			names = true
		case ok && changed(o, s):
			contents = true
		}
	}
	return names, contents
}

// Unknown records git files whose state when they were read is not known
// (a ref found by the read itself): merged as a seed, they compare as
// changed once, so that a change before the next scan is not missed.
func Unknown(git []string) *Snapshot {
	s := &Snapshot{files: map[string]sig{}, dirs: map[string]dirSig{}, git: map[string]sig{}}
	for _, g := range git {
		s.git[g] = sig{racy: true}
	}
	return s
}
