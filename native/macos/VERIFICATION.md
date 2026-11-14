> Historical pre-rebrand record. Commands, paths, outputs and pass/fail evidence
> below describe the former Throne GTK build, not validation of Nolurix. They
> are intentionally preserved. See [migration guidance](../../docs/MIGRATION.md).

# Verification scope

This file is the acceptance map, not a claim that its checks have passed.
Executed commands and exit statuses belong in the evidence section below.

## Ownership and minimum evidence

| Claim | Owner | Required evidence |
| --- | --- | --- |
| Reuses existing Store and engine without GTK | Bridge lane | Manifest, source inclusion, isolated dependency tree and build |
| Settings/language/profile compatibility | Bridge lane | Temporary-data reopen tests, old settings default Ru, En persistence, unchanged unrelated settings |
| ABI ownership/errors/closed handles | Bridge lane | C-call tests, invalid JSON/handle paths, string release and close/reopen |
| Backend events reach UI state | Bridge lane + frontend lane | Event reducer tests and Swift snapshot decoder using real bridge |
| No network/root action during initialization | Integrator | Open path review and disconnected smoke using explicit temporary data |
| Native window and full actions | Frontend lane | Imported C ABI compile, actual action wiring, later coordinated GUI interaction |
| Shutdown, cancellation and restart | Integrator | Shared engine fix first; tests for idle, failed startup, active/cancelled operations |
| Native .app/DMG without GTK | Build lane + integrator | Bundle executable adjacency, plist/signature validation, otool linkage, DMG inspection |
| GTK localization preserved | Integrator after handoff | Reread accepted localization, settings tests and actual GTK build |
| Linux/Windows remain buildable | Integrator | Actual target builds where toolchains exist; otherwise explicit unverified limitation |

## Manual GUI acceptance after coordination

Use an explicit disposable data directory. Obtain confirmation that the other
agent has stopped its GUI check before launching this GUI. Do not use the user's
database and do not enable saved auto-connect in test data.

- Real group/profile import and persistence; selection and deletion confirmation.
- RU/EN Settings persistence across process restart; standard Settings/About/menu
  commands, keyboard selection and VoiceOver names.
- Status-item logo in light/dark menu bar; hide on close, restore, reopen from
  Dock, startup minimized, full quit with confirmed backend termination.
- Routing/DNS form save and reopen; validation errors shown without losing drafts.
- Logs, connection/traffic display and backend operation errors.

Connecting, network latency/speed tests, VPN, system proxy changes and helper
registration are separate operational checks requiring authorization. A compiled
button, configuration validation or synthetic event does not prove networking.
Never run the installer/postinstall to validate PKG assembly.

## Evidence

### Focused isolated evidence

- `native/macos/verify-bridge.sh` — exit 0. Real Swift 6 code imports the C
  header, links the Rust staticlib and uses a disposable Store. Output:
  `PASS: Swift/C/Rust ABI import, snapshots, polling, ru/en persistence, errors
  and ownership; no core or GUI calls`. Reviewed test source and log:
  `.build/bridge-smoke.log`. This does not test SwiftUI rendering.
- `CARGO_TARGET_DIR="$PWD/native/macos/.build/cargo" cargo tree --manifest-path
  crates/macos-bridge/Cargo.toml --locked --prefix none` plus a package-name check
  — exit 0; no GTK/libadwaita/GLib packages. Tree: `.build/dependencies.log`.
- Bridge lane log `.build/bridge-test.log`: 4 passed, 0 failed, including the
  source-included i18n test. Covers temporary Store persistence, event reduction,
  bounded queues, closed handles, retry after controlled shutdown failure,
  concurrent ABI requests, log sink reopening and polling without self-dirtying.
  The final isolated rerun below supersedes the earlier focused run.
- Actual frontend `Backend.swift`/`Model` smoke compiled in strict Swift 6 mode
  and ran with exit 0 (`.build/frontend-smoke.log`): numeric-handle open, real
  Store snapshots/import, partial-import diagnostics, idempotent startup, language
  reopen and same-handle retry after injected transport-close failure. No GUI.
  The final isolated rerun below includes the notice-reset edit and routing
  serialization checks. This test uses frontend code, unlike a stub typecheck.

Temporary Settings stubs used during extraction are only compilation scaffolding
inside ignored `.build`; they are not accepted as full frontend verification.

Warnings currently include unused source-included paths/StopTest and a duplicate
system library linker flag. They are not evidence of network/VPN operation.
Coordinated GUI checks and shared integration remain pending.

### Historical isolated checks — all exit 0 (before shared integration)

Full command/output/exit-status evidence: `.build/acceptance.log`.

| Command | Result |
| --- | --- |
| `bash -n` for each native build/test/package/verification script | Syntax valid |
| `CARGO_TARGET_DIR="$PWD/native/macos/.build/library-tests" cargo test --locked -p throne-config -p throne-store -p throne-ipc` | config 73 passed; store 11 passed; IPC has 0 tests |
| `bash native/macos/test.sh` | 5 passed, including late successful close acknowledgement |
| `bash native/macos/verify-frontend.sh` | Swift/C/Rust smoke plus actual Backend/Model and routing roundtrip PASS; no GUI/core/helper |
| `bash native/macos/build.sh` | Full frontend linked in strict Swift 6 mode; arm64 executable, minimum macOS 13.0 |
| `bash native/macos/package.sh --dmg` | Ad-hoc signed .app verified; plist valid; DMG checksum VALID; both binaries have only system dependencies |

Artifacts from this run:

- `.build/throne-gtk` (not launched).
- `.build/package/run.NV9YLP/Throne GTK.app`.
- `.build/package/run.NV9YLP/Throne-GTK-Native.dmg`.
- `.build/artifact-inspection.log`: architecture, actual deployment targets,
  installed Rust targets and SHA-256 of frontend/bridge/shared source inputs.

The bundle correctly advertises macOS **27.0**, not 13.0, because its unchanged
prebuilt core has LC_BUILD_VERSION minos 27.0. The frontend itself has minos 13.0.
No claim of older-OS runtime compatibility is made for this bundle.

Only native Darwin and wasm Rust targets were installed at artifact inspection;
no actual Linux/Windows build was performed. The GTK app itself was not rebuilt
in this pass. Existing code and root lockfiles remained read-only.

Not established: actual SwiftUI window interaction/appearance, keyboard/VoiceOver
runtime behavior, live connection, live tests, VPN/helper operation, installer
installation, Developer ID signing or notarization. The existing shared engine
still needs lifecycle/cancellation/endpoint fixes before full feature parity.

### Integration in progress

The user explicitly handed over the listed shared files and authorized one GUI
on temporary data without connecting or invoking helper/privileged operations.
The subsequent engine/IPC/workspace changes supersede the isolated binaries;
their current-source tests/build/package must be rerun after writers reconcile.

- Workspace metadata with `--offline --filter-platform aarch64-apple-darwin`
  succeeded. The root lock diff adds only the bridge package, no version bumps.
- Unfiltered offline metadata failed (101: uncached bumpalo); offline nonmac
  dependency inspection failed (101: uncached cpufeatures). Neither was a
  Linux/Windows build. Do not infer a cross-platform build from cfg review.
- Read-only CoreGraphics inspection: active display status 0/count 1;
  `CGPreflightScreenCaptureAccess()` false. Screenshot attempt failed with
  `could not create image from display`; no screenshot/visual-quality evidence.
- Initial GUI inspection used a disposable seeded Store and a nonexistent core
  path; detailed retained evidence is being reconciled under
  `.build/gui-tests/run-1794560936/`. No profile-control or keyboard acceptance
  is inferred merely from observing a window or menu action.
- Empty Foundation substring search was reproduced as false. MainView now uses
  the shared `ProfileSearch` helper with explicit empty/whitespace behavior;
  focused Swift regression and real-header typechecks passed. Full new-source
  frontend smoke and GUI revalidation remain pending.

### Recovery pass: group selection and fresh native build (2026-11-14)

- `bash native/macos/test.sh` — exit 0; 13 tests passed, including engine
  supervision/cancellation and bridge lifecycle tests. Log: `.build/bridge-test.log`.
- `bash native/macos/verify-frontend.sh && bash native/macos/build.sh && git diff --check`
  — exit 0. Swift/C/Rust smoke and actual Backend/Model smoke passed; full SwiftUI
  executable rebuilt at `.build/throne-gtk`. Logs: `.build/bridge-smoke.log`,
  `.build/frontend-smoke.log`, `.build/swift-build.log`.
- The frontend smoke now checks group selection recovery when groups arrive or
  the selected group disappears, and preservation of a valid user choice. MainView
  calls that shared helper when readiness or the group-ID list changes; deletion
  waits for the refreshed list instead of selecting from stale groups.
- `file native/macos/.build/throne-gtk` — exit 0, arm64 Mach-O executable.
  `otool -L native/macos/.build/throne-gtk` — exit 0, system dependencies only;
  no GTK/libadwaita linkage. Executable SHA-256:
  `07b8cd191a09379b7373dc4cdb88b80822c83edb2908522d1cb58b149cdea83f`.

These checks use disposable Store data and do not establish rendering of the
user's existing groups, live networking, VPN, or installed-bundle correctness.
The installed app and `build/throne-gtk` were not replaced by this build command.

- Actual GUI check: `python3 native/macos/.build/gui-verification/check-QlNNQIhv/verify.py`
  — exit 0. The freshly compiled fixture seeded a disposable Store, and the rebuilt
  GUI launched with its explicit `--data-dir`. Accessibility inspection of that
  spawned PID found `Fixture Group` in the group picker and `Example profile` in
  the list (`observation.log:56,64`), plus disconnected status. Normal app quit
  exited 0. Exact child commands/statuses: `commands.log` in the same directory.
  No networking, system-proxy changes, helper actions, or user-data writes were
  requested. Screen capture permission was unavailable, so this establishes
  accessible control content, not pixel-level visual quality or full GUI parity.

### Sidebar hit area and VPN launch layout (2026-11-14)

- Navigation labels now declare a content shape covering the padded rounded row.
  Strict Swift 6 typecheck of all native sources exited 0; pointer hit testing
  has not been repeated in the GUI for this change.
- `bash native/macos/build.sh && THRONE_PACKAGE_OUT="$PWD/native/macos/.build/sidebar-vpn-package" bash native/macos/package.sh --no-dmg && git diff --check`
  — exit 0. Bundle: `.build/sidebar-vpn-package/Throne GTK.app`; plist and ad-hoc
  signature verification passed, system-only dependencies, minimum macOS 27.0.
  Package evidence: `.build/sidebar-vpn-package/run.inJxO1/package.log`.
- The reported VPN rejection is the unchanged helper parent-path security check:
  the development GUI is not beside the running installed helper's core. This
  package has not been installed, no helper was restarted, and no VPN connection
  was tested. Merely launching the package from the build directory will not
  align it with a helper still running from `/Applications/Throne GTK.app`.

### Routing import and editor recovery (2026-11-14)

- Import reads source table metadata and substitutes NULL for absent optional
  suffix/keyword columns. Required-column failures retain context. Synthetic
  legacy/current fixtures cover active-route filtering, mappings and skipped rules.
- `CARGO_TARGET_DIR="$PWD/native/macos/.build/store-tests" cargo test --locked -p throne-store`
  — exit 0, 13 passed. No original database was imported by these tests.
- `bash native/macos/verify-frontend.sh` — exit 0; bridge and frontend/routing
  smokes passed, including preservation of the backend reason in localized errors.
- `bash native/macos/build.sh` and
  `THRONE_PACKAGE_OUT="$PWD/native/macos/.build/routing-fix-package" bash native/macos/package.sh --no-dmg`
  — exit 0. Bundle signature/plists verified; system-only dependencies, minimum
  macOS 27.0. Package log: `.build/routing-fix-package/run.5UX2Lf/package.log`.
  `git diff --check` — exit 0.
- Rule sheet now has a scrollable body, above-field labels, bounded condition
  editors, wrapped guidance and a persistent Cancel/Save footer. Full native
  compilation passed; visual/keyboard acceptance of this revised sheet remains
  pending. Prepared package is not evidence of installation or a successful
  import of the user's actual database.

### Native menu language (2026-11-14)

- `swiftc -swift-version 6 -strict-concurrency=complete -parse-as-library native/macos/Sources/Lifecycle.swift native/macos/Verification/MenuLocalizationSmoke.swift -o native/macos/.build/menu-localization-smoke && native/macos/.build/menu-localization-smoke`
  — exit 0; synthetic RU/EN/RU checks preserve selectors, targets, shortcuts,
  enabled state, auto-enabling and File/Apple menu identity.
- An initial actual-GUI check found `Edit, Вид, Window, Help` despite language en.
  Evidence: `.build/menu-gui-verification/check-DohAugCu/observation.log:11-16`.
  This motivated a bounded View/Вид title fallback for SwiftUI menu actions.
- The harness, `bash native/macos/build.sh`,
  `THRONE_PACKAGE_OUT="$PWD/native/macos/.build/menu-language-package" bash native/macos/package.sh --no-dmg`,
  and `git diff --check` all exited 0 after that correction. Bundle signature and
  plist checks passed; package log: `.build/menu-language-package/run.fFXG2Y/package.log`.
- Actual corrected packaged GUI passed the English menu assertion: `Apple,
  Throne GTK, Edit, View, Window, Help`. Evidence:
  `.build/menu-gui-verification/check-clyTcVDm/observation.log:11-16` and
  `menu-assertion.json`; exact commands in `commands.log`. Only disposable data
  was used, spawned GUI exited normally with status 0, no network/helper actions.
- With user authorization and the installed GUI closed, updated only its GUI
  executable in `/Applications/Throne GTK.app`; installed-bundle signature and
  executable hash checks passed. Core/helper and user data remained unchanged.

### Precommit checks (2026-11-14)

Retained command output and individual exit-status files:
`.build/precommit-checks/20261114-015207-45018/`.

- `CARGO_TARGET_DIR="$PWD/native/macos/.build/cargo" cargo test --locked --workspace --exclude throne-app`
  — exit 0; 106 passed (config 73, IPC 6, bridge/shared engine 14, Store 13).
- `bash native/macos/verify-frontend.sh` — exit 0; both smoke groups passed.
- Strict Swift 6 compilation and execution of `MenuLocalizationSmoke.swift`
  with `Lifecycle.swift` — exit 0; RU/EN/RU assertions passed.
- `bash native/macos/build.sh` — exit 0; full native executable built, not launched.
- `bash -n` for native build/verification scripts and macOS packaging scripts,
  plus `git diff --check` — exit 0.

No GTK frontend, Linux/Windows build, live VPN, helper installation, or complete
Go test suite was run in this gate. The feature branch does not yet incorporate
the four newer stabilization commits on remote `dev`; integration and renewed
network/privilege validation are prerequisites for a combined release.
