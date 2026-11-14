#!/usr/bin/env python3
"""Reject escaping links and unresolved/non-system Mach-O dependencies."""
import pathlib
import plistlib
import subprocess
import sys
import re

app = pathlib.Path(sys.argv[1]).resolve(strict=True)
def contained(path):
    resolved = path.resolve(strict=True)
    if not resolved.is_relative_to(app):
        raise ValueError(f"Path escapes bundle: {path}")
    return resolved

with (app / 'Contents/Info.plist').open('rb') as source:
    assert plistlib.load(source)['CFBundleIdentifier'] == 'dev.nalart.Nolurix'
for name in ('nolurix', 'nolurix-core'):
    binary = app / 'Contents/MacOS' / name
    assert binary.is_file() and not binary.is_symlink(), binary
def local_path(value, binary):
    if value == '@executable_path':
        return contained(app / 'Contents/MacOS')
    if value == '@loader_path':
        return contained(binary.parent)
    if value.startswith('@executable_path/'):
        return contained(app / 'Contents/MacOS' / value[len('@executable_path/'):])
    if value.startswith('@loader_path/'):
        return contained(binary.parent / value[len('@loader_path/'):])
    raise ValueError(f'Non-contained runtime path {value} in {binary}')

images = {}
for path in app.rglob('*'):
    contained(path)
    if path.is_symlink() and not path.relative_to(app).as_posix().startswith('Contents/Frameworks/'):
        raise ValueError(f"Link outside Frameworks: {path}")
    if not path.is_file() or path.is_symlink():
        continue
    kind = subprocess.check_output(['file', '-b', str(path)], text=True)
    if 'Mach-O' not in kind:
        continue
    # Parse load commands, not -L: its first dylib entry is LC_ID_DYLIB,
    # an identity rather than a runtime dependency (often @rpath/foo.dylib).
    output = subprocess.check_output(['otool', '-l', str(path)], text=True)
    deps, rpaths = [], []
    command = None
    for line in output.splitlines():
        fields = line.strip().split()
        if len(fields) == 2 and fields[0] == 'cmd':
            command = fields[1]
        match = re.match(r'\s*(?:name|path) (.*) \(offset \d+\)', line)
        if not match:
            continue
        value = match.group(1)
        if command == 'LC_RPATH':
            target = local_path(value, path)
            if not target.is_dir():
                raise ValueError(f'Non-directory LC_RPATH: {value}')
            rpaths.append(target)
        elif command in {'LC_LOAD_DYLIB', 'LC_LOAD_WEAK_DYLIB', 'LC_REEXPORT_DYLIB',
                         'LC_LOAD_UPWARD_DYLIB', 'LC_LAZY_LOAD_DYLIB'}:
            deps.append(value)
        elif command == 'LC_LOAD_DYLINKER' and value != '/usr/lib/dyld':
            raise ValueError(f'Unexpected dynamic linker: {value}')
    images[path.resolve()] = (deps, rpaths)

for path, (deps, rpaths) in images.items():
    # A dylib can inherit the executable's runpath stack. Check both shipped
    # entrypoints independently; reject ambiguous resolution rather than guess.
    for executable in ('nolurix', 'nolurix-core'):
        inherited = images.get((app/'Contents/MacOS'/executable).resolve(), ([], []))[1]
        for dep in deps:
            if dep.startswith(('/usr/lib/', '/System/Library/')):
                continue
            if dep.startswith('@rpath/'):
                candidates = set()
                for directory in rpaths + inherited:
                    candidate = directory / dep[len('@rpath/'):]
                    if candidate.exists():
                        candidates.add(contained(candidate))
                if len(candidates) != 1:
                    raise ValueError(f'Unresolved/ambiguous dependency {dep} in {path}')
                target = candidates.pop()
            else:
                target = local_path(dep, path)
            if not target.is_file() or target not in images:
                raise ValueError(f'Non-Mach-O or missing dependency {dep} in {path}')
