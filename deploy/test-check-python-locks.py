#!/usr/bin/env python3
"""Unit coverage for deploy/check-python-locks.py.

Includes the Darwin companion-completeness rule: on 2026-09-16 a Dependabot
mlx-vlm bump regenerated deploy/mlx-vlm-requirements.txt without mlx-metal,
which mlx requires on Darwin, and the Mac install broke for a day while this
checker (hash validation only) stayed green. These tests cover both the
pre-existing hash rules and the new companion-completeness rule.
"""
from __future__ import annotations

import importlib.util
import pathlib
import tempfile

SCRIPT = pathlib.Path(__file__).with_name("check-python-locks.py")
spec = importlib.util.spec_from_file_location("check_python_locks", SCRIPT)
mod = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(mod)


HASH_A = "a" * 64
HASH_B = "b" * 64


def stanza(name: str, version: str) -> str:
    return (
        f"{name}=={version} \\\n"
        f"    --hash=sha256:{HASH_A} \\\n"
        f"    --hash=sha256:{HASH_B}\n"
    )


def write_lock(directory: pathlib.Path, name: str, body: str) -> pathlib.Path:
    path = directory / name
    path.write_text(body, encoding="utf-8")
    return path


def test_real_repo_locks_pass() -> None:
    repo_root = SCRIPT.parent.parent
    for name in (
        "deploy/mlx-vlm-requirements.txt",
        "deploy/mlx-audio-requirements.txt",
        "deploy/mlx-audio-build-constraints.txt",
    ):
        requirements, hashes = mod.verify(repo_root / name)
        assert requirements > 0
        assert hashes > 0


def test_lock_with_neither_mlx_nor_companion_passes() -> None:
    with tempfile.TemporaryDirectory() as directory:
        lock = write_lock(
            pathlib.Path(directory),
            "neither.txt",
            stanza("requests", "2.32.3"),
        )
        requirements, hashes = mod.verify(lock)
        assert requirements == 1
        assert hashes == 2


def test_mlx_without_companion_fails_naming_lock_and_companion() -> None:
    with tempfile.TemporaryDirectory() as directory:
        lock = write_lock(
            pathlib.Path(directory),
            "missing-companion.txt",
            stanza("mlx", "0.32.2"),
        )
        try:
            mod.verify(lock)
        except SystemExit as error:
            message = str(error)
        else:
            raise AssertionError("expected SystemExit for a missing mlx-metal companion")
        assert str(lock) in message, message
        assert "mlx-metal==0.32.2" in message, message


def test_mlx_metal_version_mismatch_fails_naming_both_versions() -> None:
    with tempfile.TemporaryDirectory() as directory:
        body = stanza("mlx", "0.32.2") + stanza("mlx-metal", "0.32.1")
        lock = write_lock(pathlib.Path(directory), "mismatch.txt", body)
        try:
            mod.verify(lock)
        except SystemExit as error:
            message = str(error)
        else:
            raise AssertionError(
                "expected SystemExit for a Darwin companion version mismatch"
            )
        assert "mlx==0.32.2" in message, message
        assert "mlx-metal==0.32.1" in message, message


def test_mlx_with_matching_companion_passes() -> None:
    with tempfile.TemporaryDirectory() as directory:
        body = stanza("mlx", "0.32.2") + stanza("mlx-metal", "0.32.2")
        lock = write_lock(pathlib.Path(directory), "matching.txt", body)
        requirements, hashes = mod.verify(lock)
        assert requirements == 2
        assert hashes == 4


def test_companion_name_normalization_is_pep503() -> None:
    # PEP 503: names compare case-insensitively with '-', '_', '.' runs folded.
    with tempfile.TemporaryDirectory() as directory:
        body = stanza("MLX", "0.32.2") + stanza("mlx_metal", "0.32.2")
        lock = write_lock(pathlib.Path(directory), "normalized.txt", body)
        requirements, hashes = mod.verify(lock)
        assert requirements == 2
        assert hashes == 4


def test_stanza_without_hash_still_fails() -> None:
    with tempfile.TemporaryDirectory() as directory:
        lock = write_lock(pathlib.Path(directory), "unhashed.txt", "requests==2.32.3\n")
        try:
            mod.verify(lock)
        except SystemExit as error:
            message = str(error)
        else:
            raise AssertionError("expected SystemExit for a requirement lacking a hash")
        assert "lacks a SHA-256 hash" in message, message


if __name__ == "__main__":
    test_real_repo_locks_pass()
    test_lock_with_neither_mlx_nor_companion_passes()
    test_mlx_without_companion_fails_naming_lock_and_companion()
    test_mlx_metal_version_mismatch_fails_naming_both_versions()
    test_mlx_with_matching_companion_passes()
    test_companion_name_normalization_is_pep503()
    test_stanza_without_hash_still_fails()
    print("check-python-locks tests passed")
