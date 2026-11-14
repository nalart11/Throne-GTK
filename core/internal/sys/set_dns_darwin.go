//go:build darwin && cgo

package sys

/*
#cgo LDFLAGS: -framework CoreFoundation -framework SystemConfiguration
#include "dns_native_darwin.h"
#include <stdlib.h>
*/
import "C"

import (
	"errors"
	"fmt"
	"net/netip"
	"os"
	"unsafe"

	tun "github.com/sagernet/sing-tun"
)

// Only the initially selected service in the current network location is
// modified. Network/location migration is deliberately not implemented. Applying
// preferences is not proof of effective resolver behavior; Mac testing required.
func AcquireSystemDNS(addr string, monitor tun.DefaultInterfaceMonitor) (*DNSLease, error) {
	if os.Geteuid() != 0 {
		return nil, errors.New("system DNS requires privileged worker")
	}
	if _, err := netip.ParseAddr(addr); err != nil {
		return nil, err
	}
	if monitor == nil || monitor.DefaultInterface() == nil {
		return nil, errors.New("no default interface for DNS lease")
	}
	journal, err := openDNSJournal("/private/var/db/throne-gtk-dns", 0)
	if err != nil {
		return nil, err
	}
	return acquireDNS(nativeDNSBackend{}, journal, dnsTarget{Interface: monitor.DefaultInterface().Name}, addr)
}

type nativeDNSBackend struct{}

func checkDNSACL(fd int) error {
	if C.nolurix_dns_safe_acl(C.int(fd)) != 0 {
		return errors.New("unsafe or unreadable DNS recovery ACL")
	}
	return nil
}

// RecoverSystemDNS is safe to call while workers are alive: the same global,
// nonblocking flock used by acquisition returns ErrDNSLeaseBusy without
// inspecting or mutating an active lease. Administrative callers must fail
// rather than treat a still-live lease as successful recovery.
func RecoverSystemDNS() error {
	if os.Getuid() != 0 || os.Geteuid() != 0 {
		return errors.New("DNS recovery requires root helper")
	}
	return recoverDNS(nativeDNSBackend{}, func() (dnsJournal, error) { return openDNSJournal("/private/var/db/throne-gtk-dns", 0) })
}

type nativeDNSTransaction struct {
	native *C.nolurix_dns_tx
	target dnsTarget
}

func (nativeDNSBackend) Begin(target dnsTarget) (dnsTransaction, error) {
	iface, service, location := C.CString(target.Interface), C.CString(target.Service), C.CString(target.Location)
	defer C.free(unsafe.Pointer(iface))
	defer C.free(unsafe.Pointer(service))
	defer C.free(unsafe.Pointer(location))
	var status C.int
	native := C.nolurix_dns_begin(iface, service, location, &status)
	if native == nil {
		if status == C.NOLURIX_DNS_PREFS_BUSY {
			return nil, ErrDNSPreferencesBusy
		}
		return nil, fmt.Errorf("open nonblocking DNS preferences (busy or service/location changed): status %d", int(status))
	}
	return &nativeDNSTransaction{native: native, target: dnsTarget{Interface: target.Interface, Service: C.GoString(C.nolurix_dns_service(native)), Location: C.GoString(C.nolurix_dns_location(native))}}, nil
}
func (t *nativeDNSTransaction) Target() dnsTarget { return t.target }
func nativeDNSBytes(data *C.uchar, size C.int, status C.int) ([]byte, error) {
	if data == nil {
		return nil, fmt.Errorf("serialize DNS snapshot: status %d", int(status))
	}
	defer C.free(unsafe.Pointer(data))
	return C.GoBytes(unsafe.Pointer(data), size), nil
}
func (t *nativeDNSTransaction) Read() ([]byte, error) {
	var size, status C.int
	data := C.nolurix_dns_snapshot(t.native, nil, &size, &status)
	return nativeDNSBytes(data, size, status)
}
func (t *nativeDNSTransaction) Prepare(addr string) ([]byte, error) {
	address := C.CString(addr)
	defer C.free(unsafe.Pointer(address))
	var size, status C.int
	data := C.nolurix_dns_snapshot(t.native, address, &size, &status)
	return nativeDNSBytes(data, size, status)
}
func (t *nativeDNSTransaction) Equal(a, b []byte) bool {
	if len(a) == 0 || len(b) == 0 {
		return false
	}
	return C.nolurix_dns_equal((*C.uchar)(unsafe.Pointer(&a[0])), C.int(len(a)), (*C.uchar)(unsafe.Pointer(&b[0])), C.int(len(b))) != 0
}
func (t *nativeDNSTransaction) Set(data []byte) error {
	if len(data) == 0 {
		return errors.New("empty DNS snapshot")
	}
	var status C.int
	if C.nolurix_dns_set(t.native, (*C.uchar)(unsafe.Pointer(&data[0])), C.int(len(data)), &status) == 0 {
		return fmt.Errorf("stage DNS preferences: status %d", int(status))
	}
	return nil
}
func (t *nativeDNSTransaction) Commit() error {
	var status C.int
	if C.nolurix_dns_commit(t.native, &status) == 0 {
		return fmt.Errorf("commit DNS preferences: status %d", int(status))
	}
	return nil
}
func (t *nativeDNSTransaction) Apply() error {
	var status C.int
	if C.nolurix_dns_apply(t.native, &status) == 0 {
		return fmt.Errorf("apply DNS preferences: status %d", int(status))
	}
	return nil
}
func (t *nativeDNSTransaction) Close() { C.nolurix_dns_end(t.native) }
