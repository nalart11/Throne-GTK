# Cross-platform implementation plan

Status: **stage 0 in progress; macOS not complete; Windows not started**.
Audit date: 2026-11-12. Development host: Linux x86_64.

## Order and gates

1. Audit and stabilize shared IPC, process ownership, cancellation and state.
2. Complete macOS, preserving GTK4/libadwaita and the existing Go core/helper.
3. Start Windows only after macOS implementation and available checks pass.
4. Consolidate platform/backend boundaries, then final regression and documentation.

The user authorized committing and pushing this stabilization handoff to `dev`
on 2026-11-12. No PR or release publication is authorized. Existing untracked
`patch` is not part of this work. No profile/schema migration is planned.

## Initial source audit

Findings are implementation facts, not README promises:

| Area | Evidence | Required action |
| --- | --- | --- |
| Concurrent IPC | `crates/ipc/src/lib.rs`: endpoint derived solely from GUI PID; Unix path removed before binding | Unique endpoint per core, private socket ownership, cancellation-safe cleanup; never unlink another core's endpoint |
| Framing | `core/dispatch.go`: peer-controlled allocation lengths and single response writes | Limit incoming/outgoing frames, handle short writes, regression tests |
| Transport failure | `crates/ipc/src/lib.rs`: reader drains pending replies but writer may remain; RPC has no deadline | Close both directions, fail queued/in-flight calls, expose liveness and bound waits |
| Test cancellation | `crates/app/src/engine.rs`: test cores run separately; StopTest targets connection core | Track and cancel test operations and their own resources; clean up on engine shutdown |
| Child ownership | Rust spawn already sets `kill_on_drop(true)` | Preserve it; verify explicit/error/cancel paths rather than claim it is missing |
| macOS helper | `core/helper_darwin.go`, `core/parentcheck/parentcheck.go`: peer credentials plus sibling executable path/name | Strengthen privileged trust beyond name/path; bound requests and handle stale socket safely |
| macOS packaging | Existing `scripts/package-macos*.sh`, launchd resources and SMAppService client | Extend existing machinery, distinguish local development from signed/notarized release |
| macOS menu bar | `crates/app/src/tray_macos.rs`: Show/Quit only | Add connection/server state and actions on AppKit main thread |
| macOS runtime | Linux host cannot establish launchd, AppKit, TUN, sleep/wake or clean-Mac behavior | Add macOS CI and manual matrix; do not equate compilation with working VPN |

The Prowl index is not initialized; initial `prowl overview` failed. Native
bounded source inspection is used instead. Toolchain available locally:
Rust 1.93.1, Go 1.27.4, protoc, GTK 4.24.1, libadwaita 1.10.0.
GitHub CLI authenticates; the subsequent explicit authorization is limited to
the `dev` handoff, not a release or PR.

## Stage 0 — stabilization

Pass goal: isolate concurrent cores, bound malformed/hung IPC, correctly cancel
tests and keep UI connection state consistent with core availability.

Before implementation, review security/concurrency contracts. Separate writer
scopes: Rust IPC, Go framing, application lifecycle. Tests must exercise actual
framing/channel/process boundaries where practical. Reconcile writers before a
single broad validation run. Record unresolved findings rather than hide them.

Acceptance evidence:
- Concurrent endpoints do not collide or delete each other; peer mismatch rejected.
- Oversized headers rejected before allocation; fragmented writes complete.
- Transport loss fails pending and future calls; shutdown and cancellation bounded.
- StopTest cancels the testing operation, not the connection core.
- Rust workspace tests/checks, Go tests, formatting and focused static checks.

Additional security findings from review:
- Production helper checks a sibling executable path, so exposure is conditional
  on installation integrity or trusted-process compromise; debug builds bypass
  that check. Neither path matching nor a client debug flag is authorization.
- Privileged input needs policy even after authenticating the GUI: imported raw
  outbound JSON can contain local-file, executable or management facilities.
  Extra-process startup currently drops Unix privileges; this is not evidence of
  arbitrary root execution through that field.
- The helper must return its worker PID; accepting any root peer is insufficient.
- Removing raw Start logs is insufficient: decoder errors also echoed full
  configurations. Regression tests must check malformed secret-bearing input.
- Frame caps do not alone bound aggregate memory. Handler admission must retain
  control capacity, and disconnect cleanup must have a bounded wait.

The active hardening pass covers these findings before any macOS feature work.
Native authorization, ACL semantics and network teardown remain real-Mac gates.

### Implemented, not yet accepted as a completed stage

- Fresh private Unix endpoints per core and non-reused Named Pipe names;
  Unix PID/UID authentication, including the exact helper worker PID.
- Bounded request/response framing, handler admission, RPC deadlines and terminal
  transport failure; owned cancellation of tests and subscription workers.
- Helper requests authenticated independently of debug mode, restricted privileged
  configuration schema, disk-cache disabling, and live GUI signature validation.
- Fifteen reproducible synthetic Rust-generated VPN fixtures, policy rejection
  tests, and an isolated loopback IPC/proxy smoke example (`just ipc-smoke`).
- Linux/macOS build-test workflow in `.github/workflows/stabilization.yml`.
  Adding this workflow does **not** mean that hosted checks have run.

Acceptance review and remaining blockers:

1. Rejected repeated Start now preserves the running instance; a real minimal
   Start/Start/Stop regression test passes.
2. Dispatch now cancels its session before cleanup; queued Start checks under
   the lifecycle lock and active Start rolls back on cancellation. Constructors
   remain non-interruptible; a hung constructor/rollback may outlast cleanup.
3. Native Xray construction errors now discard credential-bearing underlying
   diagnostics; sentinel tests cover malformed VLESS flow/encryption and gate
   errors. This is not a comprehensive audit of upstream internal logging.
4. Helper trust and PKG layout now use the protected complete bundle under
   `/Library/Application Support/Throne GTK`, with only a convenience link under
   `/Applications`. Contained Frameworks links and harmless ACLs are permitted;
   mutation grants are rejected. SMAppService is preserved; manual development
   registration is explicit, never an authentication-failure fallback. Actual
   installation, signature validation, approval and migration still require Mac
   tests. Shared system-directory permissions must not be changed.
5. Privileged schema v1 supports a reviewed subset, including native VLESS/XHTTP;
   arbitrary native Xray protocols/extensions are explicitly rejected. This is
   not full compatibility with arbitrary upstream configuration JSON.
6. macOS DNS handling now uses an SCPreferences lease, durable root-protected
   journal and compare-before-restore instead of shell-based `Empty` reset.
   Portable transaction tests do not validate the effective resolver. Only the
   initial network service is covered; location/network migration and native
   fault/leak testing remain required. See [MACOS_DNS.md](MACOS_DNS.md).

See `core/helpertrust/README.md` for policy and `docs/MACOS_HANDOFF.md` for the
protected installation procedure. Darwin compilation, ACL/Security.framework behavior, installation and
network behavior remain unverified on this Linux host.
Final Linux checks and the repeated-build Cronet permissions fix are recorded in
[STAGE0_REPORT.md](STAGE0_REPORT.md). Passing Linux checks does not lift the
macOS deployment/native-validation gate.

## Stage 1 — macOS (absolute priority)

1. Harden existing SMAppService/helper trust and worker lifecycle. Define signed
   release identity and safe explicit development policy; no root GUI.
2. Verify .app resource paths, dylib/resource closure, metadata and helper layout;
   preserve parent verification. Support architecture-specific Intel/ARM builds.
3. Review sing-box TUN routing, DNS/IPv6, outbound exclusion and cleanup. Keep
   current backend unless evidence shows a necessary change.
4. Implement opt-in system proxy via macOS APIs with durable ownership-aware
   snapshot/recovery, network changes and no writes when disabled.
5. Extend menu bar state/actions, hidden-window behavior and user-session
   autostart with actual system state; add UI only through designer-owned work.
6. Extend existing DMG/PKG paths for explicit development/release signing,
   Hardened Runtime, notarization/stapling using external credentials only.
7. Add macOS CI build/tests/plist/package inspection and documented manual
   permission, GUI/helper, proxy/TUN, network-transition and upgrade/remove tests.

Gate: implementation complete, available automation passed, manual limitations
explicit. Missing platform/access is a blocker, not a successful stage outcome.
Never claim DNS/IPv6 leak protection without corresponding network evidence.

## Stages 2–4 — gated, not active

Windows: existing MSYS2 runtime packaging; least-privilege service and protected
Named Pipes; Wintun, system proxy, tray, autostart, installer and CI. No Windows
implementation until macOS gate. Shared stage-0 security fixes may incidentally
affect common Windows transport, but are not a Windows feature stage.

After both platforms: small compile-time platform boundaries and justified core
capabilities API, no new backend, no lossy profiles. Test imports, subscriptions,
SQLite, routing/DNS, stats, tests and Linux integration. Protocol handshakes
require real configurations; untested protocols must be marked unverified.

## Verification ownership and limitations

Writers own focused tests; integrator inspects artifacts and runs broad checks.
Linux can validate shared Rust/Go behavior and shell syntax, not macOS runtime.
CI definitions alone are not evidence of a CI run. macOS execution on changed
source needs an authorized way to supply that source to a macOS runner; do not
push or publish to obtain it without permission. Signing/notarization also need
Developer ID/Keychain credentials. No credentials are stored in this repository.
The manual acceptance matrix is in [MACOS_VALIDATION.md](MACOS_VALIDATION.md).

Final stage reports must list changed files, exact commands/results, known
limitations and the next allowed stage. This file will track implemented
results separately from the intended work above.
