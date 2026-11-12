package sys

import (
	"bytes"
	"errors"
	"sync"
	"testing"
)

var testTarget = dnsTarget{Location: "location-a", Service: "service-a", Interface: "en0"}

type testDNSBackend struct {
	target                   dnsTarget
	state, pending           []byte
	fault                    string
	writes, commits, applies int
	locked                   bool
	beginErr                 error
}

func (b *testDNSBackend) Begin(target dnsTarget) (dnsTransaction, error) {
	if b.beginErr != nil {
		return nil, b.beginErr
	}
	if b.locked {
		return nil, errors.New("preferences lock already held")
	}
	if target.Location != "" && target != b.target {
		return nil, errors.New("service or location changed")
	}
	b.locked = true
	return &testDNSTx{b}, nil
}

type testDNSTx struct{ b *testDNSBackend }

func (t *testDNSTx) Target() dnsTarget     { return t.b.target }
func (t *testDNSTx) Read() ([]byte, error) { return bytes.Clone(t.b.state), nil }
func (t *testDNSTx) Prepare(string) ([]byte, error) {
	return []byte("installed-entire-dictionary-and-enabled"), nil
}
func (*testDNSTx) Equal(a, b []byte) bool  { return bytes.Equal(a, b) }
func (t *testDNSTx) Set(data []byte) error { t.b.writes++; t.b.pending = bytes.Clone(data); return nil }
func (t *testDNSTx) Commit() error {
	t.b.commits++
	fault := t.b.fault
	if fault == "commit-before" {
		t.b.fault = ""
		return errors.New(fault)
	}
	t.b.state = bytes.Clone(t.b.pending)
	if fault == "commit-after" {
		t.b.fault = ""
		return errors.New(fault)
	}
	return nil
}
func (t *testDNSTx) Apply() error {
	t.b.applies++
	if t.b.fault == "apply" {
		t.b.fault = ""
		return errors.New("apply failed")
	}
	return nil
}
func (t *testDNSTx) Close() { t.b.locked = false; t.b.pending = nil }

type testDNSJournal struct {
	record            *dnsRecord
	failSave, closed  bool
	saves             int
	loadErr, closeErr error
}

func (j *testDNSJournal) Load() (*dnsRecord, error) { return j.record, j.loadErr }
func (j *testDNSJournal) Save(r *dnsRecord) error {
	j.saves++
	if j.failSave {
		return errors.New("fsync failed")
	}
	j.record = r
	return nil
}
func (j *testDNSJournal) Remove() error { j.record = nil; return nil }
func (j *testDNSJournal) Close() error  { j.closed = true; return j.closeErr }
func newTestDNS() (*testDNSBackend, *testDNSJournal) {
	return &testDNSBackend{target: testTarget, state: []byte("original-entire-dictionary-and-disabled")}, &testDNSJournal{}
}

func TestDNSLeaseRoundTripAndConcurrentClose(t *testing.T) {
	b, j := newTestDNS()
	original := bytes.Clone(b.state)
	lease, err := acquireDNS(b, j, dnsTarget{Interface: "en0"}, "192.0.2.1")
	if err != nil {
		t.Fatal(err)
	}
	if j.record == nil || j.closed || b.locked {
		t.Fatal("lease/journal lock lifetime incorrect")
	}
	var wg sync.WaitGroup
	for range 20 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if err := lease.Close(); err != nil {
				t.Error(err)
			}
		}()
	}
	wg.Wait()
	if !bytes.Equal(b.state, original) || j.record != nil || !j.closed || b.commits != 2 {
		t.Fatalf("bad restore: %+v %+v", b, j)
	}
}

func TestDNSFailureBoundaries(t *testing.T) {
	for _, fault := range []string{"journal", "commit-before", "commit-after", "apply"} {
		t.Run(fault, func(t *testing.T) {
			b, j := newTestDNS()
			original := bytes.Clone(b.state)
			if fault == "journal" {
				j.failSave = true
			} else {
				b.fault = fault
			}
			if lease, err := acquireDNS(b, j, dnsTarget{Interface: "en0"}, "192.0.2.1"); err == nil || lease != nil {
				t.Fatal("failure ignored")
			}
			if !bytes.Equal(b.state, original) || !j.closed || b.locked {
				t.Fatal("failed start did not rollback and release locks")
			}
			if fault == "journal" {
				if b.writes != 0 {
					t.Fatal("mutation before durable journal")
				}
				return
			}
			if j.record == nil {
				t.Fatal("ambiguous failure lost recovery evidence")
			}
			// A later session obtains the global lock and retires already-original
			// evidence only after a successful native Apply.
			j.closed = false
			lease, err := acquireDNS(b, j, dnsTarget{Interface: "en0"}, "192.0.2.1")
			if err != nil {
				t.Fatal(err)
			}
			if err = lease.Close(); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestDNSRecoveryAfterCrash(t *testing.T) {
	for _, state := range []string{"original-entire-dictionary-and-disabled", "installed-entire-dictionary-and-enabled"} {
		b, j := newTestDNS()
		j.record = &dnsRecord{Version: 1, Target: testTarget, Original: bytes.Clone(b.state), Installed: []byte("installed-entire-dictionary-and-enabled")}
		b.state = []byte(state)
		if err := restoreDNS(b, j, j.record, true); err != nil {
			t.Fatal(err)
		}
		if j.record != nil || string(b.state) != "original-entire-dictionary-and-disabled" {
			t.Fatal("stale lease not recovered")
		}
	}
}

func TestDNSRestoreConflicts(t *testing.T) {
	for _, change := range []string{"external-edit", "location", "service", "interface"} {
		t.Run(change, func(t *testing.T) {
			b, j := newTestDNS()
			lease, err := acquireDNS(b, j, dnsTarget{Interface: "en0"}, "192.0.2.1")
			if err != nil {
				t.Fatal(err)
			}
			switch change {
			case "external-edit":
				b.state = []byte("other-app-config")
			case "location":
				b.target.Location = "location-b"
			case "service":
				b.target.Service = "service-b"
			case "interface":
				b.target.Interface = "en1"
			}
			state := bytes.Clone(b.state)
			writes := b.writes
			if err = lease.Close(); err == nil {
				t.Fatal("conflict silently ignored")
			}
			if !bytes.Equal(state, b.state) || b.writes != writes || j.record == nil || !j.closed {
				t.Fatal("conflict overwrote state or lost evidence")
			}
			if _, err = acquireDNS(b, j, dnsTarget{Interface: "en0"}, "192.0.2.2"); err == nil {
				t.Fatal("conflicted journal must block next session")
			}
		})
	}
}

func TestDNSLeaseNilClose(t *testing.T) {
	var lease *DNSLease
	if err := lease.Close(); err != nil {
		t.Fatal(err)
	}
}

func TestDNSRestoreFailureRetainsEvidence(t *testing.T) {
	for _, fault := range []string{"commit-before", "commit-after", "apply"} {
		t.Run(fault, func(t *testing.T) {
			b, j := newTestDNS()
			lease, err := acquireDNS(b, j, dnsTarget{Interface: "en0"}, "192.0.2.1")
			if err != nil {
				t.Fatal(err)
			}
			b.fault = fault
			if err = lease.Close(); err == nil {
				t.Fatal("restore failure ignored")
			}
			if j.record == nil || !j.closed || b.locked {
				t.Fatal("restore failure discarded evidence or retained locks")
			}
			if err = restoreDNS(b, j, j.record, true); err != nil {
				t.Fatal(err)
			}
			if j.record != nil || string(b.state) != "original-entire-dictionary-and-disabled" {
				t.Fatal("recovery after ambiguous restore failed")
			}
		})
	}
}

func TestDNSRecoveryErrorsAndPreferencesBusy(t *testing.T) {
	for _, failure := range []string{"open", "load", "preferences-busy", "apply", "close"} {
		t.Run(failure, func(t *testing.T) {
			b, j := newTestDNS()
			j.record = &dnsRecord{Version: 1, Target: testTarget, Original: bytes.Clone(b.state), Installed: []byte("installed-entire-dictionary-and-enabled")}
			b.state = bytes.Clone(j.record.Installed)
			injected := errors.New("injected recovery failure")
			switch failure {
			case "load":
				j.loadErr = injected
			case "preferences-busy":
				b.beginErr = ErrDNSPreferencesBusy
			case "apply":
				b.fault = "apply"
			case "close":
				j.closeErr = injected
			}
			err := recoverDNS(b, func() (dnsJournal, error) {
				if failure == "open" {
					return nil, injected
				}
				return j, nil
			})
			if err == nil {
				t.Fatal("recovery API failure ignored")
			}
			if failure == "preferences-busy" && (!errors.Is(err, ErrDNSPreferencesBusy) || b.writes != 0) {
				t.Fatal("preferences busy not propagated without mutation")
			}
			if failure != "close" && j.record == nil {
				t.Fatal("failed recovery discarded evidence")
			}
			if failure != "open" && !j.closed {
				t.Fatal("failed recovery retained global lock")
			}
		})
	}
}

func TestDNSRecoverySkipsLiveLease(t *testing.T) {
	b, _ := newTestDNS()
	err := recoverDNS(b, func() (dnsJournal, error) { return nil, ErrDNSLeaseBusy })
	if !errors.Is(err, ErrDNSLeaseBusy) || b.writes != 0 || b.locked {
		t.Fatal("recovery touched live lease")
	}
}

func TestDNSAdministrativeRecoveryWithoutJournal(t *testing.T) {
	b, j := newTestDNS()
	b.beginErr = errors.New("preferences must not be opened without a journal")
	if err := recoverDNS(b, func() (dnsJournal, error) { return j, nil }); err != nil {
		t.Fatal(err)
	}
	if !j.closed || b.writes != 0 {
		t.Fatal("empty recovery mutated preferences or leaked the lock")
	}
}
