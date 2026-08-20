//go:build !windows

package boxdns

import "github.com/sagernet/sing/common/control"

// HandleSystemDNS is a no-op on non-Windows platforms: system-DNS management
// there is handled elsewhere (e.g. darwin via sys.SetSystemDNS). The always-on
// interface monitor still runs and registers this callback so DefaultInterface
// behaves consistently across platforms.
func (d *DnsManager) HandleSystemDNS(ifc *control.Interface, flag int) {}
