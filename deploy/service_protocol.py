"""Strict, content-free managed-service protocol validation.

This module samples one document. Transaction polling, fresh run identity, and
continuous readiness are the installer's responsibility. It performs no writes.
"""
from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
import errno
import json
import os
from pathlib import Path
import re
import stat
from typing import Callable


class Failure(str, Enum):
    INVALID_DOCUMENT = "invalid_document"
    UNSAFE_FILE = "unsafe_file"
    UNAVAILABLE = "unavailable"
    IDENTITY_MISMATCH = "identity_mismatch"
    NOT_READY = "not_ready"
    STALE = "stale"
    PROCESS_UNAVAILABLE = "process_unavailable"


class ProtocolError(Exception):
    def __init__(self, category: Failure):
        self.category = category
        super().__init__(category.value)


# Shared source authority; Rust/Python conformance tests consume this same file.
_SCHEMA = json.loads(Path(__file__).with_name("service-protocol-v1.json").read_text())
MAX_TIME = (1 << 63) - 1
MAX_PID = (1 << 31) - 1
_HEX = re.compile(r"[0-9a-f]{64}", re.ASCII)


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ProtocolError(Failure.INVALID_DOCUMENT)
        result[key] = value
    return result


def _reject_constant(_value):
    raise ProtocolError(Failure.INVALID_DOCUMENT)


def parse_document(raw: bytes, kind: str = "readiness") -> dict:
    """Decode exact v1 keys/types; never return raw JSON in an error."""
    schema = _SCHEMA.get(kind)
    if schema is None or not isinstance(raw, bytes) or len(raw) > schema["max_bytes"]:
        raise ProtocolError(Failure.INVALID_DOCUMENT)
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=_pairs,
                           parse_constant=_reject_constant)
    except (UnicodeError, ValueError, RecursionError):
        raise ProtocolError(Failure.INVALID_DOCUMENT) from None
    if type(value) is not dict or value.keys() != schema["fields"].keys():
        raise ProtocolError(Failure.INVALID_DOCUMENT)
    for key, rule in schema["fields"].items():
        item = value[key]
        if rule["type"] == "integer":
            valid = type(item) is int and rule["min"] <= item <= rule["max"]
        elif rule["type"] == "enum":
            valid = type(item) is str and item in rule["values"]
        else:
            valid = type(item) is str and _HEX.fullmatch(item) is not None
        if not valid:
            raise ProtocolError(Failure.INVALID_DOCUMENT)
    return value


def encode_document(value: dict, kind: str = "readiness") -> bytes:
    """Produce canonical key order and trailing LF after strict validation."""
    try:
        raw = json.dumps(value, separators=(",", ":"), allow_nan=False).encode()
    except (TypeError, ValueError, RecursionError):
        raise ProtocolError(Failure.INVALID_DOCUMENT) from None
    valid = parse_document(raw, kind)
    encoded = (json.dumps({k: valid[k] for k in _SCHEMA[kind]["fields"]},
                          separators=(",", ":")) + "\n").encode()
    if len(encoded) > _SCHEMA[kind]["max_bytes"]:
        raise ProtocolError(Failure.INVALID_DOCUMENT)
    return encoded


@dataclass(frozen=True, repr=False)
class RunIdentity:
    pid: int
    nonce: str
    executable_sha256: str

    def __repr__(self):
        return "RunIdentity(<private>)"


def identity(document: dict) -> RunIdentity:
    return RunIdentity(document["pid"], document["run_nonce"], document["executable_sha256"])


def fresh(published_ms: int, transaction_start_ms: int, now_ms: int) -> bool:
    values = (published_ms, transaction_start_ms, now_ms)
    if any(type(v) is not int or not 0 <= v <= MAX_TIME for v in values):
        return False
    if published_ms < transaction_start_ms:
        return False
    if published_ms > now_ms:
        return published_ms - now_ms <= 2_000
    return now_ms - published_ms <= 30_000


def process_exists(pid: int) -> bool:
    """Both ESRCH and EPERM fail closed for this user's launchd domain."""
    if type(pid) is not int or not 1 <= pid <= MAX_PID:
        return False
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def validate_ready(document: dict, *, transaction_start_ms: int, now_ms: int,
                   launchd_pid: int, expected_sha256: str,
                   alive: Callable[[int], bool] = process_exists) -> RunIdentity:
    # Also validate callers passing an in-memory object instead of parsed bytes.
    document = parse_document(encode_document(document))
    if (type(launchd_pid) is not int or not 1 <= launchd_pid <= MAX_PID
            or type(expected_sha256) is not str or not _HEX.fullmatch(expected_sha256)):
        raise ProtocolError(Failure.IDENTITY_MISMATCH)
    if document["pid"] != launchd_pid or document["executable_sha256"] != expected_sha256:
        raise ProtocolError(Failure.IDENTITY_MISMATCH)
    if (document["phase"] != "ready" or document["discord"] != "ready"
            or document["scheduler"] != "running"):
        raise ProtocolError(Failure.NOT_READY)
    if not fresh(document["published_at_unix_ms"], transaction_start_ms, now_ms):
        raise ProtocolError(Failure.STALE)
    try:
        present = alive(launchd_pid)
    except OSError:
        present = False
    if present is not True:
        raise ProtocolError(Failure.PROCESS_UNAVAILABLE)
    return identity(document)


def _safe(metadata, uid: int, mode: int, directory: bool) -> bool:
    return (metadata.st_uid == uid and stat.S_IMODE(metadata.st_mode) == mode
            and (stat.S_ISDIR(metadata.st_mode) if directory else stat.S_ISREG(metadata.st_mode)))


def read_private(home: Path, kind: str = "readiness", *, uid: int | None = None) -> dict:
    """Read the fixed owner path with descriptor-relative no-follow checks.

    The Home root is supplied by the caller, never by a path-bearing CLI flag.
    Ancestor directories must be owned, non-symlink directories without group or
    other write permission; the service directory/file have exact 0700/0600 modes.
    """
    if kind not in _SCHEMA:
        raise ProtocolError(Failure.INVALID_DOCUMENT)
    owner = os.getuid() if uid is None else uid
    directory_flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    descriptors = []
    try:
        current = os.open(home, directory_flags)
        descriptors.append(current)
        for component in (None, ".local", "share", "abbey-bot"):
            if component is not None:
                current = os.open(component, directory_flags, dir_fd=current)
                descriptors.append(current)
            meta = os.fstat(current)
            if (meta.st_uid != owner or not stat.S_ISDIR(meta.st_mode)
                    or stat.S_IMODE(meta.st_mode) & 0o022):
                raise ProtocolError(Failure.UNSAFE_FILE)
            if component == "abbey-bot" and not _safe(meta, owner, 0o700, True):
                raise ProtocolError(Failure.UNSAFE_FILE)
        filename = "readiness.json" if kind == "readiness" else "bootstrap-status.json"
        handle = os.open(filename, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=current)
        descriptors.append(handle)
        before = os.fstat(handle)
        if not _safe(before, owner, 0o600, False):
            raise ProtocolError(Failure.UNSAFE_FILE)
        limit = _SCHEMA[kind]["max_bytes"]
        if before.st_size > limit:
            raise ProtocolError(Failure.INVALID_DOCUMENT)
        raw = bytearray()
        while len(raw) <= limit:
            chunk = os.read(handle, limit + 1 - len(raw))
            if not chunk:
                break
            raw.extend(chunk)
        after = os.fstat(handle)
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
                after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns):
            raise ProtocolError(Failure.UNAVAILABLE)
        result = parse_document(bytes(raw), kind)
    except OSError as error:
        category = Failure.UNSAFE_FILE if error.errno in (errno.ELOOP, errno.ENOTDIR) else Failure.UNAVAILABLE
        raise ProtocolError(category) from None
    finally:
        cleanup_failed = False
        for handle in reversed(descriptors):
            try:
                os.close(handle)
            except OSError:
                # Do not retry a descriptor number that the OS may have reused.
                cleanup_failed = True
    if cleanup_failed:
        raise ProtocolError(Failure.UNAVAILABLE) from None
    return result
