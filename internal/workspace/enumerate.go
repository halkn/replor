package workspace

import (
	"context"
	"errors"
	"io/fs"
)

// Limits bound a path enumeration. Zero means unlimited.
type Limits struct {
	MaxEntries int
	MaxDepth   int
}

// Enumeration outcome reasons.
const (
	Complete       = ""
	LimitedEntries = "entry limit reached"
	LimitedDepth   = "depth limit reached"
)

var errStop = errors.New("stop")

// Enumerate walks the workspace and reports file paths (slash-separated,
// workspace-relative) in batches. Symbolic links are reported but never
// descended, so link cycles cannot loop. skip, if set, prunes directories
// such as .git. The returned reason tells the caller whether the result is
// partial.
func (w *Workspace) Enumerate(ctx context.Context, lim Limits, skip func(rel string) bool, batch func([]string)) (reason string, err error) {
	const batchSize = 512
	buf := make([]string, 0, batchSize)
	count := 0
	flush := func() {
		if len(buf) > 0 {
			batch(buf)
			buf = make([]string, 0, batchSize)
		}
	}
	err = fs.WalkDir(w.root.FS(), ".", func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			// Unreadable entries are skipped; the walk continues.
			if d != nil && d.IsDir() {
				return fs.SkipDir
			}
			return nil
		}
		if count%256 == 0 && ctx.Err() != nil {
			return ctx.Err()
		}
		if p == "." {
			return nil
		}
		if skip != nil && skip(p) {
			if d.IsDir() {
				return fs.SkipDir
			}
			return nil
		}
		if d.IsDir() {
			if lim.MaxDepth > 0 && depth(p) >= lim.MaxDepth {
				reason = LimitedDepth
				return fs.SkipDir
			}
			return nil
		}
		if lim.MaxEntries > 0 && count >= lim.MaxEntries {
			reason = LimitedEntries
			return errStop
		}
		count++
		buf = append(buf, p)
		if len(buf) == batchSize {
			flush()
		}
		return nil
	})
	flush()
	if errors.Is(err, errStop) {
		err = nil
	}
	return reason, err
}

func depth(p string) int {
	n := 1
	for i := 0; i < len(p); i++ {
		if p[i] == '/' {
			n++
		}
	}
	return n
}
