#!/usr/bin/env python3
"""Behavior tests for Abbey's vendored Program 1 corpus guard."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/check-abbey-contracts.py"
PINNED_REVISION = "348754bdaaf59a40fbb858380f925e0aba95a23b"
PINNED_DIGEST = "72e241e34967df318376bf68f4a0e2db13f5ebf17d1a219709731f1f470dbe8e"


def source_corpus() -> Path:
    vendored = ROOT / "contracts/abbey/corpus"
    if vendored.is_dir():
        return vendored
    configured = os.environ.get("ABBEY_CONTRACT_SOURCE", "").strip()
    if configured:
        return Path(configured)
    raise RuntimeError("set ABBEY_CONTRACT_SOURCE for the initial RED run")


class AbbeyContractGuardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="abbey-contract-guard-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "contracts" / "abbey"
        self.root.parent.mkdir(parents=True)
        shutil.copytree(source_corpus(), self.root / "corpus")
        self.write_lock()

    def write_lock(self, *, revision: str = PINNED_REVISION, digest: str = PINNED_DIGEST) -> None:
        lock = {
            "source_repository": "https://github.com/donaldfilimon/abi",
            "source_revision": revision,
            "contract_major": 1,
            "contract_revision": 1,
            "aggregate_digest": digest,
        }
        (self.root / "abbey-contracts.lock.json").write_text(
            json.dumps(lock, indent=2) + "\n",
            encoding="utf-8",
        )

    def run_check(self, root: Path | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(root or self.root)],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def assert_closed_failure(
        self,
        result: subprocess.CompletedProcess[str],
        reason: str,
        relative_path: str,
    ) -> None:
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertEqual(
            result.stderr,
            f"abbey-contracts-check: {reason}: {relative_path}\n",
        )
        self.assertNotIn(str(self.temporary.name), result.stderr)

    def test_exact_pinned_corpus_passes(self) -> None:
        result = self.run_check()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertEqual(
            result.stdout,
            "abbey-contracts-check: verified 81 artifacts (88328 bytes), "
            f"digest={PINNED_DIGEST}\n",
        )

    def test_autocrlf_checkout_preserves_exact_vendored_corpus_bytes(self) -> None:
        source = Path(self.temporary.name) / "source"
        checkout = Path(self.temporary.name) / "checkout"
        (source / "contracts").mkdir(parents=True)
        shutil.copy2(ROOT / ".gitattributes", source / ".gitattributes")
        shutil.copytree(ROOT / "contracts/abbey", source / "contracts/abbey")

        commands = (
            ("init", "--quiet"),
            ("config", "user.name", "Abbey contract test"),
            ("config", "user.email", "abbey-contract-test@example.invalid"),
            ("add", "."),
            ("commit", "--quiet", "-m", "fixture"),
        )
        for command in commands:
            subprocess.run(
                ["git", *command],
                cwd=source,
                capture_output=True,
                text=True,
                check=True,
            )
        subprocess.run(
            [
                "git",
                "clone",
                "--quiet",
                "-c",
                "core.autocrlf=true",
                str(source),
                str(checkout),
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=True,
        )

        result = self.run_check(checkout / "contracts/abbey")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")

    def test_missing_corpus_fails_closed(self) -> None:
        missing_root = Path(self.temporary.name) / "missing" / "abbey"
        missing_root.mkdir(parents=True)
        lock = self.root / "abbey-contracts.lock.json"
        shutil.copy2(lock, missing_root / lock.name)

        result = self.run_check(missing_root)

        self.assert_closed_failure(result, "corpus_missing", "corpus")

    def test_changed_artifact_byte_reports_only_relative_path(self) -> None:
        artifact = self.root / "corpus/README.md"
        changed = bytearray(artifact.read_bytes())
        changed[0] ^= 1
        artifact.write_bytes(changed)

        result = self.run_check()

        self.assert_closed_failure(result, "artifact_digest_mismatch", "corpus/README.md")

    def test_extra_artifact_is_rejected(self) -> None:
        extra = self.root / "corpus/v1/fixtures/valid/consumer-extra.json"
        extra.write_text("{}\n", encoding="utf-8")

        result = self.run_check()

        self.assert_closed_failure(
            result,
            "corpus_inventory_mismatch",
            "corpus/v1/fixtures/valid/consumer-extra.json",
        )

    def test_lock_revision_mismatch_is_rejected(self) -> None:
        self.write_lock(revision="0" * 40)

        result = self.run_check()

        self.assert_closed_failure(
            result,
            "lock_source_revision_mismatch",
            "abbey-contracts.lock.json",
        )

    def test_invalid_manifest_path_is_not_echoed_before_normalization(self) -> None:
        manifest_path = self.root / "corpus/manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        private_path = "/Users/private/DO-NOT-ECHO-MANIFEST-PATH"
        manifest["artifacts"][0]["path"] = private_path
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")

        result = self.run_check()

        self.assert_closed_failure(
            result,
            "manifest_path_invalid",
            "corpus/manifest.json",
        )
        self.assertNotIn(private_path, result.stderr)

    def test_private_sentinel_outside_privacy_taxonomy_is_rejected_without_echo(self) -> None:
        fixture = self.root / "corpus/v1/fixtures/valid/authorization-grant.json"
        wrapper = json.loads(fixture.read_text(encoding="utf-8"))
        wrapper["document"]["token"] = "DO-NOT-ECHO-PRIVATE-SENTINEL"
        fixture.write_text(json.dumps(wrapper, indent=2) + "\n", encoding="utf-8")

        result = self.run_check()

        self.assert_closed_failure(
            result,
            "privacy_taxonomy_mismatch",
            "corpus/v1/fixtures/valid/authorization-grant.json",
        )
        self.assertNotIn("DO-NOT-ECHO-PRIVATE-SENTINEL", result.stderr)


if __name__ == "__main__":
    unittest.main()
