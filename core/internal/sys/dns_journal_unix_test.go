//go:build darwin || linux

package sys

import (
	"errors"
	"os"
	"path/filepath"
	"testing"

	"golang.org/x/sys/unix"
)

func testJournalDirectory(t *testing.T) (string, int) {
	t.Helper()
	path := t.TempDir()
	if err := os.Chmod(path, 0700); err != nil {
		t.Fatal(err)
	}
	fd, err := unix.Open(path, unix.O_RDONLY|unix.O_DIRECTORY|unix.O_CLOEXEC, 0)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = unix.Close(fd) })
	return path, fd
}

func TestDNSGlobalWriterContention(t *testing.T) {
	_, dir := testJournalDirectory(t)
	lock, err := openDNSLock(dir, uint32(os.Geteuid()))
	if err != nil {
		t.Fatal(err)
	}
	if second, err := openDNSLock(dir, uint32(os.Geteuid())); err == nil {
		_ = unix.Close(second)
		t.Fatal("second writer acquired occupied lease")
	}
	flags, err := unix.FcntlInt(uintptr(lock), unix.F_GETFD, 0)
	if err != nil || flags&unix.FD_CLOEXEC == 0 {
		t.Fatal("lock descriptor could be inherited across exec")
	}
	var before, after unix.Stat_t
	if err = unix.Fstat(lock, &before); err != nil {
		t.Fatal(err)
	}
	_ = unix.Close(lock)
	next, err := openDNSLock(dir, uint32(os.Geteuid()))
	if err != nil {
		t.Fatal(err)
	}
	defer unix.Close(next)
	if err = unix.Fstat(next, &after); err != nil {
		t.Fatal(err)
	}
	if before.Ino != after.Ino {
		t.Fatal("lock inode replaced")
	}
}

func TestDNSJournalDurabilityAndValidation(t *testing.T) {
	path, dir := testJournalDirectory(t)
	j := &fileDNSJournal{dir: dir, owner: uint32(os.Geteuid())}
	record := &dnsRecord{Version: 1, Target: testTarget, Original: []byte("original"), Installed: []byte("installed")}
	if err := j.Save(record); err != nil {
		t.Fatal(err)
	}
	got, err := j.Load()
	if err != nil || got == nil || string(got.Original) != "original" {
		t.Fatalf("load: %+v %v", got, err)
	}
	for _, data := range [][]byte{[]byte(`{"Version":2}`), make([]byte, dnsJournalLimit+1)} {
		if err = os.WriteFile(filepath.Join(path, "recovery.json"), data, 0600); err != nil {
			t.Fatal(err)
		}
		if _, err = j.Load(); err == nil {
			t.Fatal("invalid journal accepted")
		}
	}
	if err = j.Remove(); err != nil {
		t.Fatal(err)
	}
	if err = os.Symlink("not-the-journal", filepath.Join(path, "recovery.json")); err != nil {
		t.Fatal(err)
	}
	if _, err = j.Load(); err == nil {
		t.Fatal("journal symlink followed")
	}
}

func TestDNSLockRejectsSymlink(t *testing.T) {
	path, dir := testJournalDirectory(t)
	if err := os.Symlink("elsewhere", filepath.Join(path, "lease.lock")); err != nil {
		t.Fatal(err)
	}
	if lock, err := openDNSLock(dir, uint32(os.Geteuid())); err == nil {
		_ = unix.Close(lock)
		t.Fatal("lock symlink followed")
	}
}

func TestDNSACLValidationHookAndPrivateModes(t *testing.T) {
	path, dir := testJournalDirectory(t)
	denied := errors.New("unsafe or unreadable ACL")
	called := false
	acl := func(fd int) error {
		called = true
		if fd != dir {
			t.Fatal("ACL validator received wrong inode")
		}
		return denied
	}
	if err := checkDNSInode(dir, uint32(os.Geteuid()), true, true, acl); !errors.Is(err, denied) || !called {
		t.Fatal("ACL error ignored")
	}
	if err := checkDNSInode(dir, uint32(os.Geteuid()), true, true, func(int) error { return nil }); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(path, 0755); err != nil {
		t.Fatal(err)
	}
	if err := checkDNSInode(dir, uint32(os.Geteuid()), true, true, func(int) error { return nil }); err == nil {
		t.Fatal("nonprivate leaf accepted")
	}
	file, err := os.OpenFile(filepath.Join(path, "file"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	if err = checkDNSInode(int(file.Fd()), uint32(os.Geteuid()), false, false, func(int) error { return denied }); !errors.Is(err, denied) {
		t.Fatal("file ACL error ignored")
	}
	if err = file.Chmod(0644); err != nil {
		t.Fatal(err)
	}
	if err = checkDNSInode(int(file.Fd()), uint32(os.Geteuid()), false, false, func(int) error { return nil }); err == nil {
		t.Fatal("nonprivate journal accepted")
	}
}

func TestDNSRecoveryUsesSameGlobalFlock(t *testing.T) {
	path, _ := testJournalDirectory(t)
	open := func() (dnsJournal, error) {
		dir, err := unix.Open(path, unix.O_RDONLY|unix.O_DIRECTORY|unix.O_CLOEXEC, 0)
		if err != nil {
			return nil, err
		}
		lock, err := openDNSLock(dir, uint32(os.Geteuid()))
		if err != nil {
			_ = unix.Close(dir)
			return nil, err
		}
		return &fileDNSJournal{dir: dir, lock: lock, owner: uint32(os.Geteuid())}, nil
	}
	active, err := open()
	if err != nil {
		t.Fatal(err)
	}
	b, _ := newTestDNS()
	record := &dnsRecord{Version: 1, Target: testTarget, Original: []byte("original"), Installed: []byte("installed")}
	if err = active.Save(record); err != nil {
		_ = active.Close()
		t.Fatal(err)
	}
	b.state = []byte("installed")
	if err = recoverDNS(b, open); !errors.Is(err, ErrDNSLeaseBusy) {
		t.Fatal(err)
	}
	if string(b.state) != "installed" || b.writes != 0 {
		t.Fatal("recovery restored another live worker's DNS")
	}
	if _, err = os.Stat(filepath.Join(path, "recovery.json")); err != nil {
		t.Fatal("live worker's journal discarded")
	}
	// Simulates cmd.Wait observing worker death: its process-owned flock closes,
	// and the helper recovery path can now restore without a new TUN connection.
	if err = active.Close(); err != nil {
		t.Fatal(err)
	}
	if err = recoverDNS(b, open); err != nil {
		t.Fatal(err)
	}
	if string(b.state) != "original" {
		t.Fatal("dead worker not recovered")
	}
	if _, err = os.Stat(filepath.Join(path, "recovery.json")); !os.IsNotExist(err) {
		t.Fatal("successful recovery did not retire journal")
	}
}
