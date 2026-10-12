// Package process runs external commands with a fixed environment, a timeout
// and a bounded output size.
package process

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"
)

// ErrOutputLimit is returned when a command writes more than Spec.MaxOutput bytes.
var ErrOutputLimit = errors.New("process: output limit exceeded")

// inheritedEnv lists the variables passed through from the caller. Everything
// else (notably GIT_*, SSH_*, *_ASKPASS) is dropped so that the caller's shell
// cannot redirect the target repository, index or config of a child process.
var inheritedEnv = []string{
	"PATH", "HOME", "USER", "LOGNAME", "XDG_CONFIG_HOME", "TMPDIR", "SYSTEMROOT",
}

// Spec describes one command invocation.
type Spec struct {
	Path      string
	Args      []string
	Dir       string
	Env       []string // KEY=VALUE pairs added after the inherited allowlist
	Timeout   time.Duration
	MaxOutput int64
}

// Result is the outcome of a finished command.
type Result struct {
	Stdout   []byte
	Stderr   []byte
	ExitCode int
}

// Error wraps a non-zero exit with the captured stderr.
type Error struct {
	Args     []string
	ExitCode int
	Stderr   string
}

func (e *Error) Error() string {
	return fmt.Sprintf("%s: exit %d: %s", strings.Join(e.Args, " "), e.ExitCode, strings.TrimSpace(e.Stderr))
}

// Env builds the child environment from the allowlist and extra pairs.
func Env(extra ...string) []string {
	env := make([]string, 0, len(inheritedEnv)+len(extra))
	for _, k := range inheritedEnv {
		if v, ok := os.LookupEnv(k); ok {
			env = append(env, k+"="+v)
		}
	}
	return append(env, extra...)
}

// Run executes spec. A non-zero exit returns *Error together with the Result.
func Run(ctx context.Context, spec Spec) (Result, error) {
	if spec.Timeout > 0 {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, spec.Timeout)
		defer cancel()
	}
	cmd := exec.CommandContext(ctx, spec.Path, spec.Args...)
	cmd.Dir = spec.Dir
	cmd.Env = Env(spec.Env...)
	cmd.Stdin = nil
	// Children that ignore the kill (or hold the pipes open) must not block the caller.
	cmd.WaitDelay = time.Second

	stdout := &limitedBuffer{max: spec.MaxOutput}
	stderr := &limitedBuffer{max: 64 << 10}
	cmd.Stdout = stdout
	cmd.Stderr = stderr

	err := cmd.Run()
	res := Result{Stdout: stdout.Bytes(), Stderr: stderr.Bytes()}
	if stdout.exceeded {
		return res, ErrOutputLimit
	}
	if ctx.Err() != nil {
		return res, ctx.Err()
	}
	var exitErr *exec.ExitError
	if errors.As(err, &exitErr) {
		res.ExitCode = exitErr.ExitCode()
		return res, &Error{Args: append([]string{spec.Path}, spec.Args...), ExitCode: res.ExitCode, Stderr: string(res.Stderr)}
	}
	return res, err
}

// limitedBuffer must not embed bytes.Buffer: its promoted ReadFrom would let
// io.Copy bypass Write and the limit.
type limitedBuffer struct {
	buf      bytes.Buffer
	max      int64
	exceeded bool
}

func (b *limitedBuffer) Write(p []byte) (int, error) {
	if b.max > 0 && int64(b.buf.Len()+len(p)) > b.max {
		b.exceeded = true
		return 0, ErrOutputLimit
	}
	return b.buf.Write(p)
}

func (b *limitedBuffer) Bytes() []byte { return b.buf.Bytes() }
