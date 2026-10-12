package main

import (
	"bytes"
	"path/filepath"
	"strings"
	"testing"
)

func TestRunArguments(t *testing.T) {
	cases := []struct {
		args []string
		code int
		out  string
	}{
		{[]string{"-version"}, 0, "replor "},
		{[]string{"-images", "sixel", "."}, 2, "unknown -images"},
		{[]string{"-nope"}, 2, "flag provided but not defined"},
		{[]string{"a", "b"}, 2, "usage: replor"},
		{[]string{filepath.Join(t.TempDir(), "missing")}, 1, "no such file"},
	}
	for _, c := range cases {
		var stdout, stderr bytes.Buffer
		code := run(c.args, &stdout, &stderr)
		if code != c.code || !strings.Contains(stdout.String()+stderr.String(), c.out) {
			t.Errorf("%v: code %d, out %q", c.args, code, stdout.String()+stderr.String())
		}
	}
}
