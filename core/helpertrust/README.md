# macOS privileged helper trust

Authorization is mandatory in release and debug builds. The PKG installs the
complete bundle at `/Library/Application Support/Throne GTK/Throne GTK.app`.
Every replacement-sensitive ancestor and bundle object must be root-owned and
not group/world writable. `/Applications/Throne GTK.app` is a convenience link,
never a trust anchor. Shared-directory permissions are not changed.

Framework/dylib links are allowed only within the protected `Contents/Frameworks`
closure; escaping, dangling and cyclic links are rejected. Deny-only and
read-only ACL entries are allowed; mutation grants and unreadable/unknown ACL
metadata are rejected. The installation itself has ACLs removed only within
its private subtree. This avoids rejecting normal deny-delete system ACLs.

The GUI must have a valid signature; Security.framework checks the live peer
against the protected installed GUI. Ad-hoc code is pinned to its installed
CodeDirectory hash; signed release code must be hardened and satisfy the
installed designated requirement. The GUI also authenticates the live root
helper against the protected core. Local ad-hoc development is not equivalent
to hardened signed distribution or protection against a compromised GUI.
This is additional defense, not a claim that PID/path checks perfectly identify
an immutable process.
The kernel-provided socket PID/UID, nonroot UID, protected tree, and repeated
checks before spawn and after worker connection are all mandatory.

The GUI endpoint must live in a private user-owned directory without extended
ACLs; only the macOS `/var` prefix alias is accepted. The public helper socket
is discoverable (0666), but every request requires the above authentication.
Only the holder of a verified root-owned singleton lock in protected
`/private/var/run` may recover an existing root-owned socket. Recovery requires
connection refusal and an unchanged inode; live listeners, foreign files and
ambiguous failures are never removed. The permanent lock file is never unlinked.

SMAppService still registers the daemon from the protected main bundle in
release mode. Development PKGs explicitly use manual launchd registration of
the same protected executable. A protected marker prevents automatic fallback
from a broken manual installation to SMAppService; authentication failure never
triggers registration. DMG-only copies support ordinary proxy development, not
privileged execution from an administrator-writable bundle.

Before SM-managed replacement/removal run the installed GUI executable with
`--unregister-helper` as its logged-in user, **without sudo**. The operation is
bounded; a timeout is not success. Installer preflight rejects ambiguous legacy
registrations and foreign files. See [macOS handoff](../../docs/MACOS_HANDOFF.md)
for the install/update/remove procedure and remaining native validation gates.

ACL return conventions follow Apple's `acl_get_entry(3)` manual: zero means an
entry was obtained; -1 with EINVAL denotes an exhausted/empty ACL. These differ
from Linux conventions and still require real-Mac validation.

Darwin ACL and Security.framework code requires real-Mac compilation and runtime
security testing; Linux policy tests do not validate those APIs. The helper's
success payload remains inside the existing status/length frame and is JSON
`{"version":1,"ok":true,"pid":123}`. Errors retain the old framed error string.
