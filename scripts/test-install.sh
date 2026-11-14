#!/usr/bin/env bash
# Local fixtures only: never contacts GitHub or executes an installed payload.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
python3 - "$root/scripts/install.sh" <<'PY'
import hashlib, io, os, pathlib, shlex, shutil, subprocess, sys, tarfile, tempfile
script=sys.argv[1]
if os.geteuid()==0:
    p=subprocess.run(['bash',script,'--yes'],capture_output=True)
    assert p.returncode and b'not root' in p.stderr
    sys.exit('Root rejection passed; run remaining tests as an unprivileged user.')
with tempfile.TemporaryDirectory(prefix='throne-tests-') as td:
    # macOS /var is a symlink; fixtures use canonical ancestors just like a safe prefix.
    base=pathlib.Path(td).resolve(); mocks=base/'mocks'; mocks.mkdir(); fixture=base/'fixture'; fixture.mkdir()
    env=dict(os.environ,PATH=str(mocks)+':'+os.environ['PATH'],FIXTURE=str(fixture),HOME=str(base/'home'),TMPDIR=str(base))
    def mock(name,text):
        p=mocks/name; p.write_text('#!/bin/bash\nset -eu\n'+text); p.chmod(0o755)
    mock('uname','if [ "$1" = -s ]; then echo "${MOCK_OS:-Linux}"; else echo "${MOCK_ARCH:-x86_64}"; fi\n')
    mock('getconf','echo "${GLIBC_VERSION:-glibc 2.39}"\n')
    mock('sw_vers','echo "${MAC_VERSION:-15.0}"\n')
    # Execute the real embedded checks with controlled libraries, never host GTK.
    harness=base/'python-harness.py'
    harness.write_text('''import ctypes, ctypes.util, os, sys
source=sys.stdin.read()
sys.argv=['-']+sys.argv[1:]
if len(sys.argv)==2:
    ctypes.util.find_library=lambda name: name
    class Library:
        def __init__(self,name): self.name=name
        def __getattr__(self,attr):
            major,minor=(4,12) if self.name=='gtk-4' else (1,5)
            if os.environ.get('OLD_LIBRARY')==self.name: minor-=1
            return lambda: major if '_major_' in attr else minor
    def load(name):
        if os.environ.get('MISSING_LIBRARY')==name: raise OSError('fixture missing')
        return Library(name)
    ctypes.CDLL=load
elif os.environ.get('FAIL_REPLACE')=='yes':
    original=os.replace
    def replace(src,dst):
        if str(src).endswith(os.environ.get('FAIL_REPLACE_INDEX','2')+'.new'):
            os.environ.pop('FAIL_REPLACE',None)
            os.replace=original
            raise OSError('injected commit failure')
        return original(src,dst)
    os.replace=replace
exec(compile(source,'<installer>','exec'))
''')
    mock('python3','exec '+shlex.quote(shutil.which('python3'))+' '+shlex.quote(str(harness))+' "${@:2}"\n')
    mock('sudo','printf "%s\\n" "$@" > "$FIXTURE/sudo-call"\n')
    mock('curl','''out= url=
while [ "$#" -gt 0 ]; do
 case "$1" in
 --output) out=$2; shift 2;;
 --write-out|--proto|--proto-redir|--max-filesize) shift 2;;
 --*) shift;;
 *) url=$1; shift;;
 esac
done
if [[ "$url" = */latest ]]; then printf '%s' "${LATEST_URL:-https://github.com/nalart11/Nolurix/releases/tag/v0.2.1}"; exit; fi
[ "${FAIL_DOWNLOAD:-no}" = no ] || exit 22
if [ "${MISSING_NEW_ASSET:-no}" = yes ] && [[ "$url" = */nolurix-*.tar.gz ]]; then
 printf 404; exit 22
fi
cp "$FIXTURE/${url##*/}" "$out"
''')
    version='0.2.1'; name='nolurix-'+version+'-linux-x86_64'; asset=name+'.tar.gz'
    paths=['lib/nolurix/nolurix','lib/nolurix/nolurix-core','lib/nolurix/libcronet.so',
      'share/icons/hicolor/scalable/apps/dev.nalart.Nolurix.svg','share/icons/hicolor/symbolic/apps/dev.nalart.Nolurix-symbolic.svg',
      'share/applications/dev.nalart.Nolurix.desktop.in']
    def archive(extra=None):
        with tarfile.open(fixture/asset,'w:gz') as tar:
            for path in paths:
                data=b'[Desktop Entry]\nExec=@EXEC@\n' if path.endswith('.in') else b'fixture only\n'
                m=tarfile.TarInfo(name+'/'+path); m.size=len(data); tar.addfile(m,io.BytesIO(data))
            if extra:
                m=tarfile.TarInfo(extra[0]); m.type=extra[1]; m.linkname='/etc/passwd'; tar.addfile(m,io.BytesIO(b''))
        checksum()
    def checksum(mode='ok',file=asset):
        digest=hashlib.sha256((fixture/file).read_bytes()).hexdigest()
        line=digest+'  '+file+'\n'
        if mode=='missing': line=digest+'  other\n'
        if mode=='duplicate': line*=2
        if mode=='mismatch': line='0'*64+'  '+file+'\n'
        (fixture/'SHA256SUMS').write_text(line)
    prefix=base/'custom space % " quote \' dollar$ slash\\'
    def run(ok=True,args=None,**extra):
        cmd=['bash',script,'--yes']+(args if args is not None else ['--version','v0.2.1','--prefix',str(prefix)])
        p=subprocess.run(cmd,env=dict(env,**extra),capture_output=True,text=True)
        assert (p.returncode==0)==ok,(cmd,p.stdout,p.stderr)
        return p
    # A previous installation is a data source, not disposable migration input.
    old_data=pathlib.Path(env['HOME'])/'.config/throne-gtk/profiles.json'
    old_data.parent.mkdir(parents=True); old_data.write_text('legacy user data')
    old_launcher=prefix/'bin/throne-gtk'
    old_launcher.parent.mkdir(parents=True); old_launcher.write_text('unverified legacy launcher')
    old_desktop=prefix/'share/applications/dev.nalart.ThroneGtk.desktop'
    old_desktop.parent.mkdir(parents=True); old_desktop.write_text('unverified integration')
    archive(); run(); assert (prefix/'bin/nolurix').is_file()
    assert old_data.read_text()=='legacy user data'
    assert old_launcher.read_text()=='unverified legacy launcher'
    assert old_desktop.read_text()=='unverified integration'
    desktop=(prefix/'share/applications/dev.nalart.Nolurix.desktop').read_text()
    assert '%%' in desktop and '\\\\"' in desktop and '@EXEC@' not in desktop
    unrelated=prefix/'lib/nolurix/unrelated'; unrelated.write_text('keep')
    run(); assert unrelated.read_text()=='keep'
    launcher=prefix/'bin/nolurix'
    launcher.unlink(); launcher.symlink_to(prefix/'lib/nolurix/nolurix')
    run(False,FAIL_REPLACE='yes',FAIL_REPLACE_INDEX='6')
    assert launcher.is_symlink() and os.readlink(launcher)==str(prefix/'lib/nolurix/nolurix')
    assert unrelated.read_text()=='keep'
    run(); assert not launcher.is_symlink()
    hostile=base/'hostile-target'; hostile.write_text('untouched')
    launcher.unlink(); launcher.symlink_to(hostile)
    run(False); assert launcher.is_symlink() and hostile.read_text()=='untouched'
    launcher.unlink(); run()
    original=(prefix/'bin/nolurix').read_bytes()
    before={p:p.read_bytes() for p in prefix.rglob('*') if p.is_file()}
    run(False,FAIL_REPLACE='yes')
    assert all(p.read_bytes()==data for p,data in before.items())
    run(False,GLIBC_VERSION='glibc 2.38')
    for library in ['gtk-4','adwaita-1']:
        run(False,OLD_LIBRARY=library); run(False,MISSING_LIBRARY=library)
    for mode in ['missing','duplicate','mismatch']:
        checksum(mode); run(False)
    checksum(); run(False,FAIL_DOWNLOAD='yes')
    # The pinned old asset is accepted only after a proven preferred-asset 404,
    # and must have its own checksum. Other download errors never fall back.
    legacy_asset=asset.replace('nolurix-', 'throne-gtk-', 1)
    with tarfile.open(fixture/legacy_asset,'w:gz') as tar:
        for path in paths:
            oldpath=path.replace('nolurix-core','throne-gtk-core').replace('nolurix','throne-gtk').replace('dev.nalart.Nolurix','dev.nalart.ThroneGtk')
            data=b'[Desktop Entry]\nName=Throne GTK\nIcon=dev.nalart.ThroneGtk\nExec=@EXEC@\n' if path.endswith('.in') else b'historical fixture only\n'
            m=tarfile.TarInfo(name.replace('nolurix-','throne-gtk-',1)+'/'+oldpath)
            m.size=len(data); tar.addfile(m,io.BytesIO(data))
    checksum(file=legacy_asset)
    # Unknown pre-existing old launchers/desktops are never overwritten.
    run(False,MISSING_NEW_ASSET='yes')
    assert old_launcher.read_text()=='unverified legacy launcher'
    legacy_prefix=base/'historical-prefix'
    legacy_args=['--version',version,'--prefix',str(legacy_prefix)]
    p=run(args=legacy_args,MISSING_NEW_ASSET='yes')
    assert 'historical Throne GTK v0.2.1, NOT branded Nolurix' in p.stdout
    assert (legacy_prefix/'lib/throne-gtk/throne-gtk').read_bytes()==b'historical fixture only\n'
    assert (legacy_prefix/'lib/throne-gtk/throne-gtk-core').read_bytes()==b'historical fixture only\n'
    assert (legacy_prefix/'lib/throne-gtk/throne-gtk').stat().st_mode & 0o111
    assert (legacy_prefix/'lib/throne-gtk/throne-gtk-core').stat().st_mode & 0o111
    assert not (legacy_prefix/'lib/nolurix').exists()
    assert 'lib/throne-gtk/throne-gtk' in (legacy_prefix/'bin/throne-gtk').read_text()
    historical_desktop=(legacy_prefix/'share/applications/dev.nalart.ThroneGtk.desktop').read_text()
    assert 'Name=Throne GTK' in historical_desktop and 'Icon=dev.nalart.ThroneGtk' in historical_desktop
    assert 'lib/throne-gtk/throne-gtk' in historical_desktop
    historical_files={p:p.read_bytes() for p in legacy_prefix.rglob('*') if p.is_file()}
    run(args=legacy_args,MISSING_NEW_ASSET='yes')  # verified old upgrade
    run(False,FAIL_DOWNLOAD='yes')
    checksum('mismatch',file=legacy_asset); run(False,args=legacy_args,MISSING_NEW_ASSET='yes')
    checksum()
    run(args=legacy_args)  # New asset afterward installs the new identity alongside.
    assert (legacy_prefix/'lib/nolurix/nolurix').is_file()
    assert (legacy_prefix/'lib/nolurix/nolurix-core').is_file()
    assert (legacy_prefix/'bin/nolurix').is_file()
    assert (legacy_prefix/'share/applications/dev.nalart.Nolurix.desktop').is_file()
    assert all(p.read_bytes()==data for p,data in historical_files.items())
    assert old_data.read_text()=='legacy user data'
    for member,kind in [(name+'/../escape',tarfile.REGTYPE),(name+'/evil',tarfile.SYMTYPE),
                        (name+'/'+paths[0],tarfile.REGTYPE),(name+'/unexpected',tarfile.REGTYPE),
                        (name+'/hard',tarfile.LNKTYPE),(name+'/device',tarfile.CHRTYPE)]:
        archive((member,kind)); run(False)
        assert (prefix/'bin/nolurix').read_bytes()==original
    archive()
    run(False,MOCK_ARCH='aarch64'); run(False,MOCK_OS='FreeBSD')
    run(False,args=['--version','v1.2.3-rc1']); run(False,args=['--version',''])
    run(False,args=['--prefix','relative'])
    run(False,args=[],LATEST_URL='https://evil.invalid/releases/tag/v0.2.1')
    run(False,args=[],LATEST_URL='https://github.com/nalart11/Nolurix/releases/tag/v0.2.1?bad')
    run(args=['--prefix',str(prefix)])
    link=base/'symlink'; link.symlink_to(prefix,target_is_directory=True)
    run(False,args=['--version',version,'--prefix',str(link)])
    mac='nolurix-0.2.1-macos-arm64-testing-local.pkg'; (fixture/mac).write_bytes(b'mock pkg')
    checksum(file=mac)
    macenv=dict(MOCK_OS='Darwin',MOCK_ARCH='arm64')
    run(False,args=['--version',version,'--allow-unsigned-macos'],MAC_VERSION='14.7',**macenv)
    run(False,args=['--version',version,'--allow-unsigned-macos'],MAC_VERSION='invalid',**macenv)
    run(False,args=['--yes','--version',version],**macenv)
    assert not (fixture/'sudo-call').exists()
    run(args=['--version',version,'--allow-unsigned-macos'],**macenv)
    call=(fixture/'sudo-call').read_text()
    assert call.startswith('/usr/sbin/installer\n-pkg\n') and call.endswith('-target\n/\n')
    uninstall=str(pathlib.Path(script).with_name('uninstall-linux.py'))
    current=prefix/'lib/nolurix/nolurix'
    saved=current.read_bytes(); current.write_bytes(b'unrelated replacement')
    p=subprocess.run([sys.executable,uninstall,str(prefix)],capture_output=True)
    assert p.returncode and current.read_bytes()==b'unrelated replacement'
    current.write_bytes(saved)
    subprocess.run([sys.executable,uninstall,str(prefix)],check=True)
    assert unrelated.read_text()=='keep' and old_data.read_text()=='legacy user data'
    assert old_launcher.exists() and old_desktop.exists()
    print('PASS: install/upgrade, checksum failures, archive safety, preservation, versions/latest, platform, symlink destinations, macOS consent/mock installer')
    print('PASS: legacy launcher migration, hostile launcher rejection, rollback, runtime version/missing-library checks')
    print('SKIP: root rejection (requires root); runtime libraries mocked; no real package execution.')
PY
