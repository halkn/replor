package document

import (
	"strings"
	"testing"
)

func TestSanitize(t *testing.T) {
	cases := map[string]string{
		"plain\ttext\n":           "plain\ttext\n",
		"日本語😀":                    "日本語😀",
		"a\x1b[2Jb":               "a␛[2Jb",
		"crlf\r\nline":            "crlf\nline",
		"lone\rcr":                "lone␍cr",
		"del\x7f":                 "del␡",
		"c1\u009b31m":             "c1�31m",
		"bad\xff\x1b":             "bad�␛",
		"osc\x1b]8;;http://x\x07": "osc␛]8;;http://x␇",
		"kitty\x1b_Ga=d\x1b\\":    "kitty␛_Ga=d␛\\",
	}
	for in, want := range cases {
		if got := Sanitize(in); got != want {
			t.Errorf("Sanitize(%q) = %q, want %q", in, got, want)
		}
	}
}

func FuzzSanitize(f *testing.F) {
	f.Add("a\x1b[31mb\r\n\x9b")
	f.Fuzz(func(t *testing.T, s string) {
		out := Sanitize(s)
		for _, r := range out {
			if r < 0x20 && r != '\n' && r != '\t' || r == 0x7f || r >= 0x80 && r < 0xa0 {
				t.Fatalf("control %U survived in %q", r, out)
			}
		}
		if strings.Contains(out, "\r") {
			t.Fatal("CR survived")
		}
	})
}
