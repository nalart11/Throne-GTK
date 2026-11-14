#!/usr/bin/env bash
# Install released artifacts only. Compatible with Bash 3.2.
set -euo pipefail
die() { printf 'Error: %s\n' "$*" >&2; exit 1; }
usage() {
    cat <<'EOF'
Usage: install.sh [--version [v]X.Y.Z] [--prefix /absolute/path] [--yes]
                  [--allow-unsigned-macos] [--help]
Linux x86_64: user install (default ~/.local); requires Python 3, glibc >=2.39,
GTK >=4.12 and libadwaita >=1.5 runtime libraries. No compilation or capabilities.
macOS 15+ arm64 baseline: TESTING unsigned/unnotarized PKG, privileged VPN helper.
Released v0.2.1 requires macOS 15+; newer packages may require a newer system.
Explicit --version 0.2.1 may use checksum-verified historical Throne GTK assets
only after the Nolurix asset returns HTTP 404; historical identities are retained.
--yes skips ordinary confirmation, NOT the mandatory macOS testing acknowledgement.
Checksums provide download integrity, not a publisher signature.
EOF
}
version= prefix="${HOME:?HOME is required}/.local" yes=no allow=no prefix_set=no pinned=no
while [ "$#" -gt 0 ]; do
    case "$1" in
        --help) usage; exit 0 ;;
        --version|--prefix)
            [ "$#" -ge 2 ] || die "$1 needs a value"
            if [ "$1" = --version ]; then
                version=${2#v}
                pinned=yes
                [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die 'Invalid release version'
            else prefix=$2; prefix_set=yes; fi
            shift 2 ;;
        --yes) yes=yes; shift ;;
        --allow-unsigned-macos) allow=yes; shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done
[ "$EUID" -ne 0 ] || die 'Run as your normal user, not root.'
if [ -n "$version" ]; then [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die 'Invalid release version'; fi
os=$(uname -s); arch=$(uname -m)
case "$os/$arch" in
    Linux/x86_64) command -v python3 >/dev/null || die 'Python 3 is required for safe extraction' ;;
    Darwin/arm64)
        [ "$prefix_set" = no ] || die '--prefix is Linux-only'
        mac_version=$(sw_vers -productVersion) || die 'Cannot determine macOS version'
        [[ "$mac_version" =~ ^([0-9]+)\.([0-9]+)(\.[0-9]+)?$ ]] || die 'Unexpected macOS version'
        [ "${BASH_REMATCH[1]}" -ge 15 ] || die 'Requires the macOS 15+ baseline'
        printf '%s\n' 'WARNING: TESTING unsigned/unnotarized macOS package. Installs a system-wide privileged VPN helper and requires administrator authorization. Released v0.2.1 requires macOS 15+; newer package requirements may be higher and still apply.'
        [ "$allow" = yes ] || die 'Explicit --allow-unsigned-macos acknowledgement is required, even with --yes.' ;;
    *) die "Unsupported platform: $os/$arch" ;;
esac
command -v curl >/dev/null || die 'curl is required'
if command -v sha256sum >/dev/null; then hash=sha256sum
elif command -v shasum >/dev/null; then hash=shasum
else die 'sha256sum or shasum is required'; fi
curl_https() { curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' "$@"; }
if [ -z "$version" ]; then
    latest=$(curl_https --output /dev/null --write-out '%{url_effective}' https://github.com/nalart11/Nolurix/releases/latest) || die 'Latest release lookup failed'
    [[ "$latest" =~ ^https://github\.com/nalart11/Nolurix/releases/tag/v([0-9]+\.[0-9]+\.[0-9]+)$ ]] || die 'Unexpected latest release URL'
    version=${BASH_REMATCH[1]}
fi
if [ "$os" = Linux ]; then
    case "$prefix" in /*) ;; *) die '--prefix must be absolute';; esac
    python3 - "$(getconf GNU_LIBC_VERSION)" <<'PY'
import sys, re, ctypes, ctypes.util
m = re.fullmatch(r'glibc (\d+)\.(\d+)', sys.argv[1])
if not m or tuple(map(int,m.groups())) < (2,39):
    sys.exit('Requires glibc >=2.39')
for lib, stem, required in [('gtk-4','gtk',(4,12)), ('adwaita-1','adw',(1,5))]:
    try:
        obj = ctypes.CDLL(ctypes.util.find_library(lib) or ('lib'+lib+'.so.1'))
        actual = tuple(getattr(obj, stem+'_get_'+part+'_version')() for part in ('major','minor'))
        if actual < required: raise ValueError('runtime too old')
    except Exception as e:
        sys.exit('Requires %s runtime >=%s.%s (%s)' % (lib,*required,e))
PY
    asset="nolurix-${version}-linux-x86_64.tar.gz"
else asset="nolurix-${version}-macos-arm64-testing-local.pkg"; fi
printf 'Install %s? Checksums are integrity checks, not publisher signatures.\n' "$asset"
if [ "$yes" != yes ]; then
    printf 'Continue [y/N]? '; read -r answer || die 'Confirmation required'
    case "$answer" in y|Y|yes) ;; *) die 'Cancelled';; esac
fi
work=$(mktemp -d "${TMPDIR:-/tmp}/nolurix-install.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
base="https://github.com/nalart11/Nolurix/releases/download/v${version}"
curl_https --max-filesize 1048576 --output "$work/SHA256SUMS" "$base/SHA256SUMS"
legacy=no
# A transport/TLS/server failure is never grounds for selecting an older name.
# Only an explicit 404 for the preferred asset permits the pinned 0.2.1 alias.
status=$(curl_https --max-filesize 1073741824 --write-out '%{http_code}' --output "$work/$asset" "$base/$asset") && download_ok=yes || download_ok=no
if [ "$download_ok" != yes ]; then
    if [ "$status" = 404 ] && [ "$pinned" = yes ] && [ "$version" = 0.2.1 ]; then
        legacy=yes
        asset="throne-gtk-${asset#nolurix-}"
        curl_https --max-filesize 1073741824 --output "$work/$asset" "$base/$asset"
    else die 'Release asset download failed; refusing legacy fallback'; fi
fi
# Count every entry naming this file, not just valid entries (duplicates fail closed).
expected= count=0
while IFS= read -r line || [ -n "$line" ]; do
    if [[ "$line" =~ ^([^[:space:]]+)[[:space:]]+\*?(.+)$ ]] && [ "${BASH_REMATCH[2]}" = "$asset" ]; then
        count=$((count + 1)); expected=${BASH_REMATCH[1]}
    fi
done < "$work/SHA256SUMS"
[ "$count" -eq 1 ] && [[ "$expected" =~ ^[a-fA-F0-9]{64}$ ]] || die 'Missing, duplicate or invalid checksum entry'
if [ "$hash" = sha256sum ]; then actual=$(sha256sum "$work/$asset"); else actual=$(shasum -a 256 "$work/$asset"); fi
actual=${actual%% *}
[ "$(printf '%s' "$actual" | tr A-F a-f)" = "$(printf '%s' "$expected" | tr A-F a-f)" ] || die 'Checksum mismatch'
if [ "$legacy" = yes ]; then
    printf '%s\n' 'WARNING: Installing historical Throne GTK v0.2.1, NOT branded Nolurix. Historical executable, desktop, configuration and core identities are preserved.'
fi
if [ "$os" = Darwin ]; then
    # The only elevation in this script; explicitly acknowledged above.
    sudo /usr/sbin/installer -pkg "$work/$asset" -target /
    exit
fi
python3 - "$work/$asset" "$version" "$prefix" "$legacy" <<'PY'
import os, sys, tarfile, tempfile, shutil, stat, hashlib, json
from pathlib import Path
archive, version, rawprefix, legacy = sys.argv[1:]
prefix = Path(os.path.abspath(rawprefix))
if any(c in rawprefix for c in '\n\r\x00'): sys.exit('Invalid prefix')
binary = 'throne-gtk' if legacy == 'yes' else 'nolurix'
app_id = 'dev.nalart.ThroneGtk' if legacy == 'yes' else 'dev.nalart.Nolurix'
executable = f'lib/{binary}/{binary}'
desktop_path = f'share/applications/{app_id}.desktop'
launcher_path = f'bin/{binary}'
root = binary+'-'+version+'-linux-x86_64'
files = [executable, executable+'-core', f'lib/{binary}/libcronet.so',
         f'share/icons/hicolor/scalable/apps/{app_id}.svg',
         f'share/icons/hicolor/symbolic/apps/{app_id}-symbolic.svg',
         desktop_path+'.in']
dirs = {root}
for f in files:
    p = Path(root+'/'+f).parent
    while str(p) != '.': dirs.add(str(p)); p=p.parent
expected = {root+'/'+f for f in files}
def safe(path, directory=False):
    for p in [path]+list(path.parents):
        if p.is_symlink():
            if (p == path == prefix/launcher_path and
                    stat.S_ISLNK(p.lstat().st_mode) and
                    os.readlink(p) == str(prefix/executable)):
                continue  # Known legacy launcher only; replace/backup the link itself.
            raise ValueError('Symlink destination: '+str(p))
        if p.exists() and (not p.is_dir() if p != path or directory else not p.is_file()):
            raise ValueError('Unsafe destination: '+str(p))
safe(prefix, True)
with tarfile.open(archive, 'r:gz') as tar:
    members=[]; seen=set(); total=0
    for m in tar:
        name=m.name.rstrip('/') if m.isdir() else m.name
        if name in seen: raise ValueError('Duplicate archive member')
        seen.add(name)
        if len(seen)>64: raise ValueError('Too many members')
        if m.isdir() and name in dirs: pass
        elif m.isfile() and name in expected:
            if m.size>512*1024*1024: raise ValueError('Member too large')
            total+=m.size
            if total>1024*1024*1024: raise ValueError('Archive too large')
        else: raise ValueError('Unexpected or unsafe archive member: '+m.name)
        members.append(m)
    if not expected.issubset(seen): raise ValueError('Incomplete archive')
    # Validate entire archive before writing any payload or destination.
    with tempfile.TemporaryDirectory(prefix='nolurix-stage-') as stage:
        stage=Path(stage)
        for m in members:
            if m.isfile():
                dest=stage/m.name[len(root)+1:]; dest.parent.mkdir(parents=True,exist_ok=True)
                with tar.extractfile(m) as src, dest.open('wb') as out: shutil.copyfileobj(src,out)
                dest.chmod(0o755 if dest.name in (binary,binary+'-core') else 0o644)
        # Desktop Exec has two escaping layers; percent is a field-code escape.
        exe=str(prefix/executable).replace('%','%%')
        exe=''.join('\\'+c if c in '\\"`$' else c for c in exe)
        exe=('"'+exe+'"').replace('\\','\\\\')
        template=stage/files[-1]
        text=template.read_text()
        if text.count('@EXEC@') != 1: raise ValueError('Invalid desktop template')
        desktop=stage/desktop_path
        desktop.write_text(text.replace('@EXEC@',exe)); desktop.chmod(0o644)
        bindir=stage/'bin'; bindir.mkdir()
        # Shell single quoting is independent from desktop-file escaping.
        quoted="'"+str(prefix/executable).replace("'", "'\\''")+"'"
        launcher=bindir/binary; launcher.write_text('#!/bin/sh\nexec '+quoted+' "$@"\n'); launcher.chmod(0o755)
        targets=files[:-1]+[desktop_path,launcher_path]
        manifest=f'lib/{binary}/.installed-files.json'
        (stage/manifest).write_text(json.dumps({
            f: {'sha256': hashlib.sha256((stage/f).read_bytes()).hexdigest()}
            for f in targets
        }))
        targets.append(manifest)
        for f in targets: safe(prefix/f)
        installed_manifest=prefix/manifest
        owned=json.loads(installed_manifest.read_text()) if installed_manifest.is_file() else {}
        for f in targets[:-1]:
            dest=prefix/f
            if not os.path.lexists(dest): continue
            if dest.is_symlink() and f == launcher_path:
                continue  # safe() already verified the exact legacy link target.
            identity={'sha256': hashlib.sha256(dest.read_bytes()).hexdigest()}
            if owned.get(f) != identity:
                raise ValueError('Refusing unrecognized or modified destination: '+str(dest))
        prefix.mkdir(parents=True,exist_ok=True)
        # Same-filesystem staging enables atomic per-file replacement and rollback.
        with tempfile.TemporaryDirectory(prefix='.nolurix-install-',dir=str(prefix)) as txn:
            txn=Path(txn); done=[]
            try:
                for i,f in enumerate(targets):
                    dest=prefix/f; safe(dest); dest.parent.mkdir(parents=True,exist_ok=True)
                    new=txn/(str(i)+'.new'); shutil.copy2(stage/f,new)
                    backup=txn/(str(i)+'.old')
                    existed=os.path.lexists(dest)
                    if existed: os.replace(dest,backup)
                    done.append((dest,backup,existed))
                    os.replace(new,dest)
            except BaseException:
                for dest,backup,existed in reversed(done):
                    if existed: os.replace(backup,dest)
                    elif dest.exists(): dest.unlink()
                raise
PY
installed_binary=nolurix
[ "$legacy" != yes ] || installed_binary=throne-gtk
printf '%s\n' "Installed into $prefix. Run $prefix/bin/$installed_binary (not launched automatically)."
printf '%s\n' 'Previous throne-gtk installations and user data are retained, never deleted automatically.' \
    'To retire old integration, inspect ~/.local/bin/throne-gtk and ~/.local/share/applications/dev.nalart.ThroneGtk.desktop (or your previous prefix) and remove only entries you recognize as belonging to that installation. Do not delete configuration/data directories.'
printf '%s\n' 'VPN/TUN is optional and needs additional privileges. No capabilities were granted.' 'If you choose to grant CAP_NET_ADMIN to the installed core, review the security implications and use your administrator tools explicitly. File capabilities are lost on upgrades.'
