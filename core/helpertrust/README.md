# macOS privileged helper trust

## Migration from Throne GTK

The installer refuses any old canonical directory, Applications entry (including
dangling symlinks), old helper/manual launchd plist, or loaded old service before
mutation or installed root execution. As the owning logged-in user, use the
signature-verified original app's `--unregister-helper`, then its verified original
protected uninstaller (`--sm-unregistered` when required). A missing loaded job
does not prove SMAppService was unregistered. Never delete or bootout the old
installation as a substitute.

The protected DNS journal deliberately remains at
`/private/var/db/throne-gtk-dns`, with the original permanent lock and recovery
format. Nolurix uses the same fd-based ownership/ACL/symlink checks and recovery
path, so pre-existing restoration state is not abandoned or copied unsafely.

Authorization is mandatory in release and debug builds. The PKG installs the
complete bundle at `/Library/Application Support/Nolurix/Nolurix.app`.
Every replacement-sensitive ancestor and bundle object must be root-owned and
not group/world writable. `/Applications/Nolurix.app` is a convenience link,
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
`/Library/Application Support/Nolurix/run/helper.lock` (0600) may recover
`/Library/Application Support/Nolurix/run/helper.sock`. The PKG creates the
real root:wheel 0755, ACL-free `run` directory; missing or unsafe runtime
directories require PKG repair, never automatic repair or a legacy fallback.
Recovery requires
connection refusal and an unchanged inode; live listeners, foreign files and
ambiguous failures are never removed. The permanent lock file is never unlinked
during startup or upgrade; uninstall removes it only after stop and DNS recovery.

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
