# Build and run Throne GTK.
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
    "./build/throne-gtk${suffix}"

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
        go build -o "../build/throne-gtk-core${suffix}" -trimpath \
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
    ./scripts/package-macos-installer.sh

# Build the interface (Rust, GTK4) [set RELEASE=1 to enable release build]
[group('build')]
build-app:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p build
    suffix=""
    [[ "$(go env GOOS)" == "windows" ]] && suffix=".exe"
    cargo build {{ if RELEASE == "true" { "--release" } }} -p throne-app
    cp "target/{{ if RELEASE == "true" { "release" } else { "debug" } }}/throne-gtk${suffix}" build/

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
    cp "$library" "build/${library_name}"
    echo "${library_name} → build/"

# Tests
[group('test')]
test:
    cargo test --workspace

# End-to-end check with real data: import from Throne and validate configs with the core.
# The binary is copied under the name throne-gtk — the core does not accept another parent.
[group('test')]
selftest:
    #!/usr/bin/env bash
    set -euo pipefail
    suffix=""
    [[ "$(go env GOOS)" == "windows" ]] && suffix=".exe"
    cargo build -p throne-app --example selftest
    mkdir -p target/selftest
    cp "target/debug/examples/selftest${suffix}" "target/selftest/throne-gtk${suffix}"
    cp "build/throne-gtk-core${suffix}" target/selftest/
    "./target/selftest/throne-gtk${suffix}"

# Clean build data
clean:
    cargo clean
    rm -rf build target/selftest

# Grant the core TUN permissions (repeat after each reinstallation: the file is recreated)
[group('installation')]
grant-tun:
    sudo setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep build/throne-gtk-core

# Install to ~/.local: the application will appear in the application list
[group('installation')]
install: build grant-tun
    #!/usr/bin/env bash
    set -euo pipefail
    root="."
    prefix="${PREFIX:-$HOME/.local}"
    libdir="$prefix/lib/throne-gtk"

    for binary in throne-gtk throne-gtk-core; do
        if [[ ! -x "$root/build/$binary" ]]; then
            echo "Нет $root/build/$binary — сначала соберите проект (just build)" >&2
            exit 1
        fi
    done

    install -Dm755 "$root/build/throne-gtk"      "$libdir/throne-gtk"
    install -Dm755 "$root/build/throne-gtk-core" "$libdir/throne-gtk-core"
    if [[ -f "$root/build/libcronet.so" ]]; then
        install -Dm644 "$root/build/libcronet.so" "$libdir/libcronet.so"
    fi

    install -Dm644 "$root/resources/icons/dev.nalart.ThroneGtk.svg" \
        "$prefix/share/icons/hicolor/scalable/apps/dev.nalart.ThroneGtk.svg"
    install -Dm644 "$root/resources/icons/dev.nalart.ThroneGtk-symbolic.svg" \
        "$prefix/share/icons/hicolor/symbolic/apps/dev.nalart.ThroneGtk-symbolic.svg"

    sed "s|@EXEC@|$libdir/throne-gtk|" "$root/resources/throne-gtk.desktop" \
        > "$prefix/share/applications/dev.nalart.ThroneGtk.desktop"
    chmod 644 "$prefix/share/applications/dev.nalart.ThroneGtk.desktop"

    mkdir -p "$prefix/bin"
    ln -sf "$libdir/throne-gtk" "$prefix/bin/throne-gtk"

    # Make the desktop entry and icon appear without logging into the session again.
    command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true
    command -v gtk4-update-icon-cache >/dev/null && gtk4-update-icon-cache -f -t "$prefix/share/icons/hicolor" 2>/dev/null || true

    echo "Установлено в $libdir
    Ярлык: $prefix/share/applications/dev.nalart.ThroneGtk.desktop
    Команда: throne-gtk (если $prefix/bin есть в PATH)

    Для режима VPN выдайте ядру права:
      sudo setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep $libdir/throne-gtk-core
    или из каталога репозитория:
      just grant-tun

    Права живут на самом файле, а установка создаёт его заново — повторяйте
    эту команду после каждой переустановки."

# Remove the installation (profiles and settings remain)
[group('installation')]
uninstall:
    #!/usr/bin/env bash
    set -euo pipefail
    prefix="${PREFIX:-$HOME/.local}"
    rm -rf "$prefix/lib/throne-gtk"
    rm -f "$prefix/bin/throne-gtk"
    rm -f "$prefix/share/applications/dev.nalart.ThroneGtk.desktop"
    rm -f "$prefix/share/icons/hicolor/scalable/apps/dev.nalart.ThroneGtk.svg"
    rm -f "$prefix/share/icons/hicolor/symbolic/apps/dev.nalart.ThroneGtk-symbolic.svg"
    command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true
    echo "Удалено. Профили и настройки остались в ~/.config/throne-gtk"
