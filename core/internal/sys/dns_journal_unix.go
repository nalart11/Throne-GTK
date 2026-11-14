//go:build darwin || linux

package sys

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"golang.org/x/sys/unix"
)

const dnsJournalLimit = 1024 * 1024

type fileDNSJournal struct {
	dir, lock int
	owner     uint32
}

func checkDNSFile(fd int, owner uint32, directory bool) error {
	return checkDNSInode(fd, owner, directory, false, checkDNSACL)
}

// The ACL callback is explicit so portable tests can exercise fail-closed ACL
// decisions without weakening production fd-based validation or global hooks.
func checkDNSInode(fd int, owner uint32, directory, private bool, acl func(int) error) error {
	var stat unix.Stat_t
	if err := unix.Fstat(fd, &stat); err != nil {
		return err
	}
	kind := uint32(unix.S_IFREG)
	if directory {
		kind = unix.S_IFDIR
	}
	if stat.Uid != owner || uint32(stat.Mode)&unix.S_IFMT != kind || uint32(stat.Mode)&0022 != 0 || (!directory && stat.Nlink != 1) {
		return errors.New("unsafe DNS recovery filesystem ownership, permissions or file type")
	}
	mode := uint32(stat.Mode) & 07777
	if (!directory && mode != 0600) || (directory && private && mode != 0700) {
		return errors.New("DNS recovery requires private leaf directory and files")
	}
	return acl(fd)
}

// openDNSJournal traverses every directory component without following symlinks.
// The production caller supplies /private/var/db/throne-gtk-dns and owner 0.
// Only the final directory can be created. The permanent lock inode is never
// renamed/unlinked; CLOEXEC prevents the lease from leaking to child processes.
func openDNSJournal(path string, owner uint32) (_ *fileDNSJournal, err error) {
	if !filepath.IsAbs(path) || filepath.Clean(path) != path {
		return nil, errors.New("invalid DNS journal directory")
	}
	dir, err := unix.Open("/", unix.O_RDONLY|unix.O_DIRECTORY|unix.O_CLOEXEC, 0)
	if err != nil {
		return nil, err
	}
	if err = checkDNSFile(dir, owner, true); err != nil {
		_ = unix.Close(dir)
		return nil, err
	}
	parts := strings.Split(strings.TrimPrefix(path, "/"), "/")
	for i, part := range parts {
		next, e := unix.Openat(dir, part, unix.O_RDONLY|unix.O_DIRECTORY|unix.O_NOFOLLOW|unix.O_CLOEXEC, 0)
		if e == unix.ENOENT && i == len(parts)-1 {
			if e = unix.Mkdirat(dir, part, 0700); e == nil || e == unix.EEXIST {
				// Durably record the directory itself before writing recovery state.
				e = unix.Fsync(dir)
				if e == nil {
					next, e = unix.Openat(dir, part, unix.O_RDONLY|unix.O_DIRECTORY|unix.O_NOFOLLOW|unix.O_CLOEXEC, 0)
				}
			}
		}
		_ = unix.Close(dir)
		if e != nil {
			return nil, e
		}
		dir = next
		if e = checkDNSInode(dir, owner, true, i == len(parts)-1, checkDNSACL); e != nil {
			_ = unix.Close(dir)
			return nil, e
		}
	}
	defer func() {
		if err != nil {
			_ = unix.Close(dir)
		}
	}()
	lock, err := openDNSLock(dir, owner)
	if err != nil {
		return nil, err
	}
	if err = unix.Fsync(dir); err != nil {
		_ = unix.Close(lock)
		return nil, err
	}
	return &fileDNSJournal{dir: dir, lock: lock, owner: owner}, nil
}
func openDNSLock(dir int, owner uint32) (int, error) {
	lock, err := unix.Openat(dir, "lease.lock", unix.O_RDWR|unix.O_CREAT|unix.O_NOFOLLOW|unix.O_CLOEXEC, 0600)
	if err != nil {
		return -1, err
	}
	if err = checkDNSFile(lock, owner, false); err != nil {
		_ = unix.Close(lock)
		return -1, err
	}
	if err = unix.Flock(lock, unix.LOCK_EX|unix.LOCK_NB); err != nil {
		_ = unix.Close(lock)
		if err == unix.EWOULDBLOCK || err == unix.EAGAIN {
			return -1, ErrDNSLeaseBusy
		}
		return -1, fmt.Errorf("another DNS lease owns the global lock: %w", err)
	}
	return lock, nil
}

func (j *fileDNSJournal) Load() (*dnsRecord, error) {
	fd, err := unix.Openat(j.dir, "recovery.json", unix.O_RDONLY|unix.O_NONBLOCK|unix.O_NOFOLLOW|unix.O_CLOEXEC, 0)
	if err == unix.ENOENT {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	file := os.NewFile(uintptr(fd), "DNS recovery")
	defer file.Close()
	// The production constructor fixes owner to root; never infer it from a
	// journal or the connecting user's configuration directory.
	if err = checkDNSFile(fd, j.owner, false); err != nil {
		return nil, err
	}
	data, err := io.ReadAll(io.LimitReader(file, dnsJournalLimit+1))
	if err != nil {
		return nil, err
	}
	if len(data) > dnsJournalLimit {
		return nil, errors.New("DNS recovery journal exceeds size limit")
	}
	var record dnsRecord
	if err = json.Unmarshal(data, &record); err != nil {
		return nil, err
	}
	if record.Version != 1 || record.Target.Location == "" || record.Target.Service == "" || record.Target.Interface == "" || len(record.Original) == 0 || len(record.Installed) == 0 {
		return nil, errors.New("unsupported or invalid DNS recovery journal")
	}
	return &record, nil
}

func (j *fileDNSJournal) Save(record *dnsRecord) error {
	data, err := json.Marshal(record)
	if err != nil {
		return err
	}
	if len(data) > dnsJournalLimit {
		return errors.New("DNS snapshot exceeds journal size limit")
	}
	var nonce [16]byte
	if _, err = rand.Read(nonce[:]); err != nil {
		return err
	}
	name := ".recovery-" + hex.EncodeToString(nonce[:])
	fd, err := unix.Openat(j.dir, name, unix.O_WRONLY|unix.O_CREAT|unix.O_EXCL|unix.O_NOFOLLOW|unix.O_CLOEXEC, 0600)
	if err != nil {
		return err
	}
	file := os.NewFile(uintptr(fd), "DNS journal temporary")
	defer func() { _ = file.Close(); _ = unix.Unlinkat(j.dir, name, 0) }()
	if err = checkDNSFile(fd, j.owner, false); err != nil {
		return err
	}
	if _, err = file.Write(data); err != nil {
		return err
	}
	if err = file.Sync(); err != nil {
		return err
	}
	if err = file.Close(); err != nil {
		return err
	}
	if err = unix.Renameat(j.dir, name, j.dir, "recovery.json"); err != nil {
		return err
	}
	return unix.Fsync(j.dir)
}
func (j *fileDNSJournal) Remove() error {
	if err := unix.Unlinkat(j.dir, "recovery.json", 0); err != nil && err != unix.ENOENT {
		return err
	}
	return unix.Fsync(j.dir)
}
func (j *fileDNSJournal) Close() error { return errors.Join(unix.Close(j.lock), unix.Close(j.dir)) }
