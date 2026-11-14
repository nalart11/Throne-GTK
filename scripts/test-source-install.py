#!/usr/bin/env python3
"""Exercise the production source-install recipe without a build or elevation."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap


ROOT = Path(__file__).resolve().parent.parent


def run():
    # Execute the actual recipe body, bypassing only its expensive build dependency.
    tail = (ROOT / "Justfile").read_text().split("\ninstall: build\n", 1)[1]
    lines = []
    for line in tail.splitlines():
        if line and not line.startswith("    "):
            break
        lines.append(line)
    recipe = textwrap.dedent("\n".join(lines)) + "\n"
    assert recipe.startswith("#!/usr/bin/env bash\n")
    with tempfile.TemporaryDirectory(prefix="nolurix-install-retry-") as name:
        work = Path(name)
        checkout = work / "checkout"
        checkout.mkdir()
        shutil.copytree(ROOT / "resources", checkout / "resources")
        (checkout / "scripts").mkdir()
        shutil.copy2(ROOT / "scripts/uninstall-linux.py", checkout / "scripts")
        build = checkout / "build"
        build.mkdir()
        for binary in ("nolurix", "nolurix-core"):
            path = build / binary
            path.write_text("#!/bin/sh\nexit 99\n")
            path.chmod(0o755)
        (build / "libcronet.so").write_bytes(b"fixture")
        mockbin = work / "mockbin"
        mockbin.mkdir()
        sudo = mockbin / "sudo"
        sudo.write_text(
            '#!/bin/sh\nprintf "%s\\n" "$*" >> "$GRANT_LOG"\n'
            'test "$1" = setcap || exit 88\nexit "${GRANT_STATUS:-77}"\n'
        )
        sudo.chmod(0o755)
        for command in ("update-desktop-database", "gtk4-update-icon-cache"):
            path = mockbin / command
            path.write_text("#!/bin/sh\nexit 0\n")
            path.chmod(0o755)
        prefix = work / "prefix"
        grant_log = work / "grants"
        legacy_data = work / "config/throne-gtk/throne-gtk.db"
        legacy_data.parent.mkdir(parents=True)
        legacy_data.write_bytes(b"untouched legacy data")
        env = dict(os.environ, PREFIX=str(prefix), GRANT_LOG=str(grant_log),
                   PATH=f"{mockbin}:{os.environ['PATH']}", GRANT_STATUS="77")

        def install(expected):
            result = subprocess.run(["bash", "-c", recipe], cwd=checkout,
                                    env=env, capture_output=True, text=True)
            assert result.returncode == expected, (result.returncode, result.stderr)

        def check():
            subprocess.run(["python3", str(checkout / "scripts/uninstall-linux.py"),
                            "--check-install", str(prefix)], check=True)

        install(77)
        assert (prefix / "lib/nolurix/.installed-files.json").is_file()
        check()
        env["GRANT_STATUS"] = "0"
        install(0)
        check()
        # A denied upgrade must likewise record the newly copied payload.
        (build / "nolurix-core").write_text("#!/bin/sh\nexit 98\n")
        env["GRANT_STATUS"] = "77"
        install(77)
        check()
        env["GRANT_STATUS"] = "0"
        install(0)
        check()
        for grant in grant_log.read_text().splitlines():
            assert grant == (
                "setcap cap_net_admin,cap_net_raw,cap_net_bind_service+ep "
                f"{prefix}/lib/nolurix/nolurix-core"
            ), grant
        # Unknown/modified targets remain fail-closed and never request elevation.
        prior_grants = grant_log.read_bytes()
        (prefix / "lib/nolurix/nolurix-core").write_text("unrecognized")
        install(1)
        assert grant_log.read_bytes() == prior_grants
        assert legacy_data.read_bytes() == b"untouched legacy data"
    print("Source-install authorization failure/retry fixtures passed")


if __name__ == "__main__":
    run()
