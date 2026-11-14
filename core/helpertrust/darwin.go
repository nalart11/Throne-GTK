//go:build darwin && cgo

package helpertrust

/*
#cgo LDFLAGS: -framework Security -framework CoreFoundation
#include "security.h"
*/
import "C"
import (
	"fmt"
	"unsafe"
)

func CheckInstalled() error {
	return CheckTree(Bundle, CheckNoACL)
}

func CheckNoACL(path string) error {
	p := C.CString(path)
	defer C.free(unsafe.Pointer(p))
	// Allow deny-only/read-only ACLs; reject mutation grants and unknown entries.
	if C.throne_safe_acl(p) != 0 {
		return fmt.Errorf("cannot trust ACL on %s: repair installation using the PKG", path)
	}
	return nil
}

func CheckLive(pid int) error {
	p := C.CString(GUI)
	defer C.free(unsafe.Pointer(p))
	if status := C.throne_signature(p, C.int(pid)); status != 0 {
		return fmt.Errorf("GUI live signature validation failed: %d", int(status))
	}
	return nil
}
