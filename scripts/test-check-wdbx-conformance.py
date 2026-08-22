#!/usr/bin/env python3
"""Regression tests for the cross-repository WDBX fixture checker."""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/check-wdbx-conformance.py"
ABBEY_FIXTURE = ROOT / "tests/fixtures/wdbx_v1_conformance.seg.jsonl"
WDBX_RELATIVE_FIXTURE = Path(
    "crates/abi-wdbx/tests/golden/abbey-bot-projection.seg.jsonl"
)


class WdbxConformanceCheckTests(unittest.TestCase):
    def run_check(self, wdbx_root: Path) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        environment["ABBEY_WDBX_REPO"] = str(wdbx_root)
        environment["ABBEY_REQUIRE_WDBX_CONFORMANCE"] = "1"
        return subprocess.run(
            [sys.executable, str(SCRIPT)],
            cwd=ROOT,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_matching_fixture_passes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            wdbx_root = Path(temporary)
            target = wdbx_root / WDBX_RELATIVE_FIXTURE
            target.parent.mkdir(parents=True)
            target.write_bytes(ABBEY_FIXTURE.read_bytes())
            result = self.run_check(wdbx_root)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("WDBX cross-repository fixture parity", result.stdout)

    def test_divergent_fixture_fails_with_both_hashes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            wdbx_root = Path(temporary)
            target = wdbx_root / WDBX_RELATIVE_FIXTURE
            target.parent.mkdir(parents=True)
            target.write_bytes(ABBEY_FIXTURE.read_bytes() + b"drift\n")
            result = self.run_check(wdbx_root)
        self.assertEqual(result.returncode, 1)
        self.assertIn("fixtures diverged", result.stderr)
        self.assertEqual(result.stderr.count("sha256="), 2)

    def test_required_missing_fixture_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result = self.run_check(Path(temporary))
        self.assertEqual(result.returncode, 1)
        self.assertIn("external WDBX fixture unavailable", result.stderr)


if __name__ == "__main__":
    unittest.main()
