#!/usr/bin/env python3
"""Run Abbey's synthetic provider probes and atomically publish their manifest."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import stat
import subprocess
import sys
import tempfile


MAX_REPORT_BYTES = 256 * 1024
QUALIFICATION_VERSION = 1
FIXTURE_VERSION = "abbey-provider-fixtures-v1"
CAPABILITIES = (
    "text",
    "streaming",
    "structured_output",
    "tools",
    "vision",
    "ocr",
)
# Mirrors the constant of the same name in src/provider_self_test.rs. The
# self-test streams a tool CALL, then a second turn with a synthetic tool
# result, and only records `tools: pass` once the model answers that second
# turn with exactly this marker. `tools: pass` alone does not prove the
# second turn ran; require the marker so a future tool-call-only probe
# cannot publish a manifest that reads as fully qualified
# (docs/superpowers/specs/2026-09-04-mlx-vlm-tool-continuation-diagnosis.md,
# "Two related gaps found while diagnosing").
CONTINUATION_MARKER = "ABBEY_PROVIDER_CONTINUATION_V1"


class PublicationError(Exception):
    """A fixed, non-secret qualification publication failure."""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--target", required=True, choices=("primary", "fm", "all"))
    parser.add_argument("--timeout", type=int, default=900)
    return parser.parse_args()


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_regular_owned(path: pathlib.Path, label: str, *, executable: bool) -> None:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise PublicationError(f"{label} is missing or unreadable") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise PublicationError(f"{label} must be a regular file, not a symlink")
    if metadata.st_uid != os.geteuid():
        raise PublicationError(f"{label} must be owned by the current user")
    if executable and not os.access(path, os.X_OK):
        raise PublicationError(f"{label} is not executable")


def selected_entries(target: str) -> tuple[str, ...]:
    if target == "primary":
        return ("primary",)
    if target == "fm":
        return ("fm_cli",)
    return ("primary", "fm_cli")


def validate_report(report: object, target: str, binary_hash: str) -> dict[str, object]:
    if not isinstance(report, dict):
        raise PublicationError("provider self-test did not emit a JSON object")
    if (
        type(report.get("version")) is not int
        or report.get("version") != QUALIFICATION_VERSION
        or report.get("fixture_version") != FIXTURE_VERSION
        or report.get("target") != target
        or report.get("overall_pass") is not True
    ):
        raise PublicationError("provider self-test did not produce passing current evidence")
    generated = report.get("generated_unix_secs")
    if type(generated) is not int or generated < 0:
        raise PublicationError("provider self-test timestamp is malformed")
    for name in selected_entries(target):
        entry = report.get(name)
        if not isinstance(entry, dict) or entry.get("configured") is not True:
            raise PublicationError(f"provider self-test did not configure selected route {name}")
        identity = entry.get("identity")
        if (
            not isinstance(identity, dict)
            or identity.get("abbey_binary_sha256") != binary_hash
            or identity.get("fixture_version") != FIXTURE_VERSION
        ):
            raise PublicationError(f"provider self-test identity mismatch for {name}")
        capabilities = entry.get("capabilities")
        if not isinstance(capabilities, dict):
            raise PublicationError(f"provider self-test omitted capabilities for {name}")
        for capability in CAPABILITIES:
            evidence = capabilities.get(capability)
            if not isinstance(evidence, dict) or evidence.get("status") not in {
                "pass",
                "fail",
                "unsupported",
                "skipped",
            }:
                raise PublicationError(
                    f"provider self-test did not safely classify {name}.{capability}"
                )
        for capability in ("text", "structured_output", "tools"):
            if capabilities[capability]["status"] != "pass":
                raise PublicationError(
                    f"provider self-test lacks required {name}.{capability} evidence"
                )
        # `tools: pass` alone only proves a tool CALL streamed correctly.
        # Require the distinct tool-RESULT continuation marker too, so a
        # tool-call-only probe can never publish as fully qualified.
        if capabilities["tools"].get("tool_result_marker") != CONTINUATION_MARKER:
            raise PublicationError(
                f"provider self-test recorded {name}.tools: pass but not the "
                "distinct tool-result continuation marker"
            )
        image_passed = any(
            capabilities[capability]["status"] == "pass"
            for capability in ("vision", "ocr")
        )
        if image_passed:
            vision_identity = entry.get("vision_identity")
            if (
                not isinstance(vision_identity, dict)
                or vision_identity.get("abbey_binary_sha256") != binary_hash
                or vision_identity.get("fixture_version") != FIXTURE_VERSION
            ):
                raise PublicationError(
                    f"provider self-test omitted bound image identity for {name}"
                )
            if name == "fm_cli" and vision_identity != identity:
                raise PublicationError(
                    "provider self-test FM image identity differs from its qualified CLI"
                )
            if name == "primary" and (
                not isinstance(vision_identity.get("endpoint"), str)
                or not vision_identity["endpoint"]
                or not isinstance(vision_identity.get("model"), str)
                or not vision_identity["model"]
            ):
                raise PublicationError(
                    "provider self-test primary image identity lacks endpoint or model"
                )
        if name == "primary" and any(
            capabilities[capability]["status"] == "fail" for capability in CAPABILITIES
        ):
            raise PublicationError("primary provider evidence contains a failed capability")

    fm_server = report.get("fm_server")
    if isinstance(fm_server, dict) and fm_server.get("configured") is True:
        server_caps = fm_server.get("capabilities")
        if not isinstance(server_caps, dict) or any(
            not isinstance(server_caps.get(capability), dict)
            for capability in CAPABILITIES
        ):
            raise PublicationError("FM server evidence is incomplete")
        for capability in ("text", "streaming"):
            if server_caps[capability].get("status") != "pass":
                raise PublicationError(
                    f"provider self-test lacks required fm_server.{capability} evidence"
                )
    return report


def fm_records(report: dict[str, object], binary_hash: str) -> list[dict[str, object]]:
    """Project actual mode evidence into runtime V2 records, preserving failed modes."""
    modes = report.get("fm_cli_modes")
    if not isinstance(modes, list) or not modes:
        raise PublicationError("provider self-test omitted FM mode evidence")
    if modes[0] != report.get("fm_cli"):
        raise PublicationError("FM first-mode evidence differs from fm_cli")
    hashes = report.get("fm_manifest_identity")
    required_hashes = {"abbey_binary_sha256", "provider_binary_sha256", "os_sha256", "tool_schema_sha256"}
    optional_hashes = {"model_sha256", "sandbox_sha256"}
    if not isinstance(hashes, dict) or not required_hashes.issubset(hashes) or set(hashes) - required_hashes - optional_hashes or hashes.get("abbey_binary_sha256") != binary_hash:
        raise PublicationError("provider self-test omitted bound FM manifest identity")
    if any(hashes.get(key) is not None for key in optional_hashes):
        raise PublicationError("FM manifest identity has unexpected model or sandbox hashes")
    for key, value in hashes.items():
        if value is None and key in optional_hashes:
            continue
        if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
            raise PublicationError("FM manifest identity hash is malformed")
    if not isinstance(modes[0], dict):
        raise PublicationError("FM first-mode evidence is malformed")
    server = report.get("fm_server")
    server_streaming = False
    if isinstance(server, dict) and server.get("configured") is True:
        caps = server.get("capabilities")
        if server.get("identity") != next((entry.get("identity") for entry in modes if isinstance(entry, dict) and isinstance(entry.get("identity"), dict) and entry["identity"].get("mode") == "system"), None) or not isinstance(caps, dict) or any(not isinstance(caps.get(k), dict) or caps[k].get("status") not in {"pass", "fail", "unsupported", "skipped"} for k in CAPABILITIES):
            raise PublicationError("FM server identity or capabilities are malformed")
        if any(caps[k]["status"] != "pass" for k in ("text", "streaming")):
            raise PublicationError("FM server lacks passing text/streaming evidence")
        server_streaming = True
    records = []
    seen = set()
    for entry in modes:
        if not isinstance(entry, dict) or entry.get("configured") is not True:
            raise PublicationError("FM mode evidence is not configured")
        identity = entry.get("identity")
        if not isinstance(identity, dict) or identity.get("mode") not in {"system", "pcc"}:
            raise PublicationError("FM mode identity is malformed")
        mode = identity["mode"]
        if mode in seen:
            raise PublicationError("FM mode evidence is duplicated")
        seen.add(mode)
        if identity.get("abbey_binary_sha256") != binary_hash or identity.get("fixture_version") != FIXTURE_VERSION or identity.get("cli_sha256") != hashes["provider_binary_sha256"] or not isinstance(identity.get("os_build"), str) or hashlib.sha256(identity["os_build"].encode()).hexdigest() != hashes["os_sha256"]:
            raise PublicationError("FM mode identity mismatch")
        capabilities = entry.get("capabilities")
        if not isinstance(capabilities, dict) or any(not isinstance(capabilities.get(k), dict) or capabilities[k].get("status") not in {"pass", "fail", "unsupported", "skipped"} for k in CAPABILITIES):
            raise PublicationError("FM mode capabilities are malformed")
        qualified = all(capabilities[k]["status"] == "pass" for k in ("text", "structured_output", "tools"))
        if capabilities["tools"]["status"] == "pass" and capabilities["tools"].get("tool_result_marker") != CONTINUATION_MARKER:
            raise PublicationError("FM mode lacks distinct tool-result continuation marker")
        if any(capabilities[k]["status"] == "pass" for k in ("vision", "ocr")) and entry.get("vision_identity") != identity:
            raise PublicationError("FM mode image identity mismatch")
        records.append({
            "version": 2, "fixture_version": FIXTURE_VERSION,
            "provider_id": "foundation-models" if mode == "system" else "foundation-models-pcc",
            "provider_class": "os_managed_local", "identity": hashes,
            "declared_capabilities": {k: capabilities[k]["status"] == "pass" for k in CAPABILITIES},
            "isolation_capabilities": {k: False for k in ("environment_cleared", "absolute_no_shell_execution", "process_tree_contained", "private_runtime_state", "loopback_only", "sandbox_attested")},
            "qualification_status": "qualified" if qualified else "failed",
        })
    if server_streaming:
        system_record = next((record for record in records if record["provider_id"] == "foundation-models"), None)
        if system_record is None:
            raise PublicationError("FM server has no bound system mode")
        system_record["declared_capabilities"]["streaming"] = True
    if not any(record["qualification_status"] == "qualified" for record in records):
        raise PublicationError("FM evidence contains no qualified mode")
    return records


def run_self_test(binary: pathlib.Path, target: str, timeout: int) -> bytes:
    if timeout <= 0 or timeout > 3600:
        raise PublicationError("timeout must be between 1 and 3600 seconds")
    try:
        completed = subprocess.run(
            [str(binary), "--provider-self-test", target, "--json"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as error:
        raise PublicationError("provider self-test timed out") from error
    if completed.returncode != 0:
        raise PublicationError("provider self-test failed")
    if not completed.stdout or len(completed.stdout) > MAX_REPORT_BYTES:
        raise PublicationError("provider self-test report is empty or too large")
    return completed.stdout


def publish(path: pathlib.Path, payload: bytes) -> None:
    parent = path.parent
    try:
        parent_metadata = parent.lstat()
    except OSError as error:
        raise PublicationError("manifest parent is missing or unreadable") from error
    if stat.S_ISLNK(parent_metadata.st_mode) or not stat.S_ISDIR(parent_metadata.st_mode):
        raise PublicationError("manifest parent must be a real directory")
    if parent_metadata.st_uid != os.geteuid():
        raise PublicationError("manifest parent must be owned by the current user")
    if stat.S_IMODE(parent_metadata.st_mode) != 0o700:
        raise PublicationError("manifest parent must be private mode 0700")
    if path.exists() or path.is_symlink():
        require_regular_owned(path, "existing manifest", executable=False)

    temporary: pathlib.Path | None = None
    try:
        descriptor, raw_path = tempfile.mkstemp(prefix=f".{path.name}.", dir=parent)
        temporary = pathlib.Path(raw_path)
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        temporary = None
        directory = os.open(parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if temporary is not None:
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass

    require_regular_owned(path, "published manifest", executable=False)
    if stat.S_IMODE(path.stat().st_mode) != 0o600:
        raise PublicationError("published manifest is not owner-only")


def main() -> int:
    arguments = parse_args()
    # Keep the final path component unresolved so the lstat-based guards can
    # reject a caller-supplied symlink instead of silently following it.
    binary = pathlib.Path(os.path.abspath(arguments.binary))
    output = pathlib.Path(os.path.abspath(arguments.output))
    try:
        require_regular_owned(binary, "Abbey binary", executable=True)
        raw = run_self_test(binary, arguments.target, arguments.timeout)
        try:
            parsed = json.loads(raw)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise PublicationError("provider self-test report is malformed") from error
        binary_hash = sha256(binary)
        if arguments.target == "fm" and isinstance(parsed, dict) and parsed.get("fm_cli_modes"):
            if type(parsed.get("version")) is not int or parsed.get("version") != QUALIFICATION_VERSION or parsed.get("fixture_version") != FIXTURE_VERSION or parsed.get("target") != "fm" or parsed.get("overall_pass") is not True or type(parsed.get("generated_unix_secs")) is not int or parsed["generated_unix_secs"] < 0:
                raise PublicationError("provider self-test did not produce passing current evidence")
            report = fm_records(parsed, binary_hash)
        else:
            report = validate_report(parsed, arguments.target, binary_hash)
        payload = (json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n").encode()
        publish(output, payload)
    except PublicationError as error:
        print(f"provider qualification was not published: {error}", file=sys.stderr)
        return 1
    print("provider qualification passed and was atomically published")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
