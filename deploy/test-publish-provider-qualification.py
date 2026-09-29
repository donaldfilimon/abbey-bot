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
    omit_tool_continuation: bool = False,
    multi_mode: bool = False,
    refused_pcc: bool = False,
    duplicate_mode: bool = False,
    bad_hash: bool = False,
    mismatched_first: bool = False,
    server_streaming: bool = False,
    bad_server_identity: bool = False,
) -> None:
    source = f'''#!/usr/bin/env python3
import copy, hashlib, json, pathlib, sys
binary = pathlib.Path(sys.argv[0])
digest = hashlib.sha256(binary.read_bytes()).hexdigest()
capabilities = {{name: {{"status": "pass"}} for name in (
    "text", "streaming", "structured_output", "tools", "vision", "ocr")}}
if not {omit_tool_continuation!r}:
    capabilities["tools"]["tool_result_marker"] = "ABBEY_PROVIDER_CONTINUATION_V1"
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
if {multi_mode!r}:
    hashes = {{"abbey_binary_sha256": digest, "provider_binary_sha256": "a"*64, "os_sha256": hashlib.sha256(b"synthetic-os").hexdigest(), "tool_schema_sha256": "b"*64}}
    fm_cli["identity"].update({{"mode": "pcc", "cli_sha256": "a"*64, "os_build": "synthetic-os", "cli_path": "/usr/bin/fm"}})
    fm_cli["vision_identity"] = dict(fm_cli["identity"])
    system = copy.deepcopy(fm_cli)
    system["identity"]["mode"] = "system"
    system["vision_identity"] = dict(system["identity"])
    if {refused_pcc!r}:
        fm_cli["capabilities"] = {{name: {{"status": "fail", "category": "pcc_refused"}} for name in capabilities}}
        fm_cli.pop("vision_identity", None)
    if {server_streaming!r}:
        report["fm_server"] = {{"configured": True, "identity": dict(system["identity"]), "capabilities": {{name: {{"status": "pass" if name in ("text", "streaming") else "unsupported"}} for name in capabilities}}}}
        if {bad_server_identity!r}:
            report["fm_server"]["identity"]["mode"] = "pcc"
    report["fm_cli_modes"] = [copy.deepcopy(fm_cli), system]
    report["fm_manifest_identity"] = hashes
    if {duplicate_mode!r}:
        report["fm_cli_modes"][1] = copy.deepcopy(fm_cli)
    if {bad_hash!r}:
        hashes["tool_schema_sha256"] = "invalid"
    if {mismatched_first!r}:
        report["fm_cli_modes"][0]["identity"]["mode"] = "system"
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

        # A tool-CALL-only probe (tools: pass with no distinct tool-result
        # continuation marker) must never publish as fully qualified.
        fake_binary(binary, passing=True, omit_tool_continuation=True)
        tool_call_only = invoke(binary, output)
        assert tool_call_only.returncode == 1
        assert "tool-result continuation" in tool_call_only.stderr
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

        fake_binary(binary, passing=True, multi_mode=True)
        multi = invoke(binary, output)
        assert multi.returncode == 0, multi.stderr
        records = json.loads(output.read_text())
        assert [r["provider_id"] for r in records] == ["foundation-models-pcc", "foundation-models"]
        assert all(r["qualification_status"] == "qualified" for r in records)
        assert all(r["version"] == 2 for r in records)
        fake_binary(binary, passing=True, multi_mode=True, server_streaming=True)
        with_server = invoke(binary, output)
        assert with_server.returncode == 0, with_server.stderr
        assert json.loads(output.read_text())[1]["declared_capabilities"]["streaming"] is True
        original = output.read_bytes()
        fake_binary(binary, passing=True, multi_mode=True, server_streaming=True, bad_server_identity=True)
        wrong_server = invoke(binary, output)
        assert wrong_server.returncode == 1
        assert output.read_bytes() == original
        fake_binary(binary, passing=True, multi_mode=True, refused_pcc=True)
        partial = invoke(binary, output)
        assert partial.returncode == 0, partial.stderr
        records = json.loads(output.read_text())
        assert records[0]["qualification_status"] == "failed"
        assert not any(records[0]["declared_capabilities"].values())
        assert records[1]["qualification_status"] == "qualified"
        original = output.read_bytes()
        for option in ("duplicate_mode", "bad_hash", "mismatched_first", "omit_tool_continuation"):
            fake_binary(binary, passing=True, multi_mode=True, **{option: True})
            invalid = invoke(binary, output)
            assert invalid.returncode == 1, (option, invalid.stderr)
            assert output.read_bytes() == original
        fake_binary(binary, passing=False, multi_mode=True, refused_pcc=True)
        failed_multi = invoke(binary, output)
        assert failed_multi.returncode == 1
        assert output.read_bytes() == original

        output.unlink()
        target = root / "target.json"
        target.write_text("preserve", encoding="utf-8")
        output.symlink_to(target)
        fake_binary(binary, passing=True)
        rejected = invoke(binary, output)
        assert rejected.returncode == 1
        assert output.is_symlink() and target.read_text(encoding="utf-8") == "preserve"

    print("provider qualification publication tests passed (16 scenarios)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
