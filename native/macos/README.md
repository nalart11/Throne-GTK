# Native macOS frontend

This is an in-process SwiftUI/AppKit frontend for the existing Rust libraries and
Go core. Following the explicit localization handoff, Darwin build and packaging
commands select this frontend; Linux and Windows retain the GTK path.

## Isolated commands

From the repository root, on macOS 13 or newer with Rust and the Swift/Xcode
toolchain (the native frontend uses NavigationSplitView):

```sh
bash native/macos/test.sh
bash native/macos/verify-frontend.sh
bash native/macos/build.sh
```

Outputs, Cargo target files and logs stay in `native/macos/.build/`. The bridge
is a macOS-only root-workspace member and uses the shared lockfile. These direct
commands do not overwrite `build/throne-gtk`; `just build-app` publishes the
native binary and resources there. Building does not launch a GUI.

`verify-bridge.sh` compiles and runs a standalone Swift 6 Foundation program
against the real C header and Rust library, using disposable data and no GUI/core.
`verify-frontend.sh` runs that check and then tests the actual frontend actor and
observable Model, including retry after a controlled close failure.
`package.sh --dmg` assembles/signs an isolated bundle and DMG under `.build/package`
using an already-built adjacent core; it does not install or register anything.

The executable is `.build/throne-gtk`; a real core must be placed immediately
beside it as `throne-gtk-core`. Renaming the GUI or using a core in an unrelated
directory does not satisfy the core's parent-process authentication. Do not
weaken that check. Only the existing helper may launch a privileged VPN core.

VPN additionally requires the GUI to be beside the core of the **running helper**.
For example, a GUI launched from `.build/` cannot use a helper running from
`/Applications/Throne GTK.app/Contents/MacOS/throne-gtk-core`, even if `.build/`
also contains a core. Package the current build, install it at the intended app
location, and launch that installed app rather than the loose development binary.
Installing or refreshing a privileged helper is a separate, explicitly authorized
operation; packaging alone neither installs the app nor refreshes the helper.

The isolated `.app`/DMG checks passed with the existing core, but that core was
built with minimum macOS **27.0**. Packaging derives the real minimum from both
binaries, so this particular bundle requires 27.0 despite the frontend's 13.0
target. No older-system runtime test has been performed.

## Existing data and language

Normal use opens the same database:
`~/Library/Application Support/throne-gtk/throne-gtk.db`. Do not launch the GTK
and native frontend against the same installation at the same time.

Rust owns all persistent state. Language is the existing Settings field
`language` (`ru`/`en`), default Russian for old databases; there is no parallel
UserDefaults locale. The native controls can refresh their presentation from
the saved field; source-included engine diagnostics keep their startup locale.

Tests use disposable directories and must not open the user's database, change
system proxy, connect VPN or register a helper. The user granted a coordinated
GUI-check slot for temporary data only. Networking and VPN are not proven by
an isolated build or configuration validation.

## Design and acceptance

- `BRIDGE.md`: C ABI, ownership and request/snapshot contract.
- `IMPLEMENTATION.md`: scope, real GTK scenario inventory and authorized handoff.
- `VERIFICATION.md`: acceptance map and executed evidence.

Shared engine supervision and IPC lifecycle changes are undergoing integrated
verification. Consult `VERIFICATION.md` for current evidence and remaining GUI,
networking, privilege and cross-platform limitations rather than treating an
older successful build as proof of the current source.
