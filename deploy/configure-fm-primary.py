#!/usr/bin/env python3
"""Dry-run by default FM primary cutover using the MLX install transaction."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import re
from pathlib import Path
import shlex
import os
import stat
import subprocess

_spec = importlib.util.spec_from_file_location("abbey_mlx_transaction", Path(__file__).with_name("configure-mlx-primary.py"))
assert _spec is not None and _spec.loader is not None
transaction = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(transaction)
InstallLock = transaction.InstallLock
LockFailure = transaction.LockFailure
fail = transaction.fail
validate_launchctl = transaction.validate_launchctl
service_pid = transaction.service_pid
publish = transaction.publish
restore = transaction.restore
restart_and_require_stable = transaction.restart_and_require_stable
LAUNCHCTL = transaction.LAUNCHCTL
MANAGED_KEYS = ("ABBEY_FM_MODE", "ABBEY_FM_ROLE", "ABBEY_FM_CLI", "ABBEY_FM_CAPABILITY_MANIFEST")
HEADER = "# FM primary; managed by deploy/configure-fm-primary.py."


def validate_manifest(manifest: Path, binary: Path, cli: Path, *, system_streaming_required: bool = False) -> None:
    metadata = transaction.require_private_regular_file(manifest, "FM capability manifest")
    transaction.require_private_directory(manifest.parent, "manifest parent")
    if stat.S_IMODE(metadata.st_mode) != 0o600 or stat.S_IMODE(manifest.parent.lstat().st_mode) != 0o700:
        fail("FM manifest and parent require exact modes 0600 and 0700")
    if metadata.st_size > transaction.MAX_MANIFEST_BYTES:
        fail("FM capability manifest exceeds the size limit")
    try:
        records = json.loads(manifest.read_bytes())
    except (ValueError, UnicodeError):
        fail("FM capability manifest is malformed")
    if not isinstance(records, list) or not records:
        fail("FM primary requires a V2 manifest")
    ids = [record.get("provider_id") for record in records if isinstance(record, dict)]
    if len(ids) != len(records) or any(not isinstance(value, str) for value in ids) or len(set(ids)) != len(ids):
        fail("FM manifest records are malformed or duplicated")
    for candidate in records:
        allowed = {"version", "fixture_version", "provider_id", "provider_class", "identity", "declared_capabilities", "isolation_capabilities", "qualification_status"}
        if set(candidate) - allowed:
            fail("FM manifest record contains unknown fields")
        if candidate.get("version") != 2 or type(candidate.get("version")) is not int or candidate.get("fixture_version") != transaction.FIXTURE_VERSION or candidate.get("qualification_status") not in {"qualified", "failed"}:
            fail("FM manifest record schema is invalid")
        if candidate.get("provider_id") not in {"foundation-models", "foundation-models-pcc"} or candidate.get("provider_class") != "os_managed_local":
            fail("FM configurator requires scoreless FM publisher records")
        hashes = candidate.get("identity")
        required = {"abbey_binary_sha256", "provider_binary_sha256", "os_sha256", "tool_schema_sha256"}
        if not isinstance(hashes, dict) or set(hashes) - required - {"model_sha256", "sandbox_sha256"} or not required.issubset(hashes) or any(not isinstance(hashes[k], str) or re.fullmatch(r"[0-9a-f]{64}", hashes[k]) is None for k in required):
            fail("FM manifest identity hashes are malformed")
        if any(hashes.get(key) is not None for key in ("model_sha256", "sandbox_sha256")):
            fail("FM identity has unexpected model or sandbox hashes")
        caps = candidate.get("declared_capabilities")
        names = {"text", "streaming", "structured_output", "tools", "vision", "ocr"}
        if not isinstance(caps, dict) or set(caps) != names or any(type(value) is not bool for value in caps.values()):
            fail("FM manifest capability schema is invalid")
        isolation = candidate.get("isolation_capabilities")
        names = {"environment_cleared", "absolute_no_shell_execution", "process_tree_contained", "private_runtime_state", "loopback_only", "sandbox_attested"}
        if not isinstance(isolation, dict) or set(isolation) != names or any(type(value) is not bool for value in isolation.values()):
            fail("FM manifest isolation schema is invalid")
    expected = candidate_identity(binary, cli)
    qualified = [
        record for record in records
        if record["qualification_status"] == "qualified"
        and {key: value for key, value in record["identity"].items() if value is not None} == expected
        and all(record["declared_capabilities"][key] is True for key in ("text", "structured_output", "tools"))
        and (record["provider_id"] != "foundation-models" or not system_streaming_required or record["declared_capabilities"]["streaming"] is True)
    ]
    if not qualified:
        fail("FM primary requires at least one qualified FM mode with the exact candidate identity and required capabilities")


def candidate_identity(binary: Path, cli: Path) -> dict[str, str]:
    """Ask the candidate's production identity seam, with no ambient credentials."""
    try:
        completed = subprocess.run(
            [str(binary), "--fm-manifest-identity", "--cli", str(cli), "--json"],
            env={}, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, timeout=30, check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        fail("candidate FM identity inspection failed")
    if completed.returncode != 0 or not completed.stdout or len(completed.stdout) > 4096:
        fail("candidate FM identity inspection failed")
    try:
        identity = json.loads(completed.stdout)
    except (ValueError, UnicodeError):
        fail("candidate FM identity is malformed")
    required = {"abbey_binary_sha256", "provider_binary_sha256", "os_sha256", "tool_schema_sha256"}
    if not isinstance(identity, dict) or set(identity) != required or any(not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None for value in identity.values()):
        fail("candidate FM identity is malformed")
    if identity["abbey_binary_sha256"] != transaction.sha256(binary) or identity["provider_binary_sha256"] != transaction.sha256(cli):
        fail("candidate executable identity changed during validation")
    return identity


def rendered_environment(original: str, cli: Path, manifest: Path) -> str:
    kept = []
    seen = set()
    for line in original.splitlines(keepends=True):
        match = transaction.ASSIGNMENT.match(line.strip())
        key = match.group(1) if match else None
        if key in MANAGED_KEYS:
            if key in seen:
                fail("the environment contains duplicate managed key " + key)
            seen.add(key)
        elif line.rstrip("\r\n") != HEADER:
            kept.append(line)
    content = "".join(kept)
    if content and not content.endswith("\n"):
        content += "\n"
    values = ("pcc,system", "primary", str(cli), str(manifest))
    return content + HEADER + "\n" + "".join(f"{key}={shlex.quote(value)}\n" for key, value in zip(MANAGED_KEYS, values))


def run_locked(args: argparse.Namespace) -> None:
    env_file = args.env_file
    if any(not path.is_absolute() for path in (env_file, args.manifest, args.binary, args.cli, args.backup_dir)):
        fail("cutover paths must be absolute")
    transaction.require_private_regular_file(env_file, "Abbey environment")
    transaction.require_owned_executable(args.binary)
    try:
        metadata = args.cli.lstat()
    except OSError:
        fail("FM CLI is missing or unreadable")
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid not in {0, os.getuid()} or metadata.st_mode & 0o022 or not os.access(args.cli, os.X_OK):
        fail("FM CLI must be a trusted regular executable")
    original = env_file.read_bytes()
    try:
        original_text = original.decode("utf-8")
    except UnicodeError:
        fail("Abbey environment is not UTF-8")
    fallback_values = []
    endpoint_values = []
    for line in original_text.splitlines():
        match = transaction.ASSIGNMENT.match(line.strip())
        if match and match.group(1) in {"ABBEY_FM_FALLBACK", "ABBEY_FM_ENDPOINT"}:
            try:
                parsed_value = shlex.split(line.split("=", 1)[1], comments=True)
                (fallback_values if match.group(1) == "ABBEY_FM_FALLBACK" else endpoint_values).append(parsed_value)
            except ValueError:
                fail("existing FM fallback setting is malformed")
    if len(fallback_values) > 1 or any(value not in ([], [""], ["1"], ["true"], ["on"]) for value in fallback_values):
        fail("existing ABBEY_FM_FALLBACK conflicts with primary; operator must resolve it separately")
    validate_manifest(args.manifest, args.binary, args.cli, system_streaming_required=any(value not in ([], [""]) for value in endpoint_values))
    content = rendered_environment(original_text, args.cli, args.manifest)
    if not args.apply:
        print("validated FM primary cutover; dry run")
        print("managed keys: " + ", ".join(MANAGED_KEYS))
        return
    launchctl = validate_launchctl(args.launchctl or LAUNCHCTL, args.launchctl is not None)
    previous_pid = service_pid(launchctl)
    if previous_pid is None:
        fail("the existing Abbey launchd service has no running process")
    backup = publish(env_file, args.backup_dir, original, content)
    try:
        new_pid = restart_and_require_stable(launchctl, previous_pid)
    except BaseException:
        try:
            restore(env_file, backup)
        except BaseException:
            fail(
                "candidate restart failed and the previous environment could not be "
                f"restored; rollback copy retained: {backup}"
            )
        rollback_pid = service_pid(launchctl)
        if rollback_pid is None:
            rollback_pid = previous_pid
        try:
            restart_and_require_stable(launchctl, rollback_pid)
        except BaseException:
            fail(
                "candidate restart failed; the previous environment was restored but "
                f"its service restart failed; rollback copy retained: {backup}"
            )
        fail(
            "candidate restart failed; the previous environment and service were "
            f"restored; rollback copy retained: {backup}"
        )
    print(f"updated owner-only Abbey environment: {env_file}")
    print(f"verified stable Abbey launchd process: {new_pid}")
    print(f"retained owner-only rollback copy: {backup}")



def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--env-file", type=Path, default=Path.home()/".config/abbey-bot/env")
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--cli", type=Path, default=Path("/usr/bin/fm"))
    parser.add_argument("--backup-dir", type=Path, default=Path.home()/".local/share/abbey-bot/env-backups")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--apply", action="store_true")
    mode.add_argument("--dry-run", action="store_true")
    parser.add_argument("--launchctl", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--install-lock", type=Path, default=Path.home()/".local/share/abbey-bot/install.lock", help=argparse.SUPPRESS)
    args = parser.parse_args()
    try:
        with InstallLock(args.install_lock):
            run_locked(args)
    except LockFailure as error:
        fail(str(error))

if __name__ == "__main__":
    main()
