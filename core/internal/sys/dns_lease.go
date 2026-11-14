package sys

import "sync"

// DNSLease is a per-session ownership handle. Its release operation must affect
// only session-owned state, never restore a snapshot over another writer.
type DNSLease struct {
	mu      sync.Mutex
	release func() error
	err     error
}

func (l *DNSLease) Close() error {
	if l == nil {
		return nil
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.release != nil {
		l.err = l.release()
		l.release = nil
	}
	return l.err
}
