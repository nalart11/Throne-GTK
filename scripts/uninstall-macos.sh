#!/bin/bash
# Run from the protected installation after unregistering in the user's GUI.
set -eu
fail() { echo "Throne GTK: $*" >&2; exit 1; }
[ "$(id -u)" = 0 ] || fail 'Run with administrator authorization after unregistering SMAppService as the logged-in user.'
base='/Library/Application Support/Throne GTK'
app="$base/Throne GTK.app"
link='/Applications/Throne GTK.app'
label=dev.nalart.ThroneGtk.helper
plist="/Library/LaunchDaemons/$label.plist"
acl_check="$base/check-install-acl"
# Bootstrap the installed checker using system tools only. A conservative
# read/deny ACL subset suffices here; the native checker handles the full tree.
for node in / /Library '/Library/Application Support' "$base" "$acl_check"; do
    [ ! -L "$node" ] || fail "Unsafe checker ancestor: $node"
    metadata="$(/usr/bin/stat -f '%u %Lp' "$node")" || fail 'Cannot read checker metadata'
    read -r owner mode <<< "$metadata"
    [ "$owner" = 0 ] && (( (8#$mode & 8#22) == 0 )) || fail 'Checker is not root-protected'
    acl_text="$(/bin/ls -lde "$node")" || fail 'Cannot read checker ACL'
    first=1
    while IFS= read -r entry; do
        if [ "$first" = 1 ]; then first=0; continue; fi
        if [[ "$entry" =~ ^[[:space:]]*[0-9]+:[[:space:]]+[^[:space:]]+[[:space:]]+(inherited[[:space:]]+)?deny[[:space:]]+[a-z_,]+$ ]]; then
            continue
        fi
        [[ "$entry" =~ ^[[:space:]]*[0-9]+:[[:space:]]+[^[:space:]]+[[:space:]]+(inherited[[:space:]]+)?allow[[:space:]]+([a-z_,]+)$ ]] || fail 'Unrecognized checker ACL entry'
        IFS=, read -r -a rights <<< "${BASH_REMATCH[2]}"
        for right in "${rights[@]}"; do
            case "$right" in
                read|list|execute|search|readattr|readextattr|readsecurity|synchronize|file_inherit|directory_inherit|limit_inherit|only_inherit) ;;
                *) fail 'Checker bootstrap rejects mutation or unknown ACL grants' ;;
            esac
        done
    done <<< "$acl_text"
done
/usr/bin/codesign --verify --strict "$acl_check" || fail 'Invalid ACL checker signature'
"$acl_check" --ancestors / /Library '/Library/Application Support' /Library/LaunchDaemons || fail 'Unsafe shared ancestor'
"$acl_check" --tree "$base" || fail 'Unsafe private ownership, modes or ACLs'
for dir in /Library '/Library/Application Support' "$base" "$app" /Applications /Library/LaunchDaemons; do
    [ -d "$dir" ] && [ ! -L "$dir" ] || fail "Unsafe directory: $dir"
done
[ -f "$base/.throne-gtk-package" ] && [ ! -L "$base/.throne-gtk-package" ] || fail 'Missing ownership marker'
[ "$(/usr/bin/stat -f %u "$base")" = 0 ] || fail 'Not root-owned'
[ -z "$(/usr/bin/find "$base" ! -type l ! \( -path "$base/run/helper.sock" -type s -user root \) \( ! -user root -o -perm -002 -o -perm -020 \) -print)" ] || fail 'Installation is not root-protected'
/usr/bin/codesign --verify --deep --strict "$app" || fail 'Invalid app signature'
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")" = dev.nalart.ThroneGtk ] || fail 'Foreign bundle'
if [ ! -f "$base/.throne-gtk-manual" ]; then
    [ "${1:-}" = --sm-unregistered ] || fail 'As the logged-in user (NOT root), run the installed app executable with --unregister-helper first; then confirm here with --sm-unregistered. Root cannot unregister on behalf of that user.'
fi
if [ -e "$plist" ] || [ -L "$plist" ]; then
    "$acl_check" --nodes "$plist" || fail 'Unsafe service plist ownership, modes or ACLs'
    [ -f "$base/.throne-gtk-manual" ] && [ ! -L "$plist" ] || fail 'Foreign or SM-managed service; unregister in user context first'
    /usr/bin/cmp -s "$plist" "$base/manual-helper.plist" || fail 'Modified service plist; refusing removal'
    if /bin/launchctl print "system/$label" >/dev/null 2>&1; then
        /bin/launchctl bootout "system/$label" || fail 'Could not stop helper; no executable or plist removed'
        if /bin/launchctl print "system/$label" >/dev/null 2>&1; then
            fail 'Helper remains loaded; no executable or plist removed'
        fi
    fi
elif /bin/launchctl print "system/$label" >/dev/null 2>&1; then
    fail 'SMAppService is still registered. Unregister through the app as the logged-in user, not as root; bootout is not unregister.'
fi
"$app/Contents/MacOS/throne-gtk-core" --recover-dns || fail 'DNS recovery failed or is busy/conflicted; all installed artifacts preserved'
# Only after the service is stopped and recovery succeeds may its permanent
# singleton inode be removed. Preserve unknown runtime files.
/bin/rm -f "$base/run/helper.sock" "$base/run/helper.lock"
/bin/rmdir "$base/run" 2>/dev/null || true
if [ -e "$plist" ]; then
    /bin/rm "$plist"
fi
if [ -L "$link" ] && [ "$(/usr/bin/readlink "$link")" = "$app" ]; then
    /bin/rm "$link"
fi
# Preserve all profiles, preferences, logs and shared directories. Deliberately
# leave unknown files in the private directory rather than recursively deleting it.
/bin/rm -rf "$app"
/bin/rm -f "$base/.throne-gtk-package" "$base/.throne-gtk-manual" "$base/manual-helper.plist" "$base/check-install-acl" "$base/uninstall.sh"
/bin/rmdir "$base" 2>/dev/null || true
