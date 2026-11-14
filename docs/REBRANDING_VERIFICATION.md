# Nolurix rebranding verification

Verified on Linux x86_64, 2026-11-14, against the former GTK baseline
`bfe0dfc27037ca8165190165ca950b625b0d183b`. This is implementation evidence,
not a claim that native macOS or Windows releases have been accepted.

## Delivered identity and compatibility

- Product: **Nolurix**, repository `nalart11/Nolurix`, branch `main`.
- Executables: `nolurix`, `nolurix-core`; Windows executable suffixes retained.
- Desktop/bundle ID: `dev.nalart.Nolurix`; macOS bundle: `Nolurix.app`.
- Default storage: platform configuration root plus `nolurix/nolurix.db`.
- Workspace packages, Rust bridge ABI/header, native Swift entrypoint,
  packaging, helper labels, IPC endpoints and new release assets use Nolurix.
- Existing icons and screenshots were byte-compared with their baseline
  counterparts: no pixel/design changes. Screenshots in the READMEs are
  explicitly marked as historical pre-rebrand captures.
- Upstream Throne attribution, GPL notice, database/manual routing imports,
  third-party dependency URLs and supported subscription formats are retained.

See [MIGRATION.md](MIGRATION.md) for upgrade instructions. Startup migration
uses SQLite online backup, including committed WAL contents, private staging,
integrity checks and atomic no-clobber publication. It preserves the original
source and all main-database settings/groups/profiles/subscriptions/routes.
An existing destination directory wins without merging, including an empty
one. Explicit bridge storage overrides do not import unrelated default data.
Normal BoltDB core caches are deliberately regenerated; optional SQLite
caches use the same snapshot mechanism. Runtime credentials/logs are excluded.

## Commands actually run

All commands below exited **0**, unless explicitly limited in the next section.
Rust/Cargo: 1.93.1. Go: explicitly `GOTOOLCHAIN=go1.26.0`.

| Check | Evidence |
| --- | --- |
| `cargo fmt --all -- --check` | Clean formatting |
| `cargo check --workspace --all-targets --locked` | Linux workspace/targets compile |
| `cargo test --workspace --locked` | 128 tests passed: app 16, config 73, IPC 17, store 22 |
| `cargo clippy --workspace --all-targets --locked` | Passes with non-fatal warnings; not a warning-free claim |
| `bash scripts/test-linux-packaging.sh` | New archive layout, metadata, permissions and safety fixtures |
| `python3 scripts/test-source-install.py` | Actual source-install recipe, denied authorization on first install/upgrade, successful retries, modified-target refusal; no elevation |
| `bash scripts/test-install.sh` | Checksums, hostile archives/destinations, rollback, legacy fallback and preservation fixtures |
| `bash scripts/test-macos-packaging.sh` | Portable bundle/ACL/recovery guards; 12 legacy-path fixtures and both loaded labels |
| `cd core && go test ./...` | Full default Go tests with pinned toolchain |
| `cd core && go vet ./...` | Full Go vet with pinned toolchain |
| `cd core && go test -ldflags=-checklinkname=0 -tags with_clash_api,with_gvisor,with_quic,with_wireguard,with_utls,with_dhcp,with_tailscale,with_purego,with_naive_outbound,badlinkname,tfogo_checklinkname0 ./...` | Exact Linux shipped feature set |
| `cd core && go test -race ./helpertrust ./internal/sys` | Trust and DNS state-machine race checks |
| `just build` | Real renamed GTK application/core and Cronet integration |
| `just ipc-smoke` | Concurrent isolated cores, loopback HTTP proxy, shutdown/restart/cleanup; no Internet or TUN |
| `cargo check -p nolurix-ipc --target x86_64-pc-windows-gnu --locked` | Windows Rust IPC cross-check only; two existing unused-variable warnings |
| `cd core && GOOS=windows GOARCH=amd64 CGO_ENABLED=0 go build ./ipc ./parentcheck` | Windows Go IPC/parent-auth cross-compilation only |
| `desktop-file-validate resources/nolurix.desktop` | Desktop file valid |
| Shell syntax and Python AST checks | Packaging, installer and native shell scripts parse; Python scripts parse |
| `git diff --check` | No whitespace errors |

An additional temporary Xvfb/private-D-Bus GUI harness launched the actual
`build/nolurix` three times with isolated HOME/XDG directories: fresh startup,
former-GTK startup migration, and existing Nolurix startup. Each remained
running, created/opened a valid database and preserved the legacy source.
The migration screenshot was opened and showed the normal application window.
The harness terminated each process with SIGTERM; it did not exercise graceful
window-close behavior, connections, TUN or system proxy. Initial harness
execution failed because `xwininfo` was unavailable; removing that optional
inspection step allowed the screenshot/database checks to complete. Isolated
desktop-portal services emitted host-integration warnings.

Local detailed logs are in `/tmp/opencode/nolurix-checks/`; GUI evidence is in
`/tmp/opencode/nolurix-gui-pz9gd4nu/`. These are transient local artifacts,
not files needed to build or run Nolurix.

## Security and intentional legacy references

- The new macOS privileged installer refuses former protected paths,
  Applications links (including dangling links), plists and both loaded
  labels before installed-payload execution or mutation. It requires explicit
  original-app user-context unregistering and verified old-install retirement.
  No alternate untrusted GUI/core location is admitted.
- Root ownership, ACL, signature, canonical-path and symlink checks remain
  enforced. Portable fixtures do **not** establish native Service Management,
  authorization, DNS or installation behavior.
- `/private/var/db/throne-gtk-dns` and the existing helper wire-protocol magic
  intentionally remain compatible. The shared journal/permanent lock prevents
  orphaned recovery state or a second DNS lease domain; it does not authorize
  an old executable or bundle identity.
- `throne-gtk` storage names occur only as migration sources/fixtures or explicit
  legacy integration/retirement instructions. Original upstream Throne import
  names are a separate supported format, not stale Nolurix branding.
- Private `throne_safe_acl`/`throne_signature` C names remain synchronized with
  their shared security-header definitions; the exported helper ABI and checked
  executable identity are Nolurix. Portable adapter/header symbol checks prevent
  accidental caller-only renames but do not replace native compilation.
- Historical verification documents retain the names and commands actually
  observed before this change. They are not new Nolurix verification results.

## Unavailable checks and known limits

- Final focused review found and fixed two regressions: macOS adapter/header
  symbol mismatch and source-install retry after denied capability authorization.
  Both now have passing portable regression checks. The additional exhaustive
  structured review stopped when all four reviewers reached their usage limit;
  its receipt/audit coverage is incomplete and no structured-review approval is
  claimed. Native platform acceptance remains outstanding as described below.

- No native macOS Swift/AppKit build, signing/notarization, DMG/PKG installation,
  SMAppService unregister/register, privileged helper or real DNS recovery test
  was possible on this Linux host. A macOS runner and manual installation
  acceptance are still required. No installer or GUI was run as root.
- No Windows runtime, GTK packaging/elevation or system-proxy test was performed.
  The broader Windows Rust config cross-check exited **101** because the MinGW
  compiler required by `ring` is absent. The narrower IPC checks above passed.
- No privileged Linux installation, `setcap`, TUN or real system-proxy change
  was performed. Installer runtime-library checks use fixtures/mocks.
- Clippy reports non-fatal warnings. Prowl structural diagnostics include
  module/test back-reference cycles, fixture credential literals and large
  existing files; they are not a substitute for compiler/tests.
- Host-default Go 1.27.4 fails dependency `http2.Transport.connPool` linknames,
  also reproduced on the unchanged baseline. Pinned Go 1.26.0 passes; keep it.
- Published `v0.2.1` currently contains only historical Throne GTK assets.
  Default installation intentionally fails closed until a Nolurix asset exists.
  Only explicit `--version 0.2.1` plus an exact HTTP 404 permits checksum-verified
  historical fallback, retaining the old executable/desktop/config identity.
  Transport/server/checksum errors never trigger that fallback. No release was
  published or existing asset rewritten by this work.
