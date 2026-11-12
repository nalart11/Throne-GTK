# macOS DNS lease and recovery

This is implementation documentation, **not evidence of leak-free DNS**.
Native compilation, resolver behavior and fault handling must be tested on Mac.

## Scope

`core/internal/sys` uses SystemConfiguration/SCPreferences rather than invoking
`networksetup` to change DNS. TUN Start acquires a DNS lease; Stop and failed
Start release it. Ordinary local proxy mode does not acquire this lease.
The compatibility `SetSystemDNS("Empty", ...)` entry point releases only its
own lease; it no longer means "erase the user's configured DNS servers".

Only the initially selected service in the current network location is changed.
The target is a service ID plus BSD interface name and network-location ID,
not a display name. Automatic migration across Wi-Fi/hotspot/location changes
is not implemented. Existing search-domain and other DNS dictionary fields are
preserved while the server-address field is replaced for the lease.

## Transaction and ownership

A process holds the global permanent `lease.lock` inode throughout its lease.
This excludes competing workers across users, not just within one GUI. Before
mutation it opens fresh locked preferences, snapshots protocol presence/enabled
state and the complete configuration, and durably writes original and intended
states to a versioned journal. Only then does it commit and apply preferences.

The fixed recovery directory is `/private/var/db/throne-gtk-dns`. Production
ownership is root; directories/files are traversed without following symlinks,
and journal input is size-limited. Journal replacement uses an exclusive random
temporary file, file fsync, atomic rename and directory fsync. No subscription
or GUI-supplied pathname controls recovery storage.

Restore opens fresh preferences and compares complete states:

- Current equals the installed lease: restore the original snapshot, commit
  and apply, then retire the journal.
- Current already equals the original: apply it again before retiring evidence.
- Current differs, or the service/location identity changed: report a conflict,
  retain the journal and do not overwrite someone else's settings.

Ambiguous commit/apply failures retain recovery evidence even if best-effort
rollback succeeds. A conflicting journal deliberately prevents another DNS
acquisition; silently deleting it would hide uncertainty about system state.

The helper attempts recovery at startup and after each worker exits, before
releasing that user's worker reservation. Both hooks use the same nonblocking
lease lock; an active lease is skipped, never restored. Preferences-lock
contention also fails immediately rather than waiting indefinitely.

Installer upgrade/removal calls the protected core's fixed-purpose
`--recover-dns` mode after stopping the service and before replacing/removing
its executable. The command requires root and the verified protected installation,
accepts no paths, and reports busy/conflict/error as failure. Removal preserves
installed artifacts on recovery failure. It is not a general root command runner.
No user-supplied dictionary or snapshot is accepted by this mode.

## Native fault checks

Use a controlled account/network and record original settings before testing.
Check stored preferences **and** effective resolver state (`scutil --dns`) and
actual DNS packets; a successful Apply call does not establish resolver routing.

Test absent DNS protocol, present protocol without a dictionary, empty/custom
dictionaries, disabled protocol, preferences-lock contention, simultaneous
workers, external edits, and network-location/service replacement. Inject death
before and after journal publication, commit, apply and restore. Force-kill a
worker while leaving the helper alive, then test whole-group termination and
helper restart. Confirm update/removal either restores owned DNS or fails
visibly with the recovery evidence and installed executable preserved.

Do not resolve conflicts by blindly deleting `recovery.json`, restoring an old
whole-system snapshot, or assigning `Empty`. Inspect original/current/intended
states and service identity first. Journals contain network configuration and
must not be attached to public diagnostics without review.

See [MACOS_VALIDATION.md](MACOS_VALIDATION.md) for the wider TUN, IPv4/IPv6,
sleep/wake and leak-testing matrix. No such native results are established by
Linux transaction tests.
