package preview

import (
	"strings"
	"testing"
)

const sampleDiff = "diff --git a/x.md b/x.md\n" +
	"index 1..2 100644\n" +
	"--- a/x.md\n" +
	"+++ b/x.md\n" +
	"@@ -1,2 +1,2 @@\n" +
	" keep\n" +
	"-old\x1b[2J\n" +
	"+new\tline\n" +
	"\\ No newline at end of file\n"

func TestDiffLines(t *testing.T) {
	r := DiffLines(sampleDiff, 80)
	if len(r.Lines) != 9 || len(r.BlockLine) != 9 || !r.Complete {
		t.Fatalf("lines=%d blocks=%d", len(r.Lines), len(r.BlockLine))
	}
	want := map[int]string{0: diffMeta, 4: diffHunk, 6: diffDel, 7: diffAdd}
	for i, style := range want {
		if !strings.HasPrefix(r.Lines[i], style) {
			t.Errorf("line %d %q lacks style %q", i, r.Lines[i], style)
		}
	}
	if strings.Contains(strings.Join(r.Lines, ""), "\x1b[2J") || r.Plain[6] != "-old␛[2J" {
		t.Errorf("control character not neutralized: %q", r.Plain[6])
	}
	if r.Plain[7] != "+new    line" {
		t.Errorf("tab not expanded: %q", r.Plain[7])
	}
	if got := r.Find("new"); len(got) != 2 || got[0].Line != 7 {
		t.Errorf("search in diff = %+v", got)
	}
}

func TestUntrackedDiff(t *testing.T) {
	d := string(UntrackedDiff("dir/new.md", []byte("a\nb\n")))
	for _, want := range []string{"new file", "--- /dev/null", "+++ b/dir/new.md", "@@ -0,0 +1,2 @@", "+a\n+b\n"} {
		if !strings.Contains(d, want) {
			t.Errorf("missing %q in:\n%s", want, d)
		}
	}
	if d := string(UntrackedDiff("bin", []byte("a\x00b"))); !strings.Contains(d, "Binary file") {
		t.Errorf("binary:\n%s", d)
	}
	if d := string(UntrackedDiff("empty", nil)); !strings.Contains(d, "new file") || strings.Contains(d, "@@") {
		t.Errorf("empty:\n%s", d)
	}
	if d := string(UntrackedDiff("nonl", []byte("x"))); !strings.Contains(d, "+x\n\\ No newline at end of file") {
		t.Errorf("no final newline:\n%s", d)
	}
}

// A line-ending-only change must stay visible: the removed line ends in CR.
func TestDiffShowsCarriageReturn(t *testing.T) {
	r := DiffLines("@@ -1 +1 @@\n-c\r\n+c\n", 80)
	if !strings.Contains(r.Plain[1], "␍") {
		t.Errorf("removed line %q, added line %q", r.Plain[1], r.Plain[2])
	}
}

func TestUntrackedBaseDiff(t *testing.T) {
	d := string(UntrackedBaseDiff("dir/a.md", []byte("old\nsame\n"), []byte("same\nnew")))
	for _, want := range []string{"diff --git a/dir/a.md b/dir/a.md\n", "--- a/dir/a.md\n+++ b/dir/a.md\n@@ -1,2 +1,2 @@\n-old\n-same\n+same\n+new\n\\ No newline at end of file\n"} {
		if !strings.Contains(d, want) {
			t.Errorf("missing %q in:\n%s", want, d)
		}
	}
	if d := string(UntrackedBaseDiff("a", nil, []byte("x\n"))); !strings.Contains(d, "@@ -0,0 +1 @@\n+x\n") {
		t.Errorf("empty base:\n%s", d)
	}
	if d := string(UntrackedBaseDiff("a", []byte("x"), nil)); !strings.Contains(d, "@@ -1 +0,0 @@\n-x\n\\ No newline at end of file\n") {
		t.Errorf("empty now:\n%s", d)
	}
	if d := string(UntrackedBaseDiff("a", []byte("x\n"), []byte("x\n"))); strings.Contains(d, "@@") || !strings.Contains(d, "same content") {
		t.Errorf("identical:\n%s", d)
	}
	if d := string(UntrackedBaseDiff("a", []byte("x\n"), []byte("\x00"))); !strings.Contains(d, "Binary files a/a and b/a differ") {
		t.Errorf("binary:\n%s", d)
	}
}
