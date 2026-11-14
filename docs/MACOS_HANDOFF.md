# macOS testing handoff (`main`)

The branch is a stabilization handoff, **not a completed macOS release**.
Windows work remains gated. Preserve GTK4/libadwaita, Rust/Go, profiles and Linux.
Use [STAGE0_REPORT.md](STAGE0_REPORT.md) for Linux evidence and
[MACOS_VALIDATION.md](MACOS_VALIDATION.md) for native acceptance.

## First native pass

Record the checked-out commit, macOS version, CPU architecture and signing mode.
Never attach real configurations, subscription URLs or credentials to logs.

```sh
brew install gtk4 libadwaita protobuf just dylibbundler librsvg
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets
(cd core && GOTOOLCHAIN=go1.26.0 go test ./...)
(cd core && GOTOOLCHAIN=go1.26.0 go vet ./...)
bash scripts/test-macos-packaging.sh
just build
just ipc-smoke
just installer
```

The smoke test uses synthetic loopback traffic and no user database. It does
not test TUN or helper authorization. Check shipped Go tags as well as defaults;
Darwin flags differ from Linux, so use the Justfile as the build authority.
Hosted CI intentionally does not install a privileged helper or change networking.

## Protected installation

The complete finalized bundle, including Frameworks, is installed at:

```text
/Library/Application Support/Nolurix/Nolurix.app
/Applications/Nolurix.app -> the protected bundle above
```

The Applications link is not trusted. Do not chmod shared directories or remove
their ACLs. The PKG validates its dependency/link closure; only its private
subtree is normalized. Test launch via Finder and the convenience link, actual
`NSBundle.mainBundle`, GTK resources and operation without Homebrew separately.

- `just installer`: explicitly unsigned local PKG, ad-hoc app, manual launchd
  daemon. The protected manual marker disables automatic SMAppService fallback.
- `just installer-release`: requires Developer ID application/installer identities
  and `NOTARY_PROFILE`; requests signing, notarization and stapling. Native
  success must be recorded, never inferred. This installation uses SMAppService
  from the protected bundle; approve it in System Settings and retry connection.
- DMG-only copies remain usable for ordinary local proxy development, but are
  not authorized for privileged VPN operations from `/Applications`.

## Upgrade, migration and removal

Follow [the migration guide](MIGRATION.md) before installing Nolurix over a
former Throne GTK installation. Old helper retirement is explicit and must use
the verified old installation, never the new executable or a root GUI.

## Native gates still open

Build both supported architectures where tooling exists. Exercise Security.framework
and ACL checks, wrong peers, replaced images, denied approval, retry, worker death,
and application update/removal. Validate launchd process-group cleanup on upgrade.
Test [DNS restoration and recovery fault cases](MACOS_DNS.md) against the actual
resolver, not only stored preferences. Follow the full IPv4/IPv6/DNS, network-change and sleep
matrix; no leak-free or reliable-VPN claim is supported by compilation alone.

Opt-in system proxy, richer menu-bar controls and autostart remain subsequent
macOS implementation work. No Windows service/installer implementation belongs
in this handoff. Developer ID credentials and real-network manual tests are not
provided by Linux checks.
