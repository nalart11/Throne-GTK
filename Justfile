# Build and run Nolurix.
set unstable
set lists

core_tags := "with_clash_api,with_gvisor,with_quic,with_wireguard,with_utls,with_dhcp,with_tailscale,with_purego,with_naive_outbound,badlinkname,tfogo_checklinkname0"

# sing-box and sing-trusttunnel use //go:linkname to access internals of
# golang.org/x/net/http2; in Go 1.27 the required symbol is inlined and linking fails.
# Build the core with the 1.26 toolchain until the Throneproj/sing-box fork incorporates the fix.
export GOTOOLCHAIN := "go1.26.0"
RELEASE := bool(env("RELEASE", "0"))

# Run the GUI
run-app: build
    #!/usr/bin/env bash
    set -euo pipefail
    suffix=""
    [[ "$(go env GOOS)" == "windows" ]] && suffix=".exe"
    "./build/nolurix${suffix}"

# Build everything: core and interface
[group('build')]
build: build-core fetch-cronet build-app

# Build the core (Go, sing-box + Xray)
[group('build')]
build-core:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p build
    suffix=""
    cgo=1
    cgo_ldflags=""
    if [[ "$(go env GOOS)" == "windows" ]]; then
        suffix=".exe"
        cgo=0
    fi
    tags="{{ core_tags }}"
    # The Darwin Cronet module ships libcronet.a, not a dylib.  purego would
    # compile successfully but then fail to load Cronet when Naive is used.
    if [[ "$(go env GOOS)" == "darwin" ]]; then
        tags="${tags/,with_purego/}"
        cgo_ldflags="-framework UniformTypeIdentifiers"
    fi
    version="$(cd core && go list -m -f '{{{{.Version}}' github.com/sagernet/sing-box)"
    (cd core && CGO_ENABLED="$cgo" CGO_LDFLAGS="$cgo_ldflags" \
        go build -o "../build/nolurix-core${suffix}" -trimpath \
        -ldflags "-w -s -X 'github.com/sagernet/sing-box/constant.Version=${version}' -checklinkname=0" \
        -tags "$tags")

# Regenerate committed Go protobuf sources after editing core/gen/libcore.proto.
generate-core-proto:
    cd core/gen && protoc -I . --go_out=. --go-grpc_out=. libcore.proto

# Build a self-contained macOS app bundle and DMG.
[group('packaging')]
dmg:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "$(uname -s)" != "Darwin" ]]; then
        echo "DMG можно собрать только на macOS" >&2
        exit 1
    fi
    RELEASE=1 just build
    ./scripts/package-macos.sh

# Build an unsigned local PKG which installs the app and root-helper with one
# administrator prompt. It is intended for personal use without Developer ID.
[group('packaging')]
installer: dmg
    ./scripts/package-macos-installer.sh --dev-manual

# Developer ID application/installer identities and notary Keychain profile required.
[group('packaging')]
installer-release: dmg
    ./scripts/package-macos-installer.sh --release

# Build the interface (native Swift on Darwin, GTK elsewhere)
[group('build')]
build-app:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p build
    if [[ "$(go env GOOS)" == "darwin" ]]; then
        ./native/macos/build.sh
        install -m755 native/macos/.build/nolurix build/nolurix
        mkdir -p build/Resources
        cp -R native/macos/.build/Resources/. build/Resources/
        exit 0
    fi
    suffix=""
    [[ "$(go env GOOS)" == "windows" ]] && suffix=".exe"
    cargo build {{ if RELEASE == "true" { "--release" } }} -p nolurix-app
    cp "target/{{ if RELEASE == "true" { "release" } else { "debug" } }}/nolurix${suffix}" build/

# Place libcronet near core to enable Naive
[group('build')]
fetch-cronet:
    #!/usr/bin/env bash
    set -euo pipefail

    goos="$(go env GOOS)"
    goarch="$(go env GOARCH)"
    module="github.com/sagernet/cronet-go/lib/${goos}_${goarch}"
    module_dir="$(cd core && go list -m -f '{{{{.Dir}}' "$module" 2>/dev/null || true)"

    case "$goos" in
        linux)   library_name="libcronet.so" ;;
        darwin)
            # Cronet is linked from libcronet.a while building the core.
            if [[ -f "${module_dir}/libcronet.a" ]]; then
                echo "Cronet статически слинкован с ядром"
                exit 0
            fi
            echo "libcronet.a не найдена в модуле ${module} — naive работать не будет" >&2
            exit 0
            ;;
        windows) library_name="libcronet.dll" ;;
        *)
            echo "Cronet для ${goos}/${goarch} не поддержан — naive работать не будет" >&2
            exit 0
            ;;
    esac

    library="${module_dir}/${library_name}"
    if [[ ! -f "$library" ]]; then
        echo "${library_name} не найдена в модуле ${module} — naive работать не будет" >&2
        exit 0
    fi

    mkdir -p build
    # Go module-cache libraries are read-only; keep repeated builds writable.
    install -m 644 "$library" "build/${library_name}"
    echo "${library_name} → build/"

# Tests
[group('test')]
test:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "$(uname -s)" == "Darwin" ]]; then
        cargo test --workspace --exclude nolurix-app
        ./native/macos/verify-frontend.sh
    else
        cargo test --workspace
    fi

# Optional GTK frontend tests, including on macOS with GTK installed.
[group('test')]
test-gtk:
    cargo test -p nolurix-app

# End-to-end check with real data: import from Throne and validate configs with the core.
# The binary is copied under the name nolurix — the core does not accept another parent.
[group('test')]
selftest:
    #!/usr/bin/env bash
    set -euo pipefail
    suffix=""
    [[ "$(go env GOOS)" == "windows" ]] && suffix=".exe"
    cargo build -p nolurix-app --example selftest
    mkdir -p target/selftest
    cp "target/debug/examples/selftest${suffix}" "target/selftest/nolurix${suffix}"
    cp "build/nolurix-core${suffix}" target/selftest/
    "./target/selftest/nolurix${suffix}"

# Loopback-only IPC/lifecycle smoke; uses the existing core without rebuilding Go.
[group('test')]
ipc-smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    suffix=""
    [[ "$(uname -s)" == MINGW* || "$(uname -s)" == MSYS* ]] && suffix=".exe"
    cargo build --example ipc_smoke -p nolurix-app
    test -x "target/debug/examples/ipc_smoke${suffix}"
    test -x "build/nolurix-core${suffix}"
    mkdir -p target/ipc-smoke
    cp "target/debug/examples/ipc_smoke${suffix}" "target/ipc-smoke/nolurix${suffix}"
    cp "build/nolurix-core${suffix}" "target/ipc-smoke/nolurix-core${suffix}"
    "./target/ipc-smoke/nolurix${suffix}"

# Clean build data
clean:
    cargo clean
    rm -rf build target/selftest

# Grant the core TUN permissions (repeat after each reinstallation: the file is recreated)
[group('installation')]
grant-tun:
    sudo setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep build/nolurix-core

# Install to ~/.local: the application will appear in the application list
[group('installation')]
install: build
    #!/usr/bin/env bash
    set -euo pipefail
    root="."
    prefix="${PREFIX:-$HOME/.local}"
    libdir="$prefix/lib/nolurix"
    python3 scripts/uninstall-linux.py --check-install "$prefix"
    mkdir -p "$prefix/share/applications"

    for binary in nolurix nolurix-core; do
        if [[ ! -x "$root/build/$binary" ]]; then
            echo "Нет $root/build/$binary — сначала соберите проект (just build)" >&2
            exit 1
        fi
    done

    install -Dm755 "$root/build/nolurix"      "$libdir/nolurix"
    install -Dm755 "$root/build/nolurix-core" "$libdir/nolurix-core"
    if [[ -f "$root/build/libcronet.so" ]]; then
        install -Dm644 "$root/build/libcronet.so" "$libdir/libcronet.so"
    fi

    install -Dm644 "$root/resources/icons/dev.nalart.Nolurix.svg" \
        "$prefix/share/icons/hicolor/scalable/apps/dev.nalart.Nolurix.svg"
    install -Dm644 "$root/resources/icons/dev.nalart.Nolurix-symbolic.svg" \
        "$prefix/share/icons/hicolor/symbolic/apps/dev.nalart.Nolurix-symbolic.svg"

    sed "s|@EXEC@|$libdir/nolurix|" "$root/resources/nolurix.desktop" \
        > "$prefix/share/applications/dev.nalart.Nolurix.desktop"
    chmod 644 "$prefix/share/applications/dev.nalart.Nolurix.desktop"

    mkdir -p "$prefix/bin"
    ln -sf "$libdir/nolurix" "$prefix/bin/nolurix"
    python3 scripts/uninstall-linux.py --record "$prefix"

    # Record the completed payload before authorization: a denied grant must leave
    # a recognized, retryable installation, not unowned files or a stale manifest.
    # Copying drops capabilities; grant only the installed core and fail visibly.
    sudo setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep "$libdir/nolurix-core"

    # Make the desktop entry and icon appear without logging into the session again.
    command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true
    command -v gtk4-update-icon-cache >/dev/null && gtk4-update-icon-cache -f -t "$prefix/share/icons/hicolor" 2>/dev/null || true

    echo "Установлено в $libdir
    Ярлык: $prefix/share/applications/dev.nalart.Nolurix.desktop
    Команда: nolurix (если $prefix/bin есть в PATH)

    Права для режима VPN выданы установленному ядру. Для повторной выдачи:
      sudo setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep $libdir/nolurix-core
    или из каталога репозитория:
      just grant-installed-tun

    Права живут на самом файле; just install повторно выдаёт их после копирования."
    echo "Legacy throne-gtk files/data are retained. Review old launchers/desktop entries before removing them manually."

# Explicitly grant TUN permissions to the installed core, not the build copy.
[group('installation')]
grant-installed-tun:
    sudo setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep "${PREFIX:-$HOME/.local}/lib/nolurix/nolurix-core"

# Remove the installation (profiles and settings remain)
[group('installation')]
uninstall:
    #!/usr/bin/env bash
    set -euo pipefail
    prefix="${PREFIX:-$HOME/.local}"
    python3 scripts/uninstall-linux.py "$prefix"
    command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true
    echo "Удалено. Профили и настройки остались в ~/.config/nolurix"
