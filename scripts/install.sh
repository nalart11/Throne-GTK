#!/usr/bin/env bash
# Установка Throne GTK в домашний каталог: приложение появляется в списке
# программ рабочего стола. Права root не нужны.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
prefix="${PREFIX:-$HOME/.local}"
libdir="$prefix/lib/throne-gtk"

for binary in throne-gtk throne-gtk-core; do
    if [[ ! -x "$root/build/$binary" ]]; then
        echo "Нет $root/build/$binary — сначала соберите проект (just build)" >&2
        exit 1
    fi
done

# Интерфейс и ядро обязаны лежать в одной папке: ядро проверяет, что его
# родитель называется throne-gtk и находится рядом.
install -Dm755 "$root/build/throne-gtk"      "$libdir/throne-gtk"
install -Dm755 "$root/build/throne-gtk-core" "$libdir/throne-gtk-core"

# Библиотека cronet нужна выхлопу naive; без неё остальное работает как прежде.
if [[ -f "$root/build/libcronet.so" ]]; then
    install -Dm644 "$root/build/libcronet.so" "$libdir/libcronet.so"
fi

install -Dm644 "$root/resources/icons/dev.nalart.ThroneGtk.svg" \
    "$prefix/share/icons/hicolor/scalable/apps/dev.nalart.ThroneGtk.svg"
install -Dm644 "$root/resources/icons/dev.nalart.ThroneGtk-symbolic.svg" \
    "$prefix/share/icons/hicolor/symbolic/apps/dev.nalart.ThroneGtk-symbolic.svg"

# Exec с полным путём: ~/.local/bin есть в PATH не у всех оболочек, а ярлык
# должен работать независимо от этого.
sed "s|@EXEC@|$libdir/throne-gtk|" "$root/resources/throne-gtk.desktop" \
    > "$prefix/share/applications/dev.nalart.ThroneGtk.desktop"
chmod 644 "$prefix/share/applications/dev.nalart.ThroneGtk.desktop"

mkdir -p "$prefix/bin"
ln -sf "$libdir/throne-gtk" "$prefix/bin/throne-gtk"

# Чтобы ярлык и иконка появились без перезахода в сеанс.
command -v update-desktop-database >/dev/null && \
    update-desktop-database "$prefix/share/applications" || true
command -v gtk4-update-icon-cache >/dev/null && \
    gtk4-update-icon-cache -f -t "$prefix/share/icons/hicolor" 2>/dev/null || true

echo "Установлено в $libdir"
echo "Ярлык: $prefix/share/applications/dev.nalart.ThroneGtk.desktop"
echo "Команда: throne-gtk (если $prefix/bin есть в PATH)"
echo
echo "Для режима VPN выдайте ядру права:"
echo "  sudo setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep $libdir/throne-gtk-core"
echo
echo "Права живут на самом файле, а установка создаёт его заново — повторяйте"
echo "эту команду после каждой переустановки."
