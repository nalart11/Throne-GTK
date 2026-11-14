# Native macOS implementation

## Integration boundary

The user explicitly confirmed that the GTK localization writer finished and
handed over the listed integration files. Final localization diffs have been
reread; its Language contract and translations remain authoritative. Existing
unrelated unstaged/untracked files are not owned by this work. No commits or
repository-wide formatting. GUI slot is authorized only with temporary data,
without connecting, live tests, VPN, system proxy changes or helper operations.

SwiftUI/AppKit owns presentation; existing Rust configuration, SQLite and IPC
libraries remain authoritative. The intended bridge is an in-process Rust
static library with an explicitly owned C ABI and asynchronous JSON messages.
No second daemon, database, language preference or network control protocol.

The native executable must be named `throne-gtk`, next to `throne-gtk-core`.
The existing core parent/process authentication must remain unchanged. The
ordinary GUI stays unprivileged; VPN uses only the existing privileged helper.

## Current pass

Goal: real temporary/existing Store data through the Rust bridge into a native
window, followed by the existing engine commands and event-driven state.

Discovery and architecture review completed. Interface: `BRIDGE.md`.
Bridge/build harness is implemented and its standalone Swift C ABI smoke passed.
Frontend artifact review found and corrected transport/schema and lifecycle
defects; actual frontend transport and routing serialization smokes now pass.
Settings use local drafts and changed-field patches; routing has native ordered
rules, multiple conditions and actions. Full native build and isolated .app/DMG
checks passed; exact commands/statuses are in `VERIFICATION.md`.
Architecture gate passed after a late-close acknowledgement fix and deterministic
timeout/success/retry test. This is an isolated integration gate, not full parity.
Current pass: register the macOS-only bridge in the workspace, fix shared engine
lifecycle/IPC without changing GTK translations, connect Darwin commands and
native packaging, and inspect the existing compiled GUI on temporary data.
The bridge workspace registration changes no dependency versions; Linux/Windows
compile the bridge as an empty crate with no macOS dependencies.
Full acceptance remains pending integrated revalidation and GUI evidence.

The initial bridge source-includes the existing GTK-free engine and diagnostics
modules rather than copying them. This is a transitional build arrangement,
not a competing engine implementation. Any durable extraction needs handoff.

## Existing scenario inventory

| Scenario | Existing implementation | Native presentation |
| --- | --- | --- |
| Groups, search, server selection | `crates/app/src/ui/servers.rs` | Sidebar and server list |
| Add links, subscriptions, manual refresh | `crates/app/src/ui/dialogs.rs` | Native import sheet and group actions |
| Import Throne profiles/groups/rules | `dialogs.rs`, `throne_store::Store` | File chooser, result/error |
| Normal/automatic connection, pin/recheck | `state.rs`, `engine.rs`, `ui/window.rs` | Toolbar and server actions |
| Proxy/VPN, TUN, mux, tests, subscriptions | `ui/prefs.rs`, existing Settings | Native Settings forms |
| DNS and ordered/structured/raw routing | `ui/routes.rs`, `ui/prefs.rs` | Routing editor and Settings |
| Latency, speed, configuration validation | `engine.rs`, `ui/window.rs` | Server actions and progress |
| Traffic, active connections, logs | `ui/{window,connections,logs}.rs` | Live detail panes |
| Tray hide/reopen/quit, appearance | `tray_macos.rs`, `main.rs` | AppKit status item and lifecycle |
| RU/EN choice | Shared `Settings.language` | Same Store field, native translations |

Existing subscription auto-update fields have no scheduler consumer; do not
claim that exposing those fields implements automatic subscription refresh.
The existing `connect_last_on_start` is consumed at startup, but its GTK settings
control is absent. Initialization/tests must not automatically start networking.

## Authorized shared-file integration

1. `crates/app/src/engine.rs`: joinable termination, cancellation/ownership of
   test/subscription workers and correct StopTest delivery. Preserve localization.
2. `crates/ipc/src/lib.rs`: bounded IPC completion and per-core endpoint uniqueness;
   preserve process authentication and the existing privileged helper path.
3. `Justfile`: on Darwin invoke the native harness; leave Linux/Windows GTK commands
   intact. Shared `build/throne-gtk` output only after the other writer releases it.
4. `scripts/package-macos.sh`: use the native resources/binary, verify no GTK/adwaita
   linkage, remove GTK-only packaging tools; preserve adjacent core, helper plist,
   signing and notarization. Installer behavior must not run during validation.
5. Root Cargo manifest/lock: the macOS-only bridge is registered; its old nested
   workspace/lockfile has been removed. No dependency versions were changed.

Handoff and final reread are complete. Configuration/Store/UI localization files
remain read-only; only the explicitly handed-over shared files may be modified.
Imported legacy subscription quota text can still be Russian for English users
(existing Store formatter); do not modify that non-handed-over Store file here.

## Evidence and safety

Use a separate Cargo target directory and native build output. Publishing
`build/throne-gtk` is now authorized through the integrated Darwin build command;
do not launch a second GUI concurrently with any GUI verification.
All automated persistence tests use temporary data. Do not change system proxy,
enable VPN, register helpers, ask for root or touch the user's database.

Validation must record exact commands, exit statuses and relevant output.
Separate compilation, temporary-data integration tests, GUI interaction,
real network operation and VPN operation: none implies another.
Final integration requires rereading the completed localization changes and
explicit permission to modify each shared-file scope. No silent handoff.

## Outstanding risks

1. Engine supervision changes were interrupted and are being finished; no old
   isolated test result establishes correctness of this newer implementation.
2. IPC lifecycle/unique-endpoint implementation passed six synthetic tests and
   is ready for engine integration; its source still needs artifact review.
3. GUI menu/window inspection was limited. Screen capture permission is absent
   (CoreGraphics preflight false with one active display). SwiftUI controls,
   keyboard shortcuts, VoiceOver and both appearances remain unverified.
4. No Linux/Windows Rust target is installed (Darwin and wasm were observed).
   cfg inspection cannot establish successful Linux/Windows compilation.
5. Native deployment target is macOS 13.0 (NavigationSplitView); the existing GTK
   plist advertises 12.0. Agree the native minimum during packaging integration.
   `xcrun vtool -show-build build/throne-gtk-core` (exit 0) reports minos 27.0 for
   the existing core. Isolated packaging must advertise the maximum actual binary
   minimum, not falsely promise macOS 13 support with that prebuilt core.
6. Saved network settings currently apply on the next connection. Live reconnect
    parity, especially explicit VPN authorization, belongs to integration checks.
7. A real empty-search bug hid all profiles; the shared predicate has now been
   fixed and regression-tested. A new GUI build must include that fix.
