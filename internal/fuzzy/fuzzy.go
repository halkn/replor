// Package fuzzy ranks paths against a typed query (subsequence matching).
package fuzzy

import (
	"context"
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"
)

// Result is a matched item. Positions are byte offsets of the matched runes.
type Result struct {
	Index     int
	Score     int
	Positions []int
}

const (
	scoreChar        = 16
	bonusConsecutive = 12
	bonusBoundary    = 10
	bonusAfterSlash  = 12
	bonusBasename    = 30
	penaltyGap       = 1
)

// Match reports whether query is a case-insensitive subsequence of s. Among
// the possible alignments it prefers one inside the last path element, then
// the tightest one.
func Match(query, s string) (Result, bool) {
	q := []rune(strings.ToLower(query))
	if len(q) == 0 {
		return Result{}, true
	}
	full, ok := align(q, s, 0)
	if !ok {
		return Result{}, false
	}
	if base := strings.LastIndexByte(s, '/') + 1; base > 0 {
		if r, ok := align(q, s, base); ok {
			r.Score += bonusBasename + len(q)*10/max(utf8.RuneCountInString(s[base:]), 1)
			if r.Score > full.Score {
				return r, true
			}
		}
	} else {
		full.Score += bonusBasename + len(q)*10/max(utf8.RuneCountInString(s), 1)
	}
	return full, true
}

// align finds the shortest window of s[from:] containing q: the earliest
// end of a greedy forward match, then the latest start that still matches
// backwards from it.
func align(q []rune, s string, from int) (Result, bool) {
	end, qi := -1, 0
	for i, r := range s[from:] {
		if unicode.ToLower(r) == q[qi] {
			qi++
			if qi == len(q) {
				end = from + i
				break
			}
		}
	}
	if end < 0 {
		return Result{}, false
	}
	type at struct {
		off int
		r   rune
	}
	var window []at
	for i, r := range s[from : end+utf8.RuneLen(runeAt(s, end))] {
		window = append(window, at{from + i, unicode.ToLower(r)})
	}
	pos := make([]int, len(q))
	qi = len(q) - 1
	for i := len(window) - 1; i >= 0 && qi >= 0; i-- {
		if window[i].r == q[qi] {
			pos[qi] = window[i].off
			qi--
		}
	}
	return Result{Score: score(s, pos), Positions: pos}, true
}

func runeAt(s string, i int) rune {
	r, _ := utf8.DecodeRuneInString(s[i:])
	return r
}

func score(s string, pos []int) int {
	sc := 0
	for k, p := range pos {
		sc += scoreChar
		if k > 0 {
			prevEnd := pos[k-1] + utf8.RuneLen(runeAt(s, pos[k-1]))
			if p == prevEnd {
				sc += bonusConsecutive
			} else {
				sc -= penaltyGap * utf8.RuneCountInString(s[prevEnd:p])
			}
		}
		switch {
		case p == 0:
			sc += bonusAfterSlash
		default:
			prev, _ := utf8.DecodeLastRuneInString(s[:p])
			cur := runeAt(s, p)
			switch {
			case prev == '/':
				sc += bonusAfterSlash
			case strings.ContainsRune("-_. ", prev), unicode.IsLower(prev) && unicode.IsUpper(cur):
				sc += bonusBoundary
			}
		}
	}
	return sc
}

// Rank returns up to limit matches, best first; ties go to the shorter item
// and then to the earlier one. With an empty query the first limit items are
// returned in order. It returns nil if ctx is cancelled.
func Rank(ctx context.Context, query string, items []string, limit int) []Result {
	if query == "" {
		n := min(limit, len(items))
		out := make([]Result, n)
		for i := range n {
			out[i] = Result{Index: i}
		}
		return out
	}
	var out []Result
	for i, s := range items {
		if i%1024 == 0 && ctx.Err() != nil {
			return nil
		}
		if r, ok := Match(query, s); ok {
			r.Index = i
			out = append(out, r)
		}
	}
	sort.Slice(out, func(a, b int) bool {
		ra, rb := out[a], out[b]
		if ra.Score != rb.Score {
			return ra.Score > rb.Score
		}
		if la, lb := len(items[ra.Index]), len(items[rb.Index]); la != lb {
			return la < lb
		}
		return ra.Index < rb.Index
	})
	if len(out) > limit {
		out = out[:limit]
	}
	return out
}
