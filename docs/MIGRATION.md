# Nolurix migration

Nolurix is an independent project originally based on the original
[Throne](https://github.com/throneproj/Throne), not an official upstream release.
The repository is [nalart11/Nolurix](https://github.com/nalart11/Nolurix)
(`main`). GPL-3.0-or-later and upstream/third-party credits remain in force.

## User data

Before opening the normal database, automatically migrate
`throne-gtk/throne-gtk.db` to `nolurix/nolurix.db` under the platform
configuration directory:

| Platform | Configuration directory |
| --- | --- |
| Linux | `$XDG_CONFIG_HOME`, or `~/.config` |
| macOS | `~/Library/Application Support` |
| Windows | `%APPDATA%` |

Use SQLite backup, including committed WAL contents, rather than copying a
live `.db` alone. Existing Nolurix data wins; never merge or overwrite it.
Never delete the source. Use crash-safe staging/publication and retry an
interrupted migration on a later launch. Stop the old application first.

Migration snapshots the main SQLite database and an optional SQLite-format
`cache.db`. The normal sing-box `cache.db` is BoltDB, not SQLite: it is
deliberately omitted and regenerated rather than copied while potentially live.
Logs, helper credentials, runtime files and arbitrary files are not migrated.
Explicit custom data directories are not automatically migrated. Even an
existing empty Nolurix directory wins.

Original upstream Throne import (`import_throne`, or routing-only import) is
different: it is explicitly selected by the user and can add duplicate entries.
Keep its original database and URLs unchanged.

## Retire the old macOS helper before installing Nolurix

Disconnect VPN, allow DNS restoration to complete, and quit the **old** app.
Never run a GUI as root. Complete DNS recovery through the trusted original
installation's verified uninstaller before retiring it. Do not delete recovery
journals to bypass a recovery failure.

For SMAppService-managed installations, run the verified **old** protected
executable **as the logged-in owning user**, not through sudo:

```sh
"/Library/Application Support/Throne GTK/Throne GTK.app/Contents/MacOS/throne-gtk" --unregister-helper
```

Stop on failure or timeout. After successful unregistering, inspect and verify
the old protected uninstaller and invoke it with administrator authorization:

```sh
sudo "/Library/Application Support/Throne GTK/uninstall.sh" --sm-unregistered
```

For a verified manual-mode old installation, use that same verified old
protected uninstaller with `sudo`, **without** `--sm-unregistered`.
Do not substitute a new Nolurix uninstaller for old-installation removal.
Legacy layouts require an individually verified retirement path; if the old
app cannot unregister, obtain a user-context native Service Management recovery
path rather than treating `launchctl bootout` as SMAppService unregistering.

The new installer refuses old protected bases, Applications links, launchd
plists and loaded services until safely retired. It does not adopt or delete
them. Journal presence alone does not block fresh package installation:
Nolurix retains the protected `/private/var/db/throne-gtk-dns` namespace,
permanent lock and recovery format. Its trusted helper attempts retained-state
recovery at startup and before acquiring a new DNS lease; recovery failures
abort lease acquisition and preserve the journal. Existing Nolurix upgrades
also require the verified installed core's recovery before payload replacement.

The new protected application is
`/Library/Application Support/Nolurix/Nolurix.app`; `/Applications/Nolurix.app`
is only a convenience link. Its identity is `dev.nalart.Nolurix`; executables
are `nolurix` and `nolurix-core`, and release assets use `nolurix-*`.

This document specifies migration behavior and safe operational steps; it is
not evidence that native macOS installation, recovery or removal passed.
Current rebranding checks and the classification of retained legacy names are
recorded separately in [REBRANDING_VERIFICATION.md](REBRANDING_VERIFICATION.md).
