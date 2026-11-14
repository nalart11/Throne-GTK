# macOS validation matrix

**Status: not executed on macOS in this work.** The development host is Linux.
This is an acceptance checklist, not a claim of implemented or verified features.
See [CROSS_PLATFORM_PLAN.md](CROSS_PLATFORM_PLAN.md) for the active stage.

Record commit, macOS version, CPU architecture, installation/signing mode,
commands, exit status and sanitized observations for each run. Do not attach
profiles, subscription URLs, access tokens, private keys or full core configs.
Do privileged/network tests on a disposable Mac or controlled test account;
preserve unrelated network settings. Keep a second recovery network available.

## Automated build/package evidence

- Run `cargo fmt --all -- --check`, `cargo test --workspace` and
  `cargo clippy --workspace --all-targets`. Record warnings separately; the
  current baseline is not warning-free.
- In `core`, use the Go toolchain pinned by `Justfile` (`GOTOOLCHAIN=go1.26.0`)
  for `go test ./...` and `go vet ./...`. Run tests with the actual build tags
  as well; an untagged build does not cover all shipped protocols.
- Run `RELEASE=1 just build` and `just dmg`; inspect all plist files with
  `plutil -lint`. Build Intel and Apple Silicon artifacts separately where
  runners/dependencies exist; do not call a single-architecture build universal.
- Inspect `Contents/MacOS`, `Frameworks`, `Resources` and helper plists. Confirm
  GUI/core remain in the layout required by parent verification. Run `otool -L`
  on every Mach-O, including nested libraries; no build-host Homebrew paths
  may remain. Check `LC_RPATH` and dynamically loaded modules/resources too.
- Run `codesign --verify --deep --strict --verbose=2` on the final app.
  Ad-hoc success is not Developer ID distribution or notarization. In release
  mode additionally record Gatekeeper assessment and actual notary/stapler
  results without credentials. Verify PKG payload and scripts independently.

## Real-system matrix

| Scenario | Required observation |
| --- | --- |
| Clean machine, no Homebrew | Install protected PKG, launch via `/Applications` convenience link; GTK theme/icons/fonts, menus and core resources load without host tooling; DMG-only proxy launch checked separately |
| Upgrade | Existing SQLite profiles/settings survive app replacement; import and subscriptions still work |
| Concurrent cores | Connected session plus latency/speed checks have separate endpoints; canceling tests does not disconnect the session |
| Helper approval | GUI remains non-root; denied/missing approval gives actionable error; approval then retry works without full app restart |
| Helper authorization | Unrelated executable, wrong UID/PID, writable installation, malformed/slow request and replaced endpoint are rejected; ordinary GUI still works |
| Helper/core death | Terminate only the test worker/helper; GUI leaves Connected promptly, no frozen actions or abandoned network state |
| Local proxy | Real HTTP and SOCKS requests explicitly through loopback listener succeed; disconnect closes listener; default system traffic is unchanged when system-proxy setting is disabled |
| Opt-in system proxy | Snapshot HTTP/HTTPS/SOCKS plus bypass/PAC state per service; connect/disconnect restores only owned changes; concurrent external edits are not overwritten |
| Proxy crash recovery | Force-close GUI/core while enabled; verify recovery behavior and unrelated network settings; report any residual settings |
| TUN IPv4 | Controlled outbound request uses expected egress; core outbound stays outside capture; no routing loop |
| TUN IPv6 | With IPv6 enabled verify actual IPv6 egress; with it disabled document block/bypass behavior rather than assuming protection |
| DNS | Capture DNS on relevant physical/tunnel interfaces before/during/after connect using controlled domains; distinguish OS/application DoH and split DNS; report observed leaks rather than infer from config |
| Network change | Wi-Fi reconnect and Wi-Fi → hotspot while connected recover or fail visibly; routes/DNS match the new network after disconnect |
| Sleep/wake | Test active connection across sleep; no permanently Connected dead tunnel, stale interface or unbounded reconnect loop |
| Normal and abrupt exit | Quit, close hidden window, SIGTERM and forced GUI/core exit; no worker, listener, TUN, routes or owned proxy state remains beyond documented recovery limits |
| Menu bar | Status, server, actions and main-window reopening match connection state; works while window hidden; AppKit work stays on main thread |
| Autostart | Login item actual state matches UI; hidden launch and optional reconnect work; disable removes only this app's item; GUI never starts as root |
| Install/update/uninstall | PKG consent is explicit; update does not launch stale helper; removal unregisters owned privileged components without deleting profiles |

For leak tests record interface/route/resolver state and observed destinations,
not just a browser IP result. Compare before/after while accounting for changes
made by the OS or other apps. Do not restore a stale whole-system snapshot.

## Protocol and Linux regression evidence

Test VLESS/Reality/XHTTP, Hysteria2, TUIC, AnyTLS, Naive, SOCKS and HTTP only
against available controlled configurations. Record unavailable configurations
as **unverified**, not passed. Include routing, auto-selection, DNS, statistics,
latency, speed tests, subscriptions and SQLite persistence. Repeat shared tests
and a Linux connect/disconnect/tray smoke test after common changes.

## Current external blockers

- No macOS execution host has been established for these local changes.
- The user subsequently authorized a `dev` push for the Mac testing handoff.
  Record the actual hosted workflow result for that commit; authorization or a
  workflow definition alone is not a successful build.
- System permission prompts, real network changes, sleep/wake and clean-machine
  behavior require a Mac and explicit test setup.
- Developer ID/notarization credentials were not provided. Do not manufacture
  successful signing, notarization or leak-test results from build output.
