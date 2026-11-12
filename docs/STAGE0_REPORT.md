# Stage 0 stabilization — verification record

Date: 2026-11-12. Host: Linux x86_64. Status: **incomplete; native macOS validation required**.
No macOS completion, Windows implementation, release or PR is claimed.
The user subsequently authorized fixing deployment/DNS blockers and committing
and pushing a `dev` handoff; the initial results below predate that follow-up.

## Implemented scope

- `crates/ipc/src/lib.rs`, `crates/ipc/Cargo.toml`, `Cargo.lock`: isolated IPC
  endpoints, Unix peer authentication, bounded admission/deadlines and terminal
  transport teardown, including retained client clones.
- `crates/ipc/src/service_macos.rs`: bounded helper registration/control,
  authenticated root helper and exact returned worker PID. Darwin ACL semantics
  match Apple's `acl_get_entry(3)`: zero means an entry was obtained.
- `crates/app/src/engine.rs`: owned test/subscription cancellation, generation
  filtering, disconnect detection and failed polling transitions.
- `core/dispatch.go`, `core/framing*.go`: frame caps, partial-frame/write
  deadlines, full writes, admission limits and disconnect cleanup.
- `core/helper_darwin.go`, `core/helpertrust/`: authenticated helper/worker,
  protected-install policy, signature checks, singleton socket recovery and
   watchdog cleanup. Native deployment must still be validated as described below.
- `core/privileged_policy*.go`: explicit privileged RPC/configuration surface,
  recursive positive schema and disabled privileged disk cache.
- `core/internal/boxmain/cmd_run*.go`, `core/server.go`: removal of raw config
  logs and credential-bearing decoder diagnostics; invocation-owned Start
  rollback, cancellation checks and repeated-Start ownership preservation.
- `core/lifecycle_test.go`: minimal real Start/Start/Stop and cancelled queued
  Start regression tests.
- `core/internal/xray/constructor.go`, `gate.go`, `constructor_test.go`:
  credential-safe construction/gate diagnostics and sentinel regression tests.
- `core/internal/boxdns/winipcfg/*.go`: Windows-only build constraints, fixing
  non-Windows package discovery without implementing Windows features.
- `crates/config/examples/privileged_fixtures.rs`,
  `core/testdata/privileged/`: fifteen reproducible synthetic requests generated
  by the actual Rust configuration code, accepted without running a VPN.
- `crates/app/examples/ipc_smoke.rs`, `Justfile`: isolated concurrent-core,
  loopback HTTP proxy, shutdown/cleanup and restart smoke test. Cronet copying
  now uses `install -m 644`: module-cache permissions no longer break rebuilds.
- `.github/workflows/stabilization.yml`: Linux/macOS test/build automation,
  **not executed on hosted runners**.
- `docs/CROSS_PLATFORM_PLAN.md`, `docs/MACOS_VALIDATION.md`: sequential gates
  and explicit native/manual acceptance matrix.

## First reconciled Linux check

All Go commands used `GOTOOLCHAIN=go1.26.0`, matching the Justfile rather than
the installed Go 1.27.4. These results precede the final lifecycle remediation;
changed Go paths must be revalidated afterward.

| Command | Actual result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo test --workspace` | exit 0; 96 tests passed, no failures |
| `cargo clippy --workspace --all-targets` | exit 0; warnings remain |
| `gofmt -l` on changed Go sources | empty output, exit 0 |
| `cd core && go test ./...` | exit 1: Windows-only `internal/boxdns/winipcfg` lacks platform constraints; other listed packages passed |
| `cd core && go vet ./...` | exit 1: same package-selection failure |
| `cd core && go test -race . ./helpertrust ./internal/boxmain` | exit 0 |
| `just build` | exit 0; Linux app/core built, Cronet copied |
| `just ipc-smoke` | exit 0; concurrent cores, loopback HTTP proxy, isolated shutdown, restart and cleanup passed |
| `git diff --check` | exit 0 |

Logs for this development session: `/tmp/opencode/throne-stage0-checks/`.
They are temporary local evidence, not a portable release artifact.

## Final changed-Go validation after remediation

Unchanged Rust source retains the passing results above. All following Go
commands again used `GOTOOLCHAIN=go1.26.0`:

| Command | Actual result |
| --- | --- |
| `cd core && go test ./...` | exit 0 |
| `cd core && go vet ./...` | exit 0 |
| `cd core && go test -race . ./helpertrust ./internal/boxmain ./internal/xray` | exit 0 |
| `cd core && go test -ldflags=-checklinkname=0 -tags "$TAGS" ./...` | exit 0 |
| `cd core && go vet -tags "$TAGS" ./...` | exit 0 |
| `just build` followed by a second `just build` | both exit 0 after Cronet copy fix |
| `just ipc-smoke` against the rebuilt current core | exit 0 |
| `git diff --check` | exit 0 |

Here `$TAGS` was exactly the Linux shipped set from the Justfile:

```text
with_clash_api,with_gvisor,with_quic,with_wireguard,with_utls,with_dhcp,with_tailscale,with_purego,with_naive_outbound,badlinkname,tfogo_checklinkname0
```

The first rebuild attempt failed copying the existing read-only Cronet artifact
(`cp: Permission denied`), which prompted the copy fix and two successful
consecutive builds. The subsequent smoke test used the newly rebuilt core.
A focused shipped-tag lifecycle test without `-checklinkname=0` failed dependency
linking; the shipped-tag validation above includes the Justfile's linker setting.
The final scoped review found the reported lifecycle defects addressed, but
explicitly withheld full stage approval because native/deployment gates remain.

## Authorized macOS handoff follow-up

The protected full bundle now installs at
`/Library/Application Support/Throne GTK/Throne GTK.app`; `/Applications` holds
only a convenience link. Trust derives from protected installed static code and
live Security.framework checks, not an administrator-writable alias. Framework
aliases are accepted only within the protected closure. Release uses hardened
signed code; explicit manual-development installation pins ad-hoc code hashes.
Shared directory permissions are never changed to bypass validation.

The PKG scripts separate manual development from signed/notarized release,
refuse foreign installation items, and stop owned manual services before
replacement. SMAppService removal is explicitly requested in user context via
`--unregister-helper`; `bootout` is not presented as unregistering it. Profiles
and SQLite data are not removed. Bundle validation now includes Mach-O
dependency paths and LC_RPATH. CI includes portable packaging guards, isolated
proxy smoke, and macOS ad-hoc DMG/PKG construction without privileged install.

DNS configuration uses an SCPreferences transaction and ownership-aware durable
journal rather than `networksetup`/`Empty` resets. Its scope and native fault
tests are documented in [MACOS_DNS.md](MACOS_DNS.md). The Mac continuation entry
point is [MACOS_HANDOFF.md](MACOS_HANDOFF.md).

### Reconciled handoff validation

These commands ran on the final handoff sources, again with
`GOTOOLCHAIN=go1.26.0`. Logs: `/tmp/opencode/throne-dev-handoff-checks/`.

| Command | Actual result |
| --- | --- |
| `cargo fmt --all -- --check` | first exit 1 (layout/import ordering only); `cargo fmt --all` applied, final check exit 0 |
| `cargo test --workspace` | exit 0; 96 tests passed |
| `cargo clippy --workspace --all-targets` | exit 0; warnings remain |
| `gofmt -l` on changed/new Go files | empty output, exit 0 |
| `cd core && go test ./...` | exit 0 |
| `cd core && go vet ./...` | exit 0 |
| `cd core && go test -race . ./helpertrust ./internal/boxmain ./internal/xray ./internal/sys` | exit 0 |
| `cd core && go test -ldflags=-checklinkname=0 -tags "$TAGS" ./...` | exit 0; exact shipped Linux tags listed above |
| `cd core && go vet -tags "$TAGS" ./...` | exit 0 |
| `bash -n` separately for packaging, uninstall, guard-test and installer hook scripts | exit 0 |
| `bash scripts/test-macos-packaging.sh` | exit 0; portable/mocked guards, not installation |
| `just build` | exit 0 |
| `just ipc-smoke` | exit 0; current core, isolated IPC and loopback proxy, no Internet/TUN |
| `GOOS=darwin GOARCH=amd64 CGO_ENABLED=0 go test -c ./internal/sys` (from `core`, output under the log directory) | exit 0; no native C compilation or execution |
| same no-cgo compile with `GOARCH=arm64` | exit 0; same limitation |
| `git diff --check` | exit 0 |

Scoped follow-up review found the four reported blocker corrections present:
nonblocking preferences lock, inode ACL/private journal validation, helper
startup/worker-exit recovery, and pre-extraction installer ACL protection.
Administrative recovery runs before payload removal/replacement and fails on
busy/conflict. Native API-symbol review also corrected the public Security
function to `SecCodeCopyDesignatedRequirement` using Apple's declaration.
This is a development handoff, not production or macOS-stage approval.

## Acceptance blockers and limits

- Constructors cannot be interrupted mid-call. Cancellation checks prevent
  eventual publication and roll back resources, but the five-second cleanup
  budget cannot guarantee completion if construction or rollback hangs. The
  deterministic cancellation regression covers queued Start, not a slow
  constructor injection.
- Xray constructor errors no longer wrap arbitrary configuration diagnostics;
  upstream internal logging and all protocol/runtime errors have not received
  a comprehensive redaction audit.
- Protected installation, native signature/ACL validation, SMAppService and
  manual installation/update/removal still need actual Mac execution. Source
  changes remove the known always-reject `/Applications` ancestry policy; that
  is not runtime proof of a deployable package.
- Privileged schema v1 is a reviewed subset, not arbitrary sing-box/Xray support;
  native Xray non-VLESS configurations/extensions fail explicitly.
- DNS covers only the initial service/location. Migration and effective
  resolver behavior require real-Mac checks. No real-Mac DNS modification was
  performed during this work; leak protection is not established.
- No Darwin build, helper registration, AppKit, TUN, DNS/IPv6 leak, sleep/wake,
  clean-machine bundle, DMG/PKG, signing or notarization result is established.
- The user authorized committing/pushing this handoff to `dev`. Hosted/native
  results must be read separately; successful local checks are not Mac evidence.
- Synthetic fixtures establish configuration-policy compatibility, not remote
  VLESS/Reality/XHTTP/Hysteria2/TUIC/AnyTLS/Naive handshakes.
- `prowl review plan --structured` could not run because the repository has no
  initialized Prowl workspace. Bounded native review found the blockers above;
  no complete structured-review approval is claimed.

Next allowed work: run the macOS handoff checks and complete macOS integration.
Windows stays gated.
