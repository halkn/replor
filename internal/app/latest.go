// Package app holds the Bubble Tea application state.
package app

import (
	"context"
	"sync"
)

// Ticket identifies one asynchronous request for a display target ("preview",
// "tree:docs", ...). Results carry their ticket back in the message.
type Ticket struct {
	Gen uint64 // workspace/branch generation
	Seq uint64 // per-target sequence
	Key string
}

// Latest keeps only the newest request per target. Issuing a new request
// cancels the previous one; results from older tickets or generations are
// rejected even if their work could not be cancelled.
type Latest struct {
	mu     sync.Mutex
	gen    uint64
	seq    map[string]uint64
	cancel map[string]context.CancelFunc
}

func NewLatest() *Latest {
	return &Latest{seq: map[string]uint64{}, cancel: map[string]context.CancelFunc{}}
}

// Issue starts a request for key and returns its ticket and context.
func (l *Latest) Issue(parent context.Context, key string) (Ticket, context.Context) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if c := l.cancel[key]; c != nil {
		c()
	}
	ctx, cancel := context.WithCancel(parent)
	l.seq[key]++
	l.cancel[key] = cancel
	return Ticket{Gen: l.gen, Seq: l.seq[key], Key: key}, ctx
}

// Accept reports whether t is still the newest request of the current
// generation, and releases its context if so.
func (l *Latest) Accept(t Ticket) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	if t.Gen != l.gen || t.Seq != l.seq[t.Key] {
		return false
	}
	if c := l.cancel[t.Key]; c != nil {
		c()
		delete(l.cancel, t.Key)
	}
	return true
}

// NewGeneration invalidates every outstanding request (workspace or branch
// switch) and cancels their work.
func (l *Latest) NewGeneration() {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.gen++
	for k, c := range l.cancel {
		c()
		delete(l.cancel, k)
	}
}

// Current reports whether t is still the newest request without releasing
// it, for requests that deliver several results (streamed batches).
func (l *Latest) Current(t Ticket) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	return t.Gen == l.gen && t.Seq == l.seq[t.Key]
}

// Cancel invalidates and cancels the outstanding request for key.
func (l *Latest) Cancel(key string) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.seq[key]++
	if c := l.cancel[key]; c != nil {
		c()
		delete(l.cancel, key)
	}
}
