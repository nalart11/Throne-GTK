#!/usr/bin/env python3
"""Remove only explicitly managed files; retain user data and unknown files."""
import os
import hashlib
import json
from pathlib import Path
import sys

record = sys.argv[1] == "--record"
check_install = sys.argv[1] == "--check-install"
prefix = Path(os.path.abspath(sys.argv[2] if record or check_install else sys.argv[1]))
files = [
    "lib/nolurix/nolurix", "lib/nolurix/nolurix-core", "lib/nolurix/libcronet.so",
    "bin/nolurix", "share/applications/dev.nalart.Nolurix.desktop",
    "share/icons/hicolor/scalable/apps/dev.nalart.Nolurix.svg",
    "share/icons/hicolor/symbolic/apps/dev.nalart.Nolurix-symbolic.svg",
]
manifest = prefix / "lib/nolurix/.installed-files.json"
def identity(path):
    if path.is_symlink():
        return {"link": os.readlink(path)}
    return {"sha256": hashlib.sha256(path.read_bytes()).hexdigest()}

if record:
    manifest.write_text(json.dumps({name: identity(prefix / name) for name in files
                                    if os.path.lexists(prefix / name)}))
    sys.exit(0)
if manifest.is_symlink() or (not manifest.is_file() and not check_install):
    sys.exit("No installation manifest; inspect and remove owned files manually.")
owned = json.loads(manifest.read_text()) if manifest.is_file() else {}
for name in files:
    path = prefix / name
    if any(p.is_symlink() for p in path.parents):
        sys.exit("Refusing symlink ancestor: " + str(path))
    if path.is_symlink():
        if name != "bin/nolurix" or os.readlink(path) != str(prefix / "lib/nolurix/nolurix"):
            sys.exit("Refusing unknown symlink: " + str(path))
    elif path.exists() and not path.is_file():
        sys.exit("Refusing non-file: " + str(path))
    if os.path.lexists(path) and (name not in owned or identity(path) != owned[name]):
        sys.exit("Refusing unrecognized or modified file: " + str(path))
if check_install:
    sys.exit(0)
for name in files:
    path = prefix / name
    if os.path.lexists(path):
        path.unlink()
manifest.unlink()
print("Removed managed Nolurix files; settings, legacy installations and unknown files retained.")
