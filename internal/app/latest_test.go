package app

import (
	"context"
	"sync"
	"testing"
	"time"
)

// A→B selection where A finishes last: A's result must be dropped.
func TestLatestRejectsReorderedResults(t *testing.T) {
	l := NewLatest()
	ta, ctxA := l.Issue(context.Background(), "preview")
	tb, _ := l.Issue(context.Background(), "preview")

	if ctxA.Err() == nil {
		t.Error("issuing B did not cancel A")
	}
	if !l.Accept(tb) {
		t.Error("B rejected")
	}
	if l.Accept(ta) {
		t.Error("late A accepted")
	}
}

func TestLatestKeysAreIndependent(t *testing.T) {
	l := NewLatest()
	tp, _ := l.Issue(context.Background(), "preview")
	tt, _ := l.Issue(context.Background(), "tree:.")
	if !l.Accept(tt) || !l.Accept(tp) {
		t.Error("independent targets interfered")
	}
}

func TestLatestGenerationInvalidatesEverything(t *testing.T) {
	l := NewLatest()
	tk, ctx := l.Issue(context.Background(), "git")
	l.NewGeneration()
	if ctx.Err() == nil {
		t.Error("generation change did not cancel")
	}
	if l.Accept(tk) {
		t.Error("old generation accepted")
	}
	tk2, _ := l.Issue(context.Background(), "git")
	if !l.Accept(tk2) {
		t.Error("new generation rejected")
	}
}

// Rapid selection with randomized completion order: exactly the last request
// is applied.
func TestLatestUnderContention(t *testing.T) {
	l := NewLatest()
	const n = 200
	tickets := make([]Ticket, n)
	for i := range n {
		tickets[i], _ = l.Issue(context.Background(), "preview")
	}
	var mu sync.Mutex
	var applied []uint64
	var wg sync.WaitGroup
	for i := n - 1; i >= 0; i-- {
		wg.Go(func() {
			time.Sleep(time.Duration(i%7) * time.Millisecond)
			if l.Accept(tickets[i]) {
				mu.Lock()
				applied = append(applied, tickets[i].Seq)
				mu.Unlock()
			}
		})
	}
	wg.Wait()
	if len(applied) != 1 || applied[0] != tickets[n-1].Seq {
		t.Errorf("applied %v, want only %d", applied, tickets[n-1].Seq)
	}
}
