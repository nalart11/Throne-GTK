# Build and run Throne GTK.

core_tags := "with_clash_api,with_gvisor,with_quic,with_wireguard,with_utls,with_dhcp,with_tailscale,with_purego,with_naive_outbound,badlinkname,tfogo_checklinkname0"

# sing-box and sing-trusttunnel use //go:linkname to access internals of
# golang.org/x/net/http2; in Go 1.27 the required symbol is inlined and linking fails.
# Build the core with the 1.26 toolchain until the Throneproj/sing-box fork incorporates the fix.
export GOTOOLCHAIN := "go1.26.0"

# Build everything: core and interface
build: core app

# Build the core (Go, sing-box + Xray) and place the cronet library for naive next to it
core:
    cd core/gen && protoc -I . --go_out=. --go-grpc_out=. libcore.proto
    cd core && CGO_ENABLED=1 go build -o ../build/throne-gtk-core -trimpath \
        -ldflags "-w -s -X 'github.com/sagernet/sing-box/constant.Version=$(go list -m -f '{{{{.Version}}' github.com/sagernet/sing-box)' -checklinkname=0" \
        -tags "{{core_tags}}"
    ./scripts/fetch_cronet.sh

# Build the interface (Rust, GTK4)
app:
    cargo build --release -p throne-app
    cp target/release/throne-gtk build/

# Debug build of the interface
app-debug:
    cargo build -p throne-app
    cp target/debug/throne-gtk build/

# Run
run: app-debug
    ./build/throne-gtk

# Tests
test:
    cargo test --workspace

# End-to-end check with real data: import from Throne and validate configs with the core.
# The binary is copied under the name throne-gtk — the core does not accept another parent.
selftest:
    cargo build -p throne-app --example selftest
    mkdir -p target/selftest
    cp target/debug/examples/selftest target/selftest/throne-gtk
    cp build/throne-gtk-core target/selftest/
    ./target/selftest/throne-gtk

# Grant the core TUN permissions (repeat after each reinstallation: the file is recreated)
grant-tun:
    sudo setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep build/throne-gtk-core
    # The application runs the installed copy, not the one in build/: it needs the permissions too.
    -sudo setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep "{{env_var_or_default('PREFIX', env_var('HOME') + '/.local')}}/lib/throne-gtk/throne-gtk-core"

# Install to ~/.local: the application will appear in the application list
install: build
    ./scripts/install.sh

# Remove the installation (profiles and settings remain)
uninstall:
    ./scripts/uninstall.sh

clean:
    cargo clean
    rm -rf build target/selftest
