package sys

import (
	"errors"
	"golang.org/x/sys/unix"
)

// Linux is a portable test host, not a DNS mutation backend. Conservatively
// reject extended ACLs here; the macOS backend implements the ACE policy.
func checkDNSACL(fd int) error {
	for _, name := range []string{"system.posix_acl_access", "system.posix_acl_default"} {
		_, err := unix.Fgetxattr(fd, name, nil)
		if err == unix.ENODATA || err == unix.EOPNOTSUPP {
			continue
		}
		if err != nil {
			return err
		}
		return errors.New("extended DNS recovery ACL is not permitted on this test host")
	}
	return nil
}
