//go:build darwin

package sys

import (
	tun "github.com/sagernet/sing-tun"
	"sync"
)

var compatibilityDNS struct {
	sync.Mutex
	lease *DNSLease
}

// SetSystemDNS retains the legacy entry point; Empty releases only our lease.
func SetSystemDNS(addr string, monitor tun.DefaultInterfaceMonitor) error {
	compatibilityDNS.Lock()
	defer compatibilityDNS.Unlock()
	if addr == "Empty" {
		err := compatibilityDNS.lease.Close()
		compatibilityDNS.lease = nil
		return err
	}
	if err := compatibilityDNS.lease.Close(); err != nil {
		return err
	}
	compatibilityDNS.lease = nil
	next, err := AcquireSystemDNS(addr, monitor)
	if err != nil {
		return err
	}
	compatibilityDNS.lease = next
	return nil
}
