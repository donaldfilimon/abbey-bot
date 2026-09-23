#!/usr/bin/env python3
"""Verify every requirement stanza in generated deployment locks has a hash,
and that Darwin-only companion packages are pinned completely.

On 2026-09-16 a Dependabot mlx-vlm bump regenerated
deploy/mlx-vlm-requirements.txt without mlx-metal, which mlx requires on
Darwin. The Mac install broke for a day while this checker, which validated
hashes only, stayed green. DARWIN_COMPANIONS below pins the completeness rule
that would have caught it: whenever a key package is pinned with `==`, its
listed companion must be pinned at the identical version (and, like every
other requirement, carry hashes).
"""

from __future__ import annotations

import pathlib
import re
import sys


HASH_TOKEN = re.compile(r"--hash=sha256:[0-9a-f]{64}")
ANY_HASH_TOKEN = re.compile(r"--hash=[^ \\\t]+")
REQUIREMENT_NAME_VERSION = re.compile(
    r"^([A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?)(?:\[[^\]]*\])?==([^\s;]+)"
)

# Package name (PEP 503 normalized, lowercase) -> Darwin companion package
# that must be pinned at the same version whenever the key package appears.
DARWIN_COMPANIONS = {"mlx": "mlx-metal"}


def fail(message: str) -> None:
    raise SystemExit(message)


def normalize_name(name: str) -> str:
    """PEP 503 normalization: fold runs of -/_/. and lowercase."""
    return re.sub(r"[-_.]+", "-", name).lower()


def verify(path: pathlib.Path) -> tuple[int, int]:
    if not path.is_file():
        fail(f"missing Python lock: {path}")
    requirements = 0
    hashes = 0
    current: str | None = None
    current_hashed = False
    pinned: dict[str, tuple[str, str]] = {}
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
            match = REQUIREMENT_NAME_VERSION.match(current)
            if match is not None:
                name, version = match.group(1), match.group(2)
                # Last-write-wins if the same normalized name is pinned more
                # than once in one lock (real uv-generated locks never do
                # this: each package appears in exactly one stanza). Not
                # rejected here because doing so is outside this rule's
                # scope and would be untested behavior.
                pinned[normalize_name(name)] = (name, version)
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
    for key, companion in DARWIN_COMPANIONS.items():
        if key not in pinned:
            continue
        key_name, key_version = pinned[key]
        companion_key = normalize_name(companion)
        if companion_key not in pinned:
            fail(
                f"{path}: {key_name}=={key_version} requires a pinned, hashed "
                f"{companion}=={key_version} requirement on Darwin "
                f"({companion} completeness rule, 2026-09-16 incident: "
                f"{companion} was dropped from a regenerated lock and the "
                f"Mac install broke while hash validation alone stayed green)"
            )
        companion_name, companion_version = pinned[companion_key]
        if companion_version != key_version:
            fail(
                f"{path}: Darwin companion version mismatch: "
                f"{key_name}=={key_version} pins against {companion_name}=="
                f"{companion_version}, but Darwin companions must match "
                f"exactly (expected {companion}=={key_version})"
            )
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
