//go:build !linux && !windows

package main

import "os"

// hasTunPrivilege reports whether this process can create a TUN device. Only
// Linux grants that through file capabilities; elsewhere it takes root.
func hasTunPrivilege() bool {
	return os.Geteuid() == 0
}
