package document

import (
	"strings"
	"unicode/utf8"
)

// Sanitize makes untrusted text safe to hand to a renderer that writes to the
// terminal. C0/C1 controls and DEL become visible symbols; only LF and TAB are
// kept. CRLF is normalized to LF. Invalid UTF-8 becomes U+FFFD so that a
// partial escape introducer cannot be assembled across bytes.
func Sanitize(s string) string {
	if isClean(s) {
		return s
	}
	var b strings.Builder
	b.Grow(len(s) + 16)
	for i := 0; i < len(s); {
		r, size := utf8.DecodeRuneInString(s[i:])
		switch {
		case r == utf8.RuneError && size == 1:
			b.WriteRune(utf8.RuneError)
		case r == '\r' && i+1 < len(s) && s[i+1] == '\n':
			// dropped; the following LF is kept
		case r == '\n' || r == '\t':
			b.WriteRune(r)
		case r < 0x20:
			b.WriteRune(0x2400 + r) // U+2400 CONTROL PICTURES block
		case r == 0x7f:
			b.WriteRune('␡')
		case r >= 0x80 && r < 0xa0:
			b.WriteRune(utf8.RuneError)
		default:
			b.WriteString(s[i : i+size])
		}
		i += size
	}
	return b.String()
}

func isClean(s string) bool {
	if !utf8.ValidString(s) {
		return false
	}
	for _, r := range s {
		if r < 0x20 && r != '\n' && r != '\t' || r >= 0x7f && r < 0xa0 {
			return false
		}
	}
	return true
}
