#!/usr/bin/env python3
"""Compare Abbey's WDBX-v1 projection fixture with WDBX's canonical copy."""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parent.parent
ABBEY_FIXTURE = ROOT / "tests/fixtures/wdbx_v1_conformance.seg.jsonl"
WDBX_RELATIVE_FIXTURE = Path(
    "crates/abi-wdbx/tests/golden/abbey-bot-projection.seg.jsonl"
)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    configured = os.environ.get("ABBEY_WDBX_REPO", "").strip()
    wdbx_root = Path(configured).expanduser() if configured else ROOT.parent / "wdbx"
    wdbx_fixture = wdbx_root / WDBX_RELATIVE_FIXTURE
    required = os.environ.get("ABBEY_REQUIRE_WDBX_CONFORMANCE", "").strip() == "1"

    if not ABBEY_FIXTURE.is_file():
        print(f"missing Abbey WDBX fixture: {ABBEY_FIXTURE}", file=sys.stderr)
        return 1
    if not wdbx_fixture.is_file():
        message = (
            "external WDBX fixture unavailable; repository-local writer pin remains active "
            f"(looked for {wdbx_fixture})"
        )
        if required:
            print(message, file=sys.stderr)
            print(
                "set ABBEY_WDBX_REPO to the canonical WDBX checkout or unset "
                "ABBEY_REQUIRE_WDBX_CONFORMANCE",
                file=sys.stderr,
            )
            return 1
        print(f"SKIP: {message}")
        return 0

    abbey = ABBEY_FIXTURE.read_bytes()
    wdbx = wdbx_fixture.read_bytes()
    if abbey != wdbx:
        print("WDBX conformance fixtures diverged", file=sys.stderr)
        print(f"Abbey: {ABBEY_FIXTURE} sha256={digest(abbey)}", file=sys.stderr)
        print(f"WDBX:  {wdbx_fixture} sha256={digest(wdbx)}", file=sys.stderr)
        return 1

    print(
        "WDBX cross-repository fixture parity: "
        f"sha256={digest(abbey)} ({wdbx_fixture})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
