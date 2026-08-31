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
    ./build/throne-gtk

# Build everything: core and interface
[group('build')]
build: build-core fetch-cronet build-app

# Build the core (Go, sing-box + Xray)
[group('build')]
build-core:
    cd core/gen && protoc -I . --go_out=. --go-grpc_out=. libcore.proto
    cd core && CGO_ENABLED=1 go build -o ../build/throne-gtk-core -trimpath \
        -ldflags "-w -s -X 'github.com/sagernet/sing-box/constant.Version=$(go list -m -f '{{{{.Version}}' github.com/sagernet/sing-box)' -checklinkname=0" \
        -tags "{{ core_tags }}"

# Build the interface (Rust, GTK4) [set RELEASE=1 to enable release build]
[group('build')]
build-app:
    cargo build {{ if RELEASE == "true" { "--release" } }} -p throne-app
    cp target/{{ if RELEASE == "true" { "release" } else { "debug" } }}/throne-gtk build/

# Place libcronet near core to enable Naive
[group('build')]
fetch-cronet:
    #!/usr/bin/env bash
    set -euo pipefail

    root="."
    module_dir="$(go env GOMODCACHE)/github.com/parhelia512/cronet-go/lib"

    library="$(find "$module_dir" -maxdepth 2 -name libcronet.so -path "*linux_amd64*" 2>/dev/null | head -1)"
    if [[ -z "$library" ]]; then
        echo "libcronet.so не найдена в кэше модулей — naive работать не будет" >&2
        exit 0
    fi

    install -Dm644 "$library" "$root/build/libcronet.so"
    echo "libcronet.so → build/"

# Tests
[group('test')]
test:
    cargo test --workspace

# End-to-end check with real data: import from Throne and validate configs with the core.
# The binary is copied under the name throne-gtk — the core does not accept another parent.
[group('test')]
selftest:
    cargo build -p throne-app --example selftest
    mkdir -p target/selftest
    cp target/debug/examples/selftest target/selftest/throne-gtk
    cp build/throne-gtk-core target/selftest/
    ./target/selftest/throne-gtk

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
