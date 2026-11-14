package sys

import (
	"errors"
	"fmt"
)

type dnsTarget struct{ Location, Service, Interface string }
type dnsRecord struct {
	Version             int
	Target              dnsTarget
	Original, Installed []byte
}
type dnsTransaction interface {
	Target() dnsTarget
	Read() ([]byte, error)
	Prepare(string) ([]byte, error)
	Equal([]byte, []byte) bool
	Set([]byte) error
	Commit() error
	Apply() error
	Close()
}
type dnsBackend interface {
	Begin(dnsTarget) (dnsTransaction, error)
}
type dnsJournal interface {
	Load() (*dnsRecord, error)
	Save(*dnsRecord) error
	Remove() error
	Close() error
}

var ErrDNSLeaseBusy = errors.New("DNS lease is owned by a live worker")
var ErrDNSPreferencesBusy = errors.New("DNS preferences are busy; retry when the other preferences writer finishes")

// Open must acquire the global nonblocking lease lock before returning a
// journal. In particular, a busy result must not inspect or restore its state.
func recoverDNS(backend dnsBackend, open func() (dnsJournal, error)) (err error) {
	journal, err := open()
	if errors.Is(err, ErrDNSLeaseBusy) {
		return ErrDNSLeaseBusy
	}
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, journal.Close()) }()
	record, err := journal.Load()
	if err != nil || record == nil {
		return err
	}
	return restoreDNS(backend, journal, record, true)
}

// Caller must hold the global lease lock before opening a preferences session.
// Recovery never guesses ownership: a third value is evidence of an external
// writer and must leave both that value and the journal untouched.
func restoreDNS(backend dnsBackend, journal dnsJournal, record *dnsRecord, retire bool) error {
	tx, err := backend.Begin(record.Target)
	if err != nil {
		return err
	}
	defer tx.Close()
	current, err := tx.Read()
	if err != nil {
		return err
	}
	switch {
	case tx.Equal(current, record.Original):
		// A prior commit may have succeeded without Apply. Apply again before
		// discarding evidence, including the already-original recovery path.
		if err = tx.Apply(); err != nil {
			return err
		}
	case tx.Equal(current, record.Installed):
		if err = tx.Set(record.Original); err != nil {
			return err
		}
		if err = tx.Commit(); err != nil {
			return err
		}
		if err = tx.Apply(); err != nil {
			return err
		}
	default:
		return errors.New("DNS ownership conflict: configuration changed externally; recovery journal retained")
	}
	if retire {
		return journal.Remove()
	}
	return nil
}

func acquireDNS(backend dnsBackend, journal dnsJournal, target dnsTarget, address string) (lease *DNSLease, err error) {
	// journal already owns the process-wide flock. On failure retain its durable
	// evidence but release the lock so a later worker can safely attempt recovery.
	defer func() {
		if err != nil {
			_ = journal.Close()
		}
	}()
	stale, err := journal.Load()
	if err != nil {
		return nil, err
	}
	if stale != nil {
		if err = restoreDNS(backend, journal, stale, true); err != nil {
			return nil, fmt.Errorf("recover previous DNS lease: %w", err)
		}
	}
	tx, err := backend.Begin(target)
	if err != nil {
		return nil, err
	}
	closed := false
	defer func() {
		if !closed {
			tx.Close()
		}
	}()
	original, err := tx.Read()
	if err != nil {
		return nil, err
	}
	installed, err := tx.Prepare(address)
	if err != nil {
		return nil, err
	}
	record := &dnsRecord{Version: 1, Target: tx.Target(), Original: original, Installed: installed}
	if err = journal.Save(record); err != nil {
		return nil, err
	}
	defer func() {
		if err != nil {
			tx.Close()
			closed = true
			// Best-effort rollback under a fresh preferences lock. Keep the original
			// journal even if rollback succeeds: the initiating failure was ambiguous.
			if rollbackErr := restoreDNS(backend, journal, record, false); rollbackErr != nil {
				err = errors.Join(err, fmt.Errorf("DNS rollback: %w", rollbackErr))
			}
		}
	}()
	// Any failure from here is ambiguous. Leave the journal even when the native
	// API reports failure: commit/apply may have reached configd before it failed.
	if err = tx.Set(installed); err != nil {
		return nil, err
	}
	if err = tx.Commit(); err != nil {
		return nil, err
	}
	if err = tx.Apply(); err != nil {
		return nil, err
	}
	return &DNSLease{release: func() error {
		recoveryErr := restoreDNS(backend, journal, record, true)
		closeErr := journal.Close()
		return errors.Join(recoveryErr, closeErr)
	}}, nil
}
