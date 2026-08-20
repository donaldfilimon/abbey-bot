#!/usr/bin/env python3
"""Verify every requirement stanza in generated deployment locks has a hash."""

from __future__ import annotations

import pathlib
import re
import sys


HASH_TOKEN = re.compile(r"--hash=sha256:[0-9a-f]{64}")
ANY_HASH_TOKEN = re.compile(r"--hash=[^ \\\t]+")


def fail(message: str) -> None:
    raise SystemExit(message)


def verify(path: pathlib.Path) -> tuple[int, int]:
    if not path.is_file():
        fail(f"missing Python lock: {path}")
    requirements = 0
    hashes = 0
    current: str | None = None
    current_hashed = False
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.rstrip()
        if not line or line.lstrip().startswith("#"):
            continue
        if "#" in line:
            fail(f"{path}:{number}: inline comments are forbidden in generated locks: {line}")
        stripped = line.lstrip()
        valid_hashes = HASH_TOKEN.findall(line)
        hash_tokens = ANY_HASH_TOKEN.findall(line)
        if len(valid_hashes) != len(hash_tokens) or any(
            HASH_TOKEN.fullmatch(token) is None for token in hash_tokens
        ):
            fail(f"{path}:{number}: malformed or non-SHA-256 hash token: {line}")
        if not line[0].isspace() and not line.startswith("--hash="):
            if stripped.startswith("-"):
                fail(f"{path}:{number}: generated-lock directives are forbidden: {line}")
            if current is not None and not current_hashed:
                fail(f"{path}: requirement lacks a SHA-256 hash: {current}")
            current = line.removesuffix(" \\")
            current_hashed = bool(valid_hashes)
            requirements += 1
        elif valid_hashes:
            if current is None:
                fail(f"{path}:{number}: hash appears before a requirement")
            residue = HASH_TOKEN.sub("", stripped).replace("\\", "").strip()
            if residue:
                fail(f"{path}:{number}: unexpected hash-continuation content: {line}")
            current_hashed = True
        else:
            fail(f"{path}:{number}: unexpected generated-lock continuation: {line}")
        hashes += len(valid_hashes)
    if current is not None and not current_hashed:
        fail(f"{path}: requirement lacks a SHA-256 hash: {current}")
    if requirements == 0 or hashes == 0:
        fail(f"{path}: lock contains no hashed requirements")
    return requirements, hashes


def main() -> None:
    if len(sys.argv) < 2:
        fail("usage: check-python-locks.py LOCK [LOCK ...]")
    for name in sys.argv[1:]:
        path = pathlib.Path(name)
        requirements, hashes = verify(path)
        print(f"{path}: {requirements} requirements, {hashes} SHA-256 hashes")


if __name__ == "__main__":
    main()
