package process

import (
	"context"
	"errors"
	"os/exec"
	"strings"
	"testing"
	"time"
)

func sh(t *testing.T) string {
	t.Helper()
	p, err := exec.LookPath("sh")
	if err != nil {
		t.Skip("sh not found")
	}
	return p
}

func TestOutputLimit(t *testing.T) {
	_, err := Run(context.Background(), Spec{Path: sh(t), Args: []string{"-c", "yes | head -c 100000"}, MaxOutput: 1000})
	if !errors.Is(err, ErrOutputLimit) {
		t.Errorf("err = %v", err)
	}
}

func TestTimeout(t *testing.T) {
	start := time.Now()
	_, err := Run(context.Background(), Spec{Path: sh(t), Args: []string{"-c", "sleep 5"}, Timeout: 100 * time.Millisecond})
	if !errors.Is(err, context.DeadlineExceeded) || time.Since(start) > 3*time.Second {
		t.Errorf("err = %v after %v", err, time.Since(start))
	}
}

func TestEnvAllowlist(t *testing.T) {
	t.Setenv("GIT_DIR", "/elsewhere")
	t.Setenv("SSH_ASKPASS", "/bin/evil")
	res, err := Run(context.Background(), Spec{Path: sh(t), Args: []string{"-c", "env"}, Env: []string{"X_EXTRA=1"}})
	if err != nil {
		t.Fatal(err)
	}
	out := string(res.Stdout)
	if strings.Contains(out, "GIT_DIR=") || strings.Contains(out, "SSH_ASKPASS=") || !strings.Contains(out, "X_EXTRA=1") {
		t.Errorf("env:\n%s", out)
	}
}

func TestExitError(t *testing.T) {
	_, err := Run(context.Background(), Spec{Path: sh(t), Args: []string{"-c", "echo boom >&2; exit 3"}})
	var pe *Error
	if !errors.As(err, &pe) || pe.ExitCode != 3 || !strings.Contains(pe.Stderr, "boom") {
		t.Errorf("err = %#v", err)
	}
}
