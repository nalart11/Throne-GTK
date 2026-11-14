#!/usr/bin/env bash
# Portable, non-installing regression checks; no sudo or macOS tools required.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
python3 - "$root" <<'PY'
import os, pathlib, plistlib, re, shutil, socket, subprocess, sys, tempfile
def bind_fixture_socket(control, path):
    # macOS temporary-directory prefixes can exceed sun_path even for a short
    # fixture leaf. Bind relative to its private parent, restoring cwd on error.
    assert len(os.fsencode(path.name)) <= 103
    previous = os.open('.', os.O_RDONLY)
    try:
        os.chdir(path.parent)
        control.bind(path.name)
    finally:
        try:
            os.fchdir(previous)
        finally:
            os.close(previous)

r = pathlib.Path(sys.argv[1])
fixed = '/Library/Application Support/Throne GTK/Throne GTK.app'
with (r/'resources/macos/dev.nalart.ThroneGtk.manual-helper.plist').open('rb') as f:
    assert plistlib.load(f)['ProgramArguments'][0] == fixed+'/Contents/MacOS/throne-gtk-core'
with (r/'resources/macos/dev.nalart.ThroneGtk.helper.plist').open('rb') as f:
    p = plistlib.load(f)
    assert p['BundleProgram'] == 'Contents/MacOS/throne-gtk-core'
with (r/'resources/macos/installer/components.plist').open('rb') as f:
    assert plistlib.load(f)[0]['RootRelativeBundlePath'] == fixed.lstrip('/')
pre = (r/'resources/macos/installer/preinstall').read_text()
post = (r/'resources/macos/installer/postinstall').read_text()
go = (r/'core/helper_darwin.go').read_text()
rust = (r/'crates/ipc/src/service_macos.rs').read_text()
runtime = '/Library/Application Support/Throne GTK/run/'
assert re.search(r'const helperSocket = "([^"]+)"', go)[1] == runtime+'helper.sock'
assert re.search(r'const HELPER_SOCKET: &str = "([^"]+)"', rust)[1] == runtime+'helper.sock'
assert 'const path = "'+runtime+'helper.lock"' in go
assert 'run="$base/run"' in post and '/bin/mkdir "$run"' in post
assert '/bin/rm' not in post and 'chmod 755 "$run"' in post
assert all(len((runtime+leaf).encode()) + 1 <= 104 for leaf in ('helper.sock', 'helper.lock'))
assert rust.index('validate_helper_parent()?;') < rust.index('UnixStream::connect(HELPER_SOCKET)')
assert 'Path::new(HELPER_SOCKET).parent()' in rust
pkg = (r/'scripts/package-macos-installer.sh').read_text()
assert 'Foreign install directory' in pre and 'Legacy or foreign /Applications' in pre
assert 'bootout is NOT unregister' in pre
assert '[ ! -e "$link" ] && [ ! -L "$link" ]' in post
assert 'ditto --noextattr --noacl "$app" "$base/Throne GTK.app"' in pkg
assert '--dev-manual' in pkg and '--release' in pkg
# Execute the production optional-argument block, not a reimplementation. This
# runs under system Bash 3.2 on macOS CI as well as the host Bash on Linux.
argument_block = pkg[pkg.index('set --\n'):pkg.index('if [[ "$mode" = release ]]; then\n    xcrun notarytool')]
with tempfile.TemporaryDirectory() as tmp:
    fixture = pathlib.Path(tmp)
    script = fixture/'pkgbuild-arguments.sh'
    script.write_text('''set -euo pipefail
mode="$1"
INSTALLER_SIGN_IDENTITY='Developer ID Installer: Fixture (TEST ID)'
payload='fixture payload'
pkg_scripts='fixture scripts'
components='fixture components'
version=0.2.0
work='fixture work'
pkgbuild() { printf '%s\\0' "$@"; }
''' + argument_block)
    common = ['--root', 'fixture payload', '--scripts', 'fixture scripts',
              '--component-plist', 'fixture components', '--ownership', 'recommended',
              '--identifier', 'dev.nalart.ThroneGtk.local-installer', '--version', '0.2.0',
              '--install-location', '/', 'fixture work/throne-gtk-local-installer.pkg']
    for mode, signing in [('manual', []), ('release', ['--sign', 'Developer ID Installer: Fixture (TEST ID)', '--timestamp'])]:
        result = subprocess.run(['bash', str(script), mode], check=True, capture_output=True)
        argv = result.stdout.decode().split('\0')
        assert argv.pop() == ''  # final NUL, not an argv element
        assert argv == signing + common, (mode, argv)
        assert '' not in argv
# Missing any signing credential must fail before platform checks or packaging.
credentials = {'CODESIGN_IDENTITY': 'Developer ID Application: Fixture',
               'INSTALLER_SIGN_IDENTITY': 'Developer ID Installer: Fixture',
               'NOTARY_PROFILE': 'fixture-profile'}
for missing in credentials:
    env = dict(os.environ)
    for key in (*credentials, 'THRONE_CODESIGN_IDENTITY'):
        env.pop(key, None)
    env.update({key: value for key, value in credentials.items() if key != missing})
    result = subprocess.run(['bash', str(r/'scripts/package-macos-installer.sh'), '--release'],
                            env=env, capture_output=True, text=True)
    assert result.returncode != 0
    assert 'Release requires CODESIGN_IDENTITY, INSTALLER_SIGN_IDENTITY and NOTARY_PROFILE' in result.stderr
assert 'rm' not in pre.splitlines()[0] # preflight never deletes anything
assert '/bin/rm' not in pre
assert pre.index('Modified launchd plist') < pre.index('/bin/launchctl bootout')
assert '/bin/launchctl bootout' not in post
uninstall = (r/'scripts/uninstall-macos.sh').read_text()
assert uninstall.index('/bin/launchctl bootout') < uninstall.index('/bin/rm "$plist"')
assert uninstall.index('"$acl_check" --tree') < uninstall.index('"$app/Contents/MacOS/throne-gtk-core" --recover-dns') < uninstall.index('/bin/rm "$plist"')
assert uninstall.index('--recover-dns') < uninstall.index('/bin/rm -f "$base/run/helper.sock" "$base/run/helper.lock"')
assert pre.index('/bin/launchctl bootout') < pre.index('"$app/Contents/MacOS/throne-gtk-core" --recover-dns')
for source in (pre, uninstall):
    assert 'bootout "system/$label" || fail' in source
    assert 'bootout "system/$label" 2>/dev/null || true' not in source
assert pre.index('"$acl_check" --ancestors') < pre.index('/bin/launchctl bootout')
assert pre.index('"$acl_check" --tree "$base"') < pre.index('/bin/launchctl bootout')
assert '--ancestors / /Library' in pre and '/Applications || fail' not in pre
assert '"$acl_check" --nodes "$plist"' in pre
assert '/bin/chmod -N "$base"' in pre
for resource in ('dev.nalart.ThroneGtk.helper.plist', 'dev.nalart.ThroneGtk.manual-helper.plist'):
    with (r/'resources/macos'/resource).open('rb') as f:
        assert not plistlib.load(f).get('AbandonProcessGroup', False)
with tempfile.TemporaryDirectory() as tmp:
    app = pathlib.Path(tmp)/'Test.app'
    (app/'Contents/MacOS').mkdir(parents=True)
    (app/'Contents/Frameworks').mkdir()
    with (app/'Contents/Info.plist').open('wb') as f:
        plistlib.dump({'CFBundleIdentifier':'dev.nalart.ThroneGtk'}, f)
    for name in ('throne-gtk', 'throne-gtk-core'):
        (app/'Contents/MacOS'/name).write_text('fixture')
    validator = [sys.executable, str(r/'scripts/validate-macos-bundle.py'), str(app)]
    subprocess.run(validator, check=True)
    (app/'Contents/Frameworks/real.dylib').write_text('fixture')
    (app/'Contents/Frameworks/contained.dylib').symlink_to('real.dylib')
    subprocess.run(validator, check=True)
    tools = pathlib.Path(tmp)/'tools'
    tools.mkdir()
    for name, body in {
        'file': '#!/bin/sh\necho Mach-O\n',
        'otool': '#!/bin/sh\nprintf "cmd LC_LOAD_DYLIB\\n name @executable_path/../Frameworks/real.dylib (offset 24)\\n"\n',
    }.items():
        tool = tools/name
        tool.write_text(body)
        tool.chmod(0o755)
    env = dict(os.environ, PATH=str(tools)+os.pathsep+os.environ['PATH'])
    subprocess.run(validator, check=True, env=env)
    def commands(text, valid):
        (tools/'otool').write_text('#!/bin/sh\n' + "printf '%s\\n' '" + text + "'\n")
        result = subprocess.run(validator, env=env, stderr=subprocess.DEVNULL)
        assert (result.returncode == 0) == valid, text
    commands('cmd LC_LOAD_DYLIB\n name @executable_path/../Frameworks/absent.dylib (offset 24)', False)
    commands('cmd LC_RPATH\n path /opt/homebrew/lib (offset 12)', False)
    commands('cmd LC_RPATH\n path @loader_path/../../../../outside (offset 12)', False)
    commands('cmd LC_ID_DYLIB\n name @rpath/identity-only.dylib (offset 24)', True)
    commands('cmd LC_RPATH\n path @executable_path/../Frameworks (offset 12)\ncmd LC_LOAD_DYLIB\n name @rpath/real.dylib (offset 24)', True)
    commands('cmd LC_LOAD_DYLIB\n name @rpath/real.dylib (offset 24)', False)
    (app/'Contents/Frameworks/escape.dylib').symlink_to('/etc/passwd')
    assert subprocess.run(validator, stderr=subprocess.DEVNULL).returncode != 0
    (app/'Contents/Frameworks/escape.dylib').unlink()
    (app/'Contents/Frameworks/missing.dylib').symlink_to('absent')
    assert subprocess.run(validator, stderr=subprocess.DEVNULL).returncode != 0
with tempfile.TemporaryDirectory() as tmp:
    # Rewrite a test COPY only. Production has no environment/root override.
    fixture = pathlib.Path(tmp)
    source = pre.replace('/Library', tmp+'/Library').replace('/Applications', tmp+'/Applications')
    mocks = {
        '/usr/bin/stat': 'case "$2" in *Lp*) echo "0 755";; *) echo 0;; esac', '/usr/bin/find': 'exit 0',
        '/usr/bin/codesign': 'exit 0', '/usr/libexec/PlistBuddy': 'echo dev.nalart.ThroneGtk',
        '/bin/launchctl': 'case "$1" in\nprint) test -f "$STATE";;\nbootout) echo stop >> "$EVENTS"; [ "${STOP_FAIL:-0}" = 0 ] || exit 1; rm "$STATE";;\nesac',
        '/bin/chmod': 'echo "chmod $*" >> "$ACL_EVENTS"',
        '/usr/sbin/chown': 'echo "chown $*" >> "$ACL_EVENTS"',
        '/bin/ls': 'echo metadata; [ -z "${ACL_TEXT:-}" ] || printf "%s\\n" "$ACL_TEXT"; exit 0',
        '/bin/rm': 'echo remove >> "$EVENTS"; /bin/rm "$@"',
    }
    for index, (command, body) in enumerate(mocks.items()):
        mock = fixture/f'mock{index}'
        mock.write_text('#!/bin/sh\n'+body+'\n')
        mock.chmod(0o755)
        source = source.replace(command+' ', str(mock)+' ')
    script = fixture/'preinstall'
    script.write_text(source)
    checker = fixture/'check-install-acl'
    checker.write_text('#!/bin/sh\necho "$*" >> "$ACL_EVENTS"\n[ "${ACL_FAIL:-}" != "$1" ]\n')
    checker.chmod(0o755)
    (fixture/'mode').write_text('manual\n')
    for name in ('Library/Application Support', 'Library/LaunchDaemons', 'Applications'):
        (fixture/name).mkdir(parents=True, exist_ok=True)
    base = fixture/'Library/Application Support/Throne GTK'
    base.mkdir()
    state, events = fixture/'state', fixture/'events'
    state.touch()
    acl_events = fixture/'acl-events'
    env = dict(os.environ, STATE=str(state), EVENTS=str(events), ACL_EVENTS=str(acl_events))
    def preflight(valid, stop_fail='0', acl_fail='', dns_fail='0'):
        result = subprocess.run(['bash', str(script)], env=dict(env, STOP_FAIL=stop_fail, ACL_FAIL=acl_fail, DNS_FAIL=dns_fail), stderr=subprocess.DEVNULL)
        assert (result.returncode == 0) == valid
    preflight(False) # foreign directory: no ownership marker
    assert not events.exists()
    (base/'.throne-gtk-package').write_text('manual\n')
    (base/'.throne-gtk-manual').touch()
    (base/'Throne GTK.app').mkdir()
    core = base/'Throne GTK.app/Contents/MacOS/throne-gtk-core'
    core.parent.mkdir(parents=True)
    core.write_text('#!/bin/sh\n[ "$1" = --recover-dns ] || exit 2\necho recover >> "$EVENTS"\n[ "${DNS_FAIL:-0}" = 0 ]\n')
    core.chmod(0o755)
    plist = fixture/'Library/LaunchDaemons/dev.nalart.ThroneGtk.helper.plist'
    plist.write_text('owned')
    (base/'manual-helper.plist').write_text('owned')
    link = fixture/'Applications/Throne GTK.app'
    link.write_text('foreign')
    preflight(False)
    assert link.read_text() == 'foreign' and not events.exists()
    link.unlink()
    plist.write_text('foreign')
    preflight(False)
    assert not events.exists()
    plist.write_text('owned')
    for blocked in ('--ancestors', '--tree', '--nodes'):
        preflight(False, acl_fail=blocked)
        assert not events.exists() and state.exists()
    preflight(False, '1') # stop failure must propagate before payload phase
    assert state.exists() and plist.read_text() == 'owned'
    preflight(True)
    assert not state.exists() and events.read_text() == 'stop\nstop\nrecover\n'
    preflight(False, dns_fail='1')
    assert plist.exists() and core.exists()
    # Exercise uninstall COPY with mocked system metadata/ACL/launchctl only.
    uninstall_source = uninstall.replace('/Library', tmp+'/Library').replace('/Applications', tmp+'/Applications')
    uninstall_source = uninstall_source.replace('$(id -u)', '$(printf 0)')
    for index, command in enumerate(mocks):
        uninstall_source = uninstall_source.replace(command+' ', str(fixture/f'mock{index}')+' ')
    uninstaller = fixture/'uninstall'
    uninstaller.write_text(uninstall_source)
    shutil.copy2(checker, base/'check-install-acl')
    state.touch()
    events.write_text('')
    for entry in (' 0: group:everyone allow write', ' 0: group:everyone inherited allow writesecurity'):
        result = subprocess.run(['bash', str(uninstaller)], env=dict(env, ACL_TEXT=entry), stderr=subprocess.DEVNULL)
        assert result.returncode != 0 and events.read_text() == ''
        assert plist.exists() and core.exists()
    result = subprocess.run(['bash', str(uninstaller)], env=dict(env, DNS_FAIL='1', ACL_TEXT=' 0: group:everyone allow read,readattr,readsecurity'), stderr=subprocess.DEVNULL)
    assert result.returncode != 0
    assert events.read_text() == 'stop\nrecover\n'
    assert plist.exists() and core.exists() and (base/'.throne-gtk-package').exists()
    run = base/'run'
    run.mkdir()
    (run/'helper.lock').touch()
    control = socket.socket(socket.AF_UNIX)
    bind_fixture_socket(control, run/'helper.sock')
    result = subprocess.run(['bash', str(uninstaller)], env=env, stderr=subprocess.DEVNULL)
    assert result.returncode == 0
    assert events.read_text().startswith('stop\nrecover\nrecover\nremove\n')
    assert not plist.exists() and not core.exists() and not base.exists()
    control.close()
    preflight(False, acl_fail='--ancestors')
    assert not base.exists()
    acl_events.write_text('')
    preflight(True)
    assert base.is_dir()
    checks = acl_events.read_text()
    assert checks.index('--ancestors') < checks.index('chmod -N') < checks.index('chmod 755') < checks.index('--tree')
with tempfile.TemporaryDirectory() as tmp:
    # Execute the production tree/type/mode walker on Linux with ONLY the
    # Darwin ACL reader stubbed. Native ACL behavior still requires macOS.
    fixture = pathlib.Path(tmp)
    source = (r/'scripts/check-macos-install-acl.c').read_text()
    source = source.replace('#include <sys/acl.h>', '')
    source = source.replace('#include "acl_policy.h"', '').replace('#include "../core/internal/sys/acl_presence.h"', '')
    start, end = source.index('static int safe_acl('), source.index('static int safe_node(')
    source = source[:start] + 'static int safe_acl(const char *p) { return getenv("UNSAFE_ACL") ? -1 : 0; }\n' + source[end:]
    # Map fixture owner to root for nonroot CI; ownership rejection is separately
    # exercised by changing the expected uid in a second compiled copy.
    source = source.replace('st.st_uid != 0', 'st.st_uid != '+str(os.getuid()))
    c, binary = fixture/'walker.c', fixture/'walker'
    c.write_text(source)
    subprocess.run(['cc', str(c), '-o', str(binary)], check=True)
    tree = fixture/'base'
    (tree/'run').mkdir(parents=True)
    def check(valid, **env):
        result = subprocess.run([str(binary), '--tree', str(tree)], env=dict(os.environ, **env), stderr=subprocess.DEVNULL)
        assert (result.returncode == 0) == valid
    with socket.socket(socket.AF_UNIX) as control:
        bind_fixture_socket(control, tree/'run/helper.sock')
        (tree/'run/helper.sock').chmod(0o666)
        check(True)
        # Inject a foreign owner for only the socket, retaining the exact
        # production ownership check (no chown/root privileges needed).
        foreign = fixture/'foreign.c'
        foreign.write_text(source.replace('if (lstat(path, &st) ||',
            'if (lstat(path, &st)) return -1; if (strstr(path, "/helper.sock")) st.st_uid = '+str(os.getuid()+1)+'; if ('))
        foreign_binary = fixture/'foreign'
        subprocess.run(['cc', str(foreign), '-o', str(foreign_binary)], check=True)
        assert subprocess.run([str(foreign_binary), '--tree', str(tree)], stderr=subprocess.DEVNULL).returncode != 0
        check(False, UNSAFE_ACL='1')
        (tree/'run').chmod(0o775)
        check(False)  # Generic writable-directory policy is not weakened.
        (tree/'run').chmod(0o755)
        with socket.socket(socket.AF_UNIX) as other:
            bind_fixture_socket(other, tree/'other.sock')
            check(False)
        (tree/'other.sock').unlink()
        (tree/'run/helper.sock').unlink()
        (tree/'run/helper.sock').symlink_to('/dev/null')
        check(False)
        (tree/'run/helper.sock').unlink()
        os.mkfifo(tree/'fifo')
        check(False)
        (tree/'fifo').unlink()
        (tree/'run').rename(tree/'real-run')
        (tree/'run').symlink_to('real-run')
        check(False)
with tempfile.TemporaryDirectory() as tmp:
    # Compile the exact shared portable ACL decision, without Darwin headers.
    compiler = shutil.which('cc')
    assert compiler, 'portable ACL decision checks require a host C compiler'
    source = pathlib.Path(tmp)/'acl-decisions.c'
    source.write_text('''#include "acl_policy.h"
int main(void) {
    if (throne_acl_decision(1, 0, 1, 1) != 0) return 1; /* deny delete */
    if (throne_acl_decision(0, 1, 1, 0) != 0) return 2; /* allow read */
    if (throne_acl_decision(0, 1, 1, 1) == 0) return 3; /* allow mutation */
    if (throne_acl_decision(0, 1, 0, 0) == 0) return 4; /* unreadable */
    if (throne_acl_decision(0, 0, 1, 0) == 0) return 5; /* unknown ACE */
    return 0;
}
''')
    binary = pathlib.Path(tmp)/'acl-decisions'
    subprocess.run([compiler, '-I', str(r/'core/helpertrust'), str(source), '-o', str(binary)], check=True)
    subprocess.run([str(binary)], check=True)
print('macOS packaging regression checks passed')
PY
