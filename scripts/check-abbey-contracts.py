#!/usr/bin/env python3
"""Verify Abbey's exact vendored Program 1 corpus and privacy taxonomy."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import sys
from typing import Any, Iterable


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_VENDOR_ROOT = ROOT / "contracts/abbey"
LOCK_NAME = "abbey-contracts.lock.json"
MANIFEST_NAME = "manifest.json"
PINNED_REPOSITORY = "https://github.com/donaldfilimon/abi"
PINNED_REVISION = "348754bdaaf59a40fbb858380f925e0aba95a23b"
PINNED_DIGEST = "72e241e34967df318376bf68f4a0e2db13f5ebf17d1a219709731f1f470dbe8e"
PINNED_ARTIFACT_COUNT = 81
PINNED_TOTAL_BYTES = 88_328
MAX_ARTIFACT_BYTES = 1024 * 1024
MAX_CORPUS_BYTES = 16 * 1024 * 1024
AGGREGATE_DOMAIN = b"abbey-contract-corpus-v1\0"
LOCK_KEYS = {
    "source_repository",
    "source_revision",
    "contract_major",
    "contract_revision",
    "aggregate_digest",
}
MANIFEST_KEYS = {
    "contract_major",
    "contract_revision",
    "algorithm",
    "redaction_profile",
    "artifacts",
    "aggregate_digest",
}
FORBIDDEN_KEYS = {
    "audio",
    "transcript",
    "message",
    "prompt",
    "response_text",
    "credential",
    "token",
    "password",
    "username",
    "display_name",
    "filesystem_path",
    "participant_identity",
}


class GuardError(Exception):
    """A closed failure containing only a reason code and relative path."""

    def __init__(self, code: str, path: str | None = None) -> None:
        self.code = code
        self.path = path
        super().__init__(f"{code}: {path}" if path else code)


def _reject_constant(_value: str) -> Any:
    raise GuardError("json_non_finite")


def _pairs_without_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise GuardError("json_duplicate_member")
        result[key] = value
    return result


def _load_json(path: Path, display_path: str) -> Any:
    try:
        status = path.lstat()
    except OSError as exc:
        raise GuardError("artifact_unreadable", display_path) from exc
    if path.is_symlink():
        raise GuardError("symlink_forbidden", display_path)
    if not path.is_file():
        raise GuardError("artifact_not_regular", display_path)
    if status.st_size > MAX_ARTIFACT_BYTES:
        raise GuardError("artifact_too_large", display_path)
    try:
        text = path.read_bytes().decode("utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise GuardError("artifact_unreadable", display_path) from exc
    try:
        return json.loads(
            text,
            object_pairs_hook=_pairs_without_duplicates,
            parse_constant=_reject_constant,
        )
    except GuardError:
        raise
    except (json.JSONDecodeError, RecursionError) as exc:
        raise GuardError("json_invalid", display_path) from exc


def _load_fixture_for_privacy(path: Path, display_path: str) -> Any:
    """Read fixture taxonomy while leaving duplicate-member judgment to Rust."""

    try:
        status = path.lstat()
        if path.is_symlink() or not path.is_file() or status.st_size > MAX_ARTIFACT_BYTES:
            raise OSError
        text = path.read_bytes().decode("utf-8")
        return json.loads(text, parse_constant=_reject_constant)
    except GuardError:
        raise
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, RecursionError) as exc:
        raise GuardError("fixture_unreadable", display_path) from exc


def _relative_artifact_path(value: str) -> PurePosixPath:
    candidate = PurePosixPath(value)
    if (
        not value
        or "\\" in value
        or candidate.is_absolute()
        or any(part in {"", ".", ".."} for part in candidate.parts)
    ):
        raise GuardError("manifest_path_invalid", f"corpus/{MANIFEST_NAME}")
    return candidate


def _discover(corpus: Path) -> tuple[str, ...]:
    discovered: list[str] = []
    for directory, directory_names, file_names in os.walk(corpus, followlinks=False):
        current = Path(directory)
        for name in directory_names:
            candidate = current / name
            if candidate.is_symlink():
                relative = candidate.relative_to(corpus).as_posix()
                raise GuardError("symlink_forbidden", f"corpus/{relative}")
        for name in file_names:
            if name == ".DS_Store":
                continue
            candidate = current / name
            relative = candidate.relative_to(corpus).as_posix()
            if candidate.is_symlink():
                raise GuardError("symlink_forbidden", f"corpus/{relative}")
            if not candidate.is_file():
                raise GuardError("artifact_not_regular", f"corpus/{relative}")
            if relative != MANIFEST_NAME:
                discovered.append(relative)
    return tuple(sorted(discovered, key=lambda value: value.encode("utf-8")))


def _walk_values(value: Any) -> Iterable[tuple[str | None, Any]]:
    if isinstance(value, dict):
        for key, item in value.items():
            yield key, item
            yield from _walk_values(item)
    elif isinstance(value, list):
        for item in value:
            yield None, item
            yield from _walk_values(item)


def _contains_private_sentinel(value: Any) -> bool:
    for key, item in _walk_values(value):
        if key is not None and key.lower() in FORBIDDEN_KEYS:
            return True
        if isinstance(item, str):
            if item.isdecimal() and 17 <= len(item) <= 20:
                return True
            if item.startswith(("/Users/", "/home/", "C:\\", "sk-", "ghp_")):
                return True
    return False


def _verify_privacy_taxonomy(corpus: Path, listed: Iterable[str]) -> None:
    prefix = "v1/fixtures/"
    for relative in listed:
        if not relative.startswith(prefix) or not relative.endswith(".json"):
            continue
        fixture = _load_fixture_for_privacy(corpus / relative, f"corpus/{relative}")
        if not isinstance(fixture, dict) or set(fixture) != {"case_id", "schema", "expect", "document"}:
            raise GuardError("fixture_shape", f"corpus/{relative}")
        taxonomy = PurePosixPath(relative).parts[2]
        expected = fixture.get("expect")
        if taxonomy == "privacy":
            if expected not in {
                "forbidden_content",
                "learning_authority_forbidden",
                "schema_invalid",
            }:
                raise GuardError("privacy_taxonomy_mismatch", f"corpus/{relative}")
        elif _contains_private_sentinel(fixture.get("document")):
            raise GuardError("privacy_taxonomy_mismatch", f"corpus/{relative}")


def _fixed_json_bytes(value: Any) -> bytes:
    try:
        return (json.dumps(value, ensure_ascii=False, allow_nan=False, indent=2) + "\n").encode(
            "utf-8"
        )
    except (TypeError, ValueError) as exc:
        raise GuardError("manifest_shape", f"corpus/{MANIFEST_NAME}") from exc


def _aggregate_digest(rows: list[dict[str, Any]], manifest: dict[str, Any]) -> str:
    zeroed = dict(manifest)
    zeroed["aggregate_digest"] = "0" * 64
    manifest_bytes = _fixed_json_bytes(zeroed)
    entries = [dict(row) for row in rows]
    entries.append(
        {
            "path": MANIFEST_NAME,
            "bytes": len(manifest_bytes),
            "sha256": hashlib.sha256(manifest_bytes).hexdigest(),
        }
    )
    entries.sort(key=lambda row: row["path"].encode("utf-8"))
    digest = hashlib.sha256()
    digest.update(AGGREGATE_DOMAIN)
    for row in entries:
        digest.update(row["path"].encode("utf-8"))
        digest.update(b"\0")
        digest.update(str(row["bytes"]).encode("ascii"))
        digest.update(b"\0")
        digest.update(row["sha256"].encode("ascii"))
        digest.update(b"\n")
    return digest.hexdigest()


def verify(root: Path) -> tuple[int, int, str]:
    """Verify a managed vendor directory without returning corpus content."""

    if root.is_symlink():
        raise GuardError("vendor_symlink")
    lock_path = root / LOCK_NAME
    lock = _load_json(lock_path, LOCK_NAME)
    if not isinstance(lock, dict) or set(lock) != LOCK_KEYS:
        raise GuardError("lock_shape", LOCK_NAME)
    if lock.get("source_repository") != PINNED_REPOSITORY:
        raise GuardError("lock_source_repository_mismatch", LOCK_NAME)
    if lock.get("source_revision") != PINNED_REVISION:
        raise GuardError("lock_source_revision_mismatch", LOCK_NAME)
    if lock.get("contract_major") != 1 or lock.get("contract_revision") != 1:
        raise GuardError("lock_contract_version_mismatch", LOCK_NAME)
    if lock.get("aggregate_digest") != PINNED_DIGEST:
        raise GuardError("lock_aggregate_mismatch", LOCK_NAME)

    corpus = root / "corpus"
    if not corpus.is_dir() or corpus.is_symlink():
        raise GuardError("corpus_missing", "corpus")
    manifest_path = corpus / MANIFEST_NAME
    manifest = _load_json(manifest_path, f"corpus/{MANIFEST_NAME}")
    if not isinstance(manifest, dict) or set(manifest) != MANIFEST_KEYS:
        raise GuardError("manifest_shape", f"corpus/{MANIFEST_NAME}")
    if (
        manifest.get("contract_major") != 1
        or manifest.get("contract_revision") != 1
        or manifest.get("algorithm") != "abbey-contract-corpus-sha256-v1"
        or manifest.get("redaction_profile") != "abbey-contract-redaction-v1"
    ):
        raise GuardError("manifest_identity_mismatch", f"corpus/{MANIFEST_NAME}")
    if manifest.get("aggregate_digest") != PINNED_DIGEST:
        raise GuardError("manifest_aggregate_mismatch", f"corpus/{MANIFEST_NAME}")
    rows = manifest.get("artifacts")
    if not isinstance(rows, list):
        raise GuardError("manifest_shape", f"corpus/{MANIFEST_NAME}")

    indexed: dict[str, dict[str, Any]] = {}
    for row in rows:
        if not isinstance(row, dict):
            raise GuardError("manifest_artifact_shape", f"corpus/{MANIFEST_NAME}")
        required = {"path", "bytes", "media_type", "sha256"}
        allowed = required | {"schema_id"}
        if not required.issubset(row) or not set(row).issubset(allowed):
            raise GuardError("manifest_artifact_shape", f"corpus/{MANIFEST_NAME}")
        relative = row.get("path")
        if not isinstance(relative, str):
            raise GuardError("manifest_artifact_shape", f"corpus/{MANIFEST_NAME}")
        _relative_artifact_path(relative)
        if relative in indexed:
            raise GuardError("manifest_duplicate_artifact", f"corpus/{relative}")
        byte_count = row.get("bytes")
        digest = row.get("sha256")
        if (
            isinstance(byte_count, bool)
            or not isinstance(byte_count, int)
            or not 0 <= byte_count <= MAX_ARTIFACT_BYTES
            or not isinstance(digest, str)
            or re.fullmatch(r"[0-9a-f]{64}", digest) is None
        ):
            raise GuardError("manifest_artifact_shape", f"corpus/{relative}")
        indexed[relative] = row

    actual = set(_discover(corpus))
    listed = set(indexed)
    mismatch = sorted(actual ^ listed, key=lambda value: value.encode("utf-8"))
    if mismatch:
        raise GuardError("corpus_inventory_mismatch", f"corpus/{mismatch[0]}")

    _verify_privacy_taxonomy(corpus, indexed)

    total_bytes = 0
    for relative, row in indexed.items():
        path = corpus / relative
        try:
            raw = path.read_bytes()
        except OSError as exc:
            raise GuardError("artifact_unreadable", f"corpus/{relative}") from exc
        total_bytes += len(raw)
        if len(raw) != row["bytes"]:
            raise GuardError("artifact_length_mismatch", f"corpus/{relative}")
        if hashlib.sha256(raw).hexdigest() != row["sha256"]:
            raise GuardError("artifact_digest_mismatch", f"corpus/{relative}")
    if total_bytes > MAX_CORPUS_BYTES:
        raise GuardError("corpus_too_large")
    if _aggregate_digest(rows, manifest) != PINNED_DIGEST:
        raise GuardError("aggregate_digest_mismatch", f"corpus/{MANIFEST_NAME}")
    if len(indexed) != PINNED_ARTIFACT_COUNT or total_bytes != PINNED_TOTAL_BYTES:
        raise GuardError("corpus_identity_mismatch", f"corpus/{MANIFEST_NAME}")
    return len(indexed), total_bytes, PINNED_DIGEST


def _parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=DEFAULT_VENDOR_ROOT)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(sys.argv[1:] if argv is None else argv)
    try:
        artifact_count, total_bytes, digest = verify(args.root)
    except GuardError as exc:
        print(f"abbey-contracts-check: {exc}", file=sys.stderr)
        return 1
    print(
        f"abbey-contracts-check: verified {artifact_count} artifacts "
        f"({total_bytes} bytes), digest={digest}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
