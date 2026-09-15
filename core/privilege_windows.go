//go:build windows

package main

import "golang.org/x/sys/windows"

// Windows creates the TUN adapter only from an elevated process. The GUI must
// itself be started as administrator so the core it launches inherits the
// elevated token.
func hasTunPrivilege() bool {
	return windows.GetCurrentProcessToken().IsElevated()
}
