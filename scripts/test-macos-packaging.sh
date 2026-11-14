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
fixed = '/Library/Application Support/Nolurix/Nolurix.app'
# Linux cannot compile ServiceManagement, but it can catch shared-header symbol
# drift in the Objective-C adapter before a Darwin compiler is available.
adapter = (r/'crates/ipc/src/service_macos.m').read_text()
security_header = (r/'core/helpertrust/security.h').read_text()
header_functions = set(re.findall(r'static int (\w+)\(', security_header))
adapter_calls = set(re.findall(r'\breturn\s+(\w+)\(', adapter))
assert len(adapter_calls) == 2 and adapter_calls <= header_functions, adapter_calls
assert fixed+'/Contents/MacOS/nolurix-core' in adapter
with (r/'resources/macos/dev.nalart.Nolurix.manual-helper.plist').open('rb') as f:
    manual = plistlib.load(f)
    assert manual['ProgramArguments'][0] == fixed+'/Contents/MacOS/nolurix-core'
    assert manual['Label'] == 'dev.nalart.Nolurix.helper'
with (r/'resources/macos/dev.nalart.Nolurix.helper.plist').open('rb') as f:
    p = plistlib.load(f)
    assert p['BundleProgram'] == 'Contents/MacOS/nolurix-core'
    assert p['Label'] == 'dev.nalart.Nolurix.helper'
with (r/'resources/macos/installer/components.plist').open('rb') as f:
    assert plistlib.load(f)[0]['RootRelativeBundlePath'] == fixed.lstrip('/')
pre = (r/'resources/macos/installer/preinstall').read_text()
# Execute the actual production legacy preflight with redirected fixture paths.
# Any attempt to reach the ACL reader/mutation stage is a test failure.
legacy_preflight = pre[:pre.index("base='/Library/Application Support/Nolurix'")]
for operation in ('"$acl_check" --ancestors', '/bin/launchctl bootout',
                  '"$app/Contents/MacOS/nolurix-core" --recover-dns',
                  '/bin/mkdir', '/bin/chmod', '/usr/sbin/chown'):
    assert pre.index(operation) > len(legacy_preflight)
assert all(command not in legacy_preflight for command in
           ('/bin/rm', '/bin/mkdir', '/bin/chmod', '/usr/sbin/chown', '--recover-dns'))
legacy_paths = [
    '/Library/Application Support/Throne GTK', '/Applications/Throne GTK.app',
    '/Library/LaunchDaemons/dev.nalart.ThroneGtk.helper.plist',
    '/Library/LaunchDaemons/dev.nalart.ThroneGtk.manual-helper.plist',
]
legacy_labels = ['dev.nalart.ThroneGtk.helper', 'dev.nalart.ThroneGtk.manual-helper']
with tempfile.TemporaryDirectory() as tmp:
    fixture = pathlib.Path(tmp)
    launchctl = fixture/'launchctl'
    launchctl.write_text('#!/bin/bash\n[ "$1" = print ] && [ "$2" = "${LOADED:-never}" ]\n')
    launchctl.chmod(0o755)
    source = legacy_preflight.replace('/bin/launchctl', str(launchctl))
    for index, path in enumerate(legacy_paths):
        source = source.replace(path, str(fixture/f'legacy-{index}'))
    script = fixture/'preflight'
    script.write_text(source + '\necho REACHED_SAFE_STAGE\n')
    def run_legacy(loaded=''):
        return subprocess.run(['bash', str(script)], text=True, capture_output=True,
                              env={**os.environ, 'LOADED': loaded})
    assert run_legacy().returncode == 0
    for index in range(len(legacy_paths)):
        node = fixture/f'legacy-{index}'
        for kind in ('directory', 'file', 'dangling-link'):
            if kind == 'directory':
                node.mkdir()
            elif kind == 'file':
                node.touch()
            else:
                node.symlink_to(fixture/'missing-target')
            result = run_legacy()
            assert result.returncode != 0 and 'REACHED_SAFE_STAGE' not in result.stdout
            assert '--unregister-helper' in result.stderr and 'original protected uninstaller' in result.stderr
            assert node.exists() or node.is_symlink()  # no automatic cleanup
            node.rmdir() if kind == 'directory' else node.unlink()
    for label in legacy_labels:
        result = run_legacy('system/'+label)
        assert result.returncode != 0 and 'REACHED_SAFE_STAGE' not in result.stdout
        assert '--unregister-helper' in result.stderr
# The brand migration must not abandon, move, or split the old recovery lock.
dns = (r/'core/internal/sys/set_dns_darwin.go').read_text()
assert dns.count('openDNSJournal("/private/var/db/throne-gtk-dns", 0)') == 2
assert '/private/var/db/nolurix-dns' not in dns
journal = (r/'core/internal/sys/dns_journal_unix.go').read_text()
assert 'unix.O_NOFOLLOW' in journal and 'unix.Flock(lock, unix.LOCK_EX|unix.LOCK_NB)' in journal
assert 'checkDNSFile(fd, j.owner, false)' in journal
assert 'nolurix_dns_safe_acl' in dns
print('Legacy refusal: 12 path fixtures and 2 loaded labels passed; shared DNS journal preserved')
post = (r/'resources/macos/installer/postinstall').read_text()
go = (r/'core/helper_darwin.go').read_text()
rust = (r/'crates/ipc/src/service_macos.rs').read_text()
runtime = '/Library/Application Support/Nolurix/run/'
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
assert 'ditto --noextattr --noacl "$app" "$base/Nolurix.app"' in pkg
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
              '--identifier', 'dev.nalart.Nolurix.local-installer', '--version', '0.2.0',
              '--install-location', '/', 'fixture work/nolurix-local-installer.pkg']
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
    for key in (*credentials, 'NOLURIX_CODESIGN_IDENTITY'):
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
assert uninstall.index('"$acl_check" --tree') < uninstall.index('"$app/Contents/MacOS/nolurix-core" --recover-dns') < uninstall.index('/bin/rm "$plist"')
assert uninstall.index('--recover-dns') < uninstall.index('/bin/rm -f "$base/run/helper.sock" "$base/run/helper.lock"')
assert pre.index('/bin/launchctl bootout') < pre.index('"$app/Contents/MacOS/nolurix-core" --recover-dns')
for source in (pre, uninstall):
    assert 'bootout "system/$label" || fail' in source
    assert 'bootout "system/$label" 2>/dev/null || true' not in source
assert pre.index('"$acl_check" --ancestors') < pre.index('/bin/launchctl bootout')
assert pre.index('"$acl_check" --tree "$base"') < pre.index('/bin/launchctl bootout')
assert '--ancestors / /Library' in pre and '/Applications || fail' not in pre
assert '"$acl_check" --nodes "$plist"' in pre
assert '/bin/chmod -N "$base"' in pre
for resource in ('dev.nalart.Nolurix.helper.plist', 'dev.nalart.Nolurix.manual-helper.plist'):
    with (r/'resources/macos'/resource).open('rb') as f:
        assert not plistlib.load(f).get('AbandonProcessGroup', False)
with tempfile.TemporaryDirectory() as tmp:
    app = pathlib.Path(tmp)/'Test.app'
    (app/'Contents/MacOS').mkdir(parents=True)
    (app/'Contents/Frameworks').mkdir()
    with (app/'Contents/Info.plist').open('wb') as f:
        plistlib.dump({'CFBundleIdentifier':'dev.nalart.Nolurix'}, f)
    for name in ('nolurix', 'nolurix-core'):
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
        '/usr/bin/codesign': 'exit 0', '/usr/libexec/PlistBuddy': 'echo dev.nalart.Nolurix',
        '/bin/launchctl': '[ "$2" = system/dev.nalart.Nolurix.helper ] || exit 1\ncase "$1" in\nprint) test -f "$STATE";;\nbootout) echo stop >> "$EVENTS"; [ "${STOP_FAIL:-0}" = 0 ] || exit 1; rm "$STATE";;\nesac',
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
    base = fixture/'Library/Application Support/Nolurix'
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
    (base/'.nolurix-package').write_text('manual\n')
    (base/'.nolurix-manual').touch()
    (base/'Nolurix.app').mkdir()
    core = base/'Nolurix.app/Contents/MacOS/nolurix-core'
    core.parent.mkdir(parents=True)
    core.write_text('#!/bin/sh\n[ "$1" = --recover-dns ] || exit 2\necho recover >> "$EVENTS"\n[ "${DNS_FAIL:-0}" = 0 ]\n')
    core.chmod(0o755)
    plist = fixture/'Library/LaunchDaemons/dev.nalart.Nolurix.helper.plist'
    plist.write_text('owned')
    (base/'manual-helper.plist').write_text('owned')
    link = fixture/'Applications/Nolurix.app'
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
    assert plist.exists() and core.exists() and (base/'.nolurix-package').exists()
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
