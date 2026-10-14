// Package git reads repository state through the git CLI under a read-only
// execution contract (see docs/adr/0001-p0-technical-spike.md).
package git

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"sync/atomic"
	"time"

	"github.com/halkn/replor/internal/process"
)

// MinVersion is the oldest git accepted at all: GIT_CONFIG_COUNT (2.31) is
// needed to neutralize filter drivers. LazyFetchVersion (--no-lazy-fetch) is
// required for partial clones because older versions cannot be told not to
// fetch.
var (
	MinVersion       = Version{2, 31, 0}
	LazyFetchVersion = Version{2, 45, 0}
)

const (
	defaultTimeout   = 10 * time.Second
	defaultMaxOutput = 64 << 20
)

// ErrUnavailable marks conditions where Git features are switched off but file
// browsing continues.
var ErrUnavailable = errors.New("git unavailable")

// UnavailableError explains why Git features are disabled.
type UnavailableError struct{ Reason string }

func (e *UnavailableError) Error() string   { return "git unavailable: " + e.Reason }
func (e *UnavailableError) Is(t error) bool { return t == ErrUnavailable }

// Version is a parsed `git version` triple.
type Version [3]int

func (v Version) Less(o Version) bool {
	for i := range v {
		if v[i] != o[i] {
			return v[i] < o[i]
		}
	}
	return false
}

func (v Version) String() string { return fmt.Sprintf("%d.%d.%d", v[0], v[1], v[2]) }

var versionRe = regexp.MustCompile(`^git version (\d+)\.(\d+)(?:\.(\d+))?`)

// ParseVersion accepts vendor suffixes such as "(Apple Git-155)" or ".windows.1".
func ParseVersion(out string) (Version, error) {
	m := versionRe.FindStringSubmatch(strings.TrimSpace(out))
	if m == nil {
		return Version{}, fmt.Errorf("unrecognized git version %q", out)
	}
	var v Version
	for i := range 3 {
		if m[i+1] != "" {
			v[i], _ = strconv.Atoi(m[i+1])
		}
	}
	return v, nil
}

// Git is a located git binary that satisfies MinVersion.
type Git struct {
	Path    string
	Version Version
}

// Detect finds git on PATH and checks its version.
func Detect(ctx context.Context) (*Git, error) {
	path, err := exec.LookPath("git")
	if err != nil {
		return nil, &UnavailableError{Reason: "git is not installed"}
	}
	res, err := process.Run(ctx, process.Spec{Path: path, Args: []string{"version"}, Env: baseEnv, Timeout: defaultTimeout, MaxOutput: 4096})
	if err != nil {
		return nil, &UnavailableError{Reason: "git version failed: " + err.Error()}
	}
	v, err := ParseVersion(string(res.Stdout))
	if err != nil {
		return nil, &UnavailableError{Reason: err.Error()}
	}
	if v.Less(MinVersion) {
		return nil, &UnavailableError{Reason: fmt.Sprintf("git %s is older than the required %s", v, MinVersion)}
	}
	return &Git{Path: path, Version: v}, nil
}

// baseEnv is appended to the process allowlist. LC_ALL=C keeps messages stable;
// the prompt/pager variables cover code paths not reached by the global flags.
var baseEnv = []string{
	"LC_ALL=C",
	"GIT_TERMINAL_PROMPT=0",
	"GIT_PAGER=cat",
	"PAGER=cat",
	"GIT_NO_LAZY_FETCH=1",
	"GIT_OPTIONAL_LOCKS=0",
}

// globalArgs come before every subcommand. Each one closes an execution or
// write path that read-only commands otherwise take:
//   - --no-optional-locks: `git status` otherwise refreshes .git/index
//   - core.fsmonitor=false: a hook path in config is executed by status/diff
//   - log.showSignature=false: would run gpg.program from `git log`
//   - core.quotePath=false: keep raw bytes; callers use -z for machine output
func (g *Git) globalArgs() []string {
	args := []string{
		"--no-pager",
		"--no-optional-locks",
		"--literal-pathspecs",
		"-c", "core.fsmonitor=false",
		"-c", "log.showSignature=false",
		"-c", "core.quotePath=false",
		"-c", "color.ui=false",
	}
	if !g.Version.Less(LazyFetchVersion) {
		args = append(args, "--no-lazy-fetch")
	}
	return args
}

// Repo is a git repository reached from a workspace directory.
type Repo struct {
	git      *Git
	dir      string
	Toplevel string
	// Prefix is the workspace directory relative to Toplevel ("" or "sub/dir/").
	Prefix string

	revision atomic.Uint64
	// sem runs the commands of one repository one at a time (DESIGN 7.3):
	// a cancelled command may still be exiting when the next one starts.
	sem chan struct{}
}

// Open resolves the repository containing dir. A directory outside any
// repository yields ErrNotRepository.
func (g *Git) Open(ctx context.Context, dir string) (*Repo, error) {
	r := &Repo{git: g, dir: dir, sem: make(chan struct{}, 1)}
	out, err := r.run(ctx, nil, "rev-parse", "--is-inside-work-tree")
	if err != nil {
		var pe *process.Error
		if errors.As(err, &pe) {
			switch {
			case strings.Contains(pe.Stderr, "not a git repository"):
				return nil, ErrNotRepository
			case strings.Contains(pe.Stderr, "dubious ownership"):
				return nil, &UnavailableError{Reason: "repository is owned by another user (safe.directory)"}
			}
		}
		return nil, err
	}
	if strings.TrimSpace(string(out)) != "true" {
		return nil, ErrNotRepository
	}
	// One value per call: rev-parse separates values with newlines, which
	// may also occur inside the paths themselves.
	if r.Toplevel, err = r.revParsePath(ctx, "--show-toplevel"); err != nil {
		return nil, err
	}
	if r.Prefix, err = r.revParsePath(ctx, "--show-prefix"); err != nil {
		return nil, err
	}
	if g.Version.Less(LazyFetchVersion) {
		partial, err := r.isPartialClone(ctx)
		if err != nil {
			return nil, err
		}
		if partial {
			return nil, &UnavailableError{Reason: fmt.Sprintf("partial clone requires git %s or later to prevent on-demand fetches", LazyFetchVersion)}
		}
	}
	return r, nil
}

func (r *Repo) revParsePath(ctx context.Context, flag string) (string, error) {
	out, err := r.run(ctx, nil, "rev-parse", flag)
	if err != nil {
		return "", err
	}
	return strings.TrimSuffix(string(out), "\n"), nil
}

// ErrNotRepository means the workspace is not inside a git work tree.
var ErrNotRepository = errors.New("not a git repository")

// Run executes a read-only git subcommand in the workspace directory.
// Callers must pass diff-family commands with --no-ext-diff and --no-textconv.
func (r *Repo) Run(ctx context.Context, args ...string) ([]byte, error) {
	names, err := r.filters(ctx)
	if err != nil {
		return nil, err
	}
	return r.run(ctx, neutralizeFilters(names), args...)
}

// filters lists configured filter driver names. Attributes from the work tree,
// info/attributes or core.attributesFile can select any of them, and status
// and diff then run the driver's clean command, so every configured driver is
// overridden rather than relying on which attribute files are read. The list
// is read again for every command because config can change at any time.
func (r *Repo) filters(ctx context.Context) ([]string, error) {
	out, err := r.run(ctx, nil, "config", "-z", "--name-only", "--get-regexp", `^filter\.`)
	if err != nil {
		var pe *process.Error
		// exit 1 = no matching keys
		if !errors.As(err, &pe) || pe.ExitCode != 1 {
			return nil, err
		}
	}
	seen := map[string]bool{}
	names := []string{}
	for key := range bytes.SplitSeq(out, []byte{0}) {
		k := string(bytes.TrimSpace(key))
		if k == "" {
			continue
		}
		// filter.<name>.<var>; <name> may itself contain dots
		i, j := strings.Index(k, "."), strings.LastIndex(k, ".")
		if i < 0 || j <= i {
			continue
		}
		name := k[i+1 : j]
		if !seen[name] {
			seen[name] = true
			names = append(names, name)
		}
	}
	return names, nil
}

// neutralizeFilters returns GIT_CONFIG_COUNT pairs instead of -c options
// because a subsection name may contain "=", which -c cannot express.
func neutralizeFilters(names []string) []string {
	env := []string{}
	i := 0
	for _, n := range names {
		for _, kv := range [][2]string{{"clean", ""}, {"smudge", ""}, {"process", ""}, {"required", "false"}} {
			env = append(env,
				fmt.Sprintf("GIT_CONFIG_KEY_%d=filter.%s.%s", i, n, kv[0]),
				fmt.Sprintf("GIT_CONFIG_VALUE_%d=%s", i, kv[1]))
			i++
		}
	}
	return append(env, fmt.Sprintf("GIT_CONFIG_COUNT=%d", i))
}

func (r *Repo) isPartialClone(ctx context.Context) (bool, error) {
	out, err := r.run(ctx, nil, "config", "-z", "--get-regexp", `^(remote\..*\.promisor|extensions\.partialclone)$`)
	if err != nil {
		var pe *process.Error
		if errors.As(err, &pe) && pe.ExitCode == 1 {
			return false, nil
		}
		return false, err
	}
	return len(bytes.TrimSpace(out)) > 0, nil
}

func (r *Repo) run(ctx context.Context, extraEnv []string, args ...string) ([]byte, error) {
	select {
	case r.sem <- struct{}{}:
		defer func() { <-r.sem }()
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	res, err := process.Run(ctx, process.Spec{
		Path:      r.git.Path,
		Args:      append(r.git.globalArgs(), args...),
		Dir:       r.dir,
		Env:       append(append([]string{}, baseEnv...), extraEnv...),
		Timeout:   defaultTimeout,
		MaxOutput: defaultMaxOutput,
	})
	return res.Stdout, err
}

// FromGit converts a toplevel-relative path from git output to a
// workspace-relative one. ok is false when the path lies outside the
// workspace (e.g. the other side of a rename across the boundary).
func (r *Repo) FromGit(p string) (rel string, ok bool) {
	if r.Prefix == "" {
		return p, true
	}
	rel, ok = strings.CutPrefix(p, r.Prefix)
	return rel, ok && rel != ""
}

// ToGit converts a workspace-relative slash path to a toplevel-relative one.
func (r *Repo) ToGit(rel string) string { return r.Prefix + rel }
