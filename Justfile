# Сборка и запуск Throne GTK.

core_tags := "with_clash_api,with_gvisor,with_quic,with_wireguard,with_utls,with_dhcp,with_tailscale,with_purego,with_naive_outbound,badlinkname,tfogo_checklinkname0"

# Собрать всё: ядро и интерфейс
build: core app

# Собрать ядро (Go, sing-box + Xray) и положить рядом библиотеку cronet для naive
core:
    cd core/gen && protoc -I . --go_out=. --go-grpc_out=. libcore.proto
    cd core && CGO_ENABLED=1 go build -o ../build/throne-gtk-core -trimpath \
        -ldflags "-w -s -X 'github.com/sagernet/sing-box/constant.Version=$(cd core && go list -m -f '{{{{.Version}}}}' github.com/sagernet/sing-box)' -checklinkname=0" \
        -tags "{{core_tags}}"
    ./scripts/fetch_cronet.sh

# Собрать интерфейс (Rust, GTK4)
app:
    cargo build --release -p throne-app
    cp target/release/throne-gtk build/

# Отладочная сборка интерфейса
app-debug:
    cargo build -p throne-app
    cp target/debug/throne-gtk build/

# Запустить
run: app-debug
    ./build/throne-gtk

# Тесты
test:
    cargo test --workspace

# Сквозная проверка на реальных данных: импорт из Throne и проверка конфигов ядром.
# Бинарь копируется под именем throne-gtk — ядро не принимает другого родителя.
selftest:
    cargo build -p throne-app --example selftest
    mkdir -p target/selftest
    cp target/debug/examples/selftest target/selftest/throne-gtk
    cp build/throne-gtk-core target/selftest/
    ./target/selftest/throne-gtk

# Выдать ядру права на TUN (нужно один раз для режима VPN)
grant-tun:
    sudo setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep build/throne-gtk-core

# Установить в ~/.local: приложение появится в списке программ
install: build
    ./scripts/install.sh

# Удалить установленное (профили и настройки остаются)
uninstall:
    ./scripts/uninstall.sh

clean:
    cargo clean
    rm -rf build target/selftest
