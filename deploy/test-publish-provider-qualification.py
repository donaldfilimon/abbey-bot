#!/usr/bin/env python3
"""Offline tests for atomic provider qualification publication."""

from __future__ import annotations

import json
import os
import pathlib
import stat
import subprocess
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[1]
PUBLISHER = ROOT / "deploy" / "publish-provider-qualification.py"


def fake_binary(
    path: pathlib.Path,
    *,
    passing: bool,
    optional_image_failure: bool = False,
    omit_vision_identity: bool = False,
    mismatch_vision_identity: bool = False,
) -> None:
    source = f'''#!/usr/bin/env python3
import hashlib, json, pathlib, sys
binary = pathlib.Path(sys.argv[0])
digest = hashlib.sha256(binary.read_bytes()).hexdigest()
capabilities = {{name: {{"status": "pass"}} for name in (
    "text", "streaming", "structured_output", "tools", "vision", "ocr")}}
if {optional_image_failure!r}:
    capabilities["vision"] = {{"status": "fail", "category": "semantic_vision"}}
    capabilities["ocr"] = {{"status": "fail", "category": "semantic_ocr"}}
identity = {{"abbey_binary_sha256": digest, "fixture_version": "abbey-provider-fixtures-v1"}}
skipped = {{"configured": False, "capabilities": {{name: {{"status": "skipped"}} for name in capabilities}}}}
fm_cli = {{"configured": True, "identity": identity, "capabilities": capabilities}}
if not {omit_vision_identity!r}:
    fm_cli["vision_identity"] = dict(identity)
    if {mismatch_vision_identity!r}:
        fm_cli["vision_identity"]["mode"] = "different-route"
report = {{
    "version": 1,
    "fixture_version": "abbey-provider-fixtures-v1",
    "generated_unix_secs": 1,
    "target": "fm",
    "overall_pass": {passing!r},
    "primary": skipped,
    "fm_server": skipped,
    "fm_cli": fm_cli,
}}
print(json.dumps(report))
sys.exit(0 if report["overall_pass"] else 1)
'''
    path.write_text(source, encoding="utf-8")
    path.chmod(0o700)


def invoke(binary: pathlib.Path, output: pathlib.Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "python3",
            str(PUBLISHER),
            "--binary",
            str(binary),
            "--output",
            str(output),
            "--target",
            "fm",
        ],
        text=True,
        capture_output=True,
        check=False,
    )


def main() -> int:
    if not hasattr(os, "geteuid"):
        print("provider qualification publication tests skipped: POSIX-only publisher")
        return 0
    with tempfile.TemporaryDirectory(prefix="abbey-provider-publish-") as raw:
        root = pathlib.Path(raw)
        binary = root / "abbey"
        output = root / "qualification.json"

        fake_binary(binary, passing=True)
        passed = invoke(binary, output)
        assert passed.returncode == 0, passed.stderr
        assert output.is_file() and not output.is_symlink()
        assert stat.S_IMODE(output.stat().st_mode) == 0o600
        assert json.loads(output.read_text())["overall_pass"] is True

        original = output.read_bytes()
        fake_binary(binary, passing=True, omit_vision_identity=True)
        missing_image_identity = invoke(binary, output)
        assert missing_image_identity.returncode == 1
        assert output.read_bytes() == original

        fake_binary(binary, passing=True, mismatch_vision_identity=True)
        mismatched_image_identity = invoke(binary, output)
        assert mismatched_image_identity.returncode == 1
        assert output.read_bytes() == original

        # FM text/schema/tool qualification remains publishable when remote
        # vision is selected and the separately recorded FM image probes fail.
        fake_binary(
            binary,
            passing=True,
            optional_image_failure=True,
            omit_vision_identity=True,
        )
        optional = invoke(binary, output)
        assert optional.returncode == 0, optional.stderr
        assert json.loads(output.read_text())["fm_cli"]["capabilities"]["vision"][
            "status"
        ] == "fail"

        original = output.read_bytes()
        fake_binary(binary, passing=False)
        failed = invoke(binary, output)
        assert failed.returncode == 1
        assert output.read_bytes() == original

        output.unlink()
        target = root / "target.json"
        target.write_text("preserve", encoding="utf-8")
        output.symlink_to(target)
        fake_binary(binary, passing=True)
        rejected = invoke(binary, output)
        assert rejected.returncode == 1
        assert output.is_symlink() and target.read_text(encoding="utf-8") == "preserve"

    print("provider qualification publication tests passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
