//go:build darwin && !cgo

package sys

import (
	"errors"
	tun "github.com/sagernet/sing-tun"
)

func AcquireSystemDNS(string, tun.DefaultInterfaceMonitor) (*DNSLease, error) {
	return nil, errors.New("macOS system DNS requires cgo and SystemConfiguration")
}
func RecoverSystemDNS() error {
	return errors.New("macOS DNS recovery requires cgo and SystemConfiguration")
}
func checkDNSACL(int) error { return errors.New("macOS DNS ACL validation requires cgo") }
