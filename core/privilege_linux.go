package main

import (
	"os"

	"golang.org/x/sys/unix"
)

// hasTunPrivilege reports whether this process can create a TUN device.
//
// Root always can. So can an unprivileged process whose binary carries
// CAP_NET_ADMIN as a file capability (`just grant-tun`), and that path leaves
// euid untouched — so checking euid alone would report "no privilege" for a
// setcap'd core that is in fact able to bring the interface up, leaving the UI
// with no way to ever enter VPN mode.
func hasTunPrivilege() bool {
	if os.Geteuid() == 0 {
		return true
	}

	hdr := unix.CapUserHeader{Version: unix.LINUX_CAPABILITY_VERSION_3}
	// Version 3 reports capabilities in two 32-bit words; Capget writes both.
	var data [2]unix.CapUserData
	if err := unix.Capget(&hdr, &data[0]); err != nil {
		return false
	}

	word, bit := unix.CAP_NET_ADMIN>>5, uint(unix.CAP_NET_ADMIN&31)
	return data[word].Effective&(1<<bit) != 0
}
