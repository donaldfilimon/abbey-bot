#!/usr/bin/env python3
"""Exercise the actual offline evaluator without credentials, providers or stores."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class StartupTests(unittest.TestCase):
    binary: Path
    fixture = Path(__file__).resolve().parents[1] / "tests/fixtures/learning-quality-v1.json"

    def invoke(self, scratch: Path, corpus: Path) -> subprocess.CompletedProcess[bytes]:
        environment = {key: os.environ[key] for key in ("PATH", "SystemRoot", "SYSTEMROOT", "WINDIR") if key in os.environ}
        environment.update({
            "RUST_LOG": "trace",
            "ABBEY_DATA_DIR": str(scratch / "canonical-state"),
            "ABBEY_PROVIDER_STATE_DIR": str(scratch / "provider-state"),
            "ABBEY_EPISODE_GATE_CONFIG": str(scratch / "missing-episode-config"),
            "ABBEY_GUILD_ID": "invalid-guild-must-not-be-read",
        })
        result = subprocess.run(
            [str(self.binary), "--learning-quality", str(corpus), "--json"],
            cwd=scratch, env=environment, capture_output=True, timeout=15, check=False,
        )
        self.assertFalse((scratch / "canonical-state").exists())
        self.assertFalse((scratch / "provider-state").exists())
        return result

    def test_token_free_startup_emits_reproducible_closed_report(self) -> None:
        with tempfile.TemporaryDirectory(prefix="abbey-learning-cli-") as temporary:
            scratch = Path(temporary)
            first = self.invoke(scratch, self.fixture)
            second = self.invoke(scratch, self.fixture)
            self.assertEqual(first.returncode, 0, first.stderr.decode())
            self.assertEqual(second.returncode, 0, second.stderr.decode())
            self.assertEqual(first.stdout, second.stdout)
            self.assertEqual(first.stderr, b"")
            report = json.loads(first.stdout)
            self.assertEqual(report["quality"]["total"], {
                "total": 100, "true_positive": 30, "false_positive": 0,
                "true_negative": 70, "false_negative": 0,
            })
            self.assertEqual(len(report["quality"]["by_class"]), 5)
            self.assertTrue(all(row["total"] == 20 for row in report["quality"]["by_class"].values()))
            self.assertEqual(report["corpus_sha256"], hashlib.sha256(self.fixture.read_bytes()).hexdigest())
            self.assertEqual(len(report["evaluator_source_sha256"]), 64)
            self.assertIn("pending independent human adjudication", report["label_provenance"])
            for forbidden in (b"Amber", b"source-01", b"cited_source_ids", str(scratch).encode()):
                self.assertNotIn(forbidden, first.stdout + first.stderr)
            print(json.dumps(report, sort_keys=True))

    def test_invalid_and_oversized_corpora_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="abbey-learning-invalid-") as temporary:
            scratch = Path(temporary)
            corpus = scratch / "private-fixture.json"
            for payload in (b"PRIVATE_SYNTHETIC_INVALID_JSON", b"x" * (1024 * 1024 + 1)):
                corpus.write_bytes(payload)
                result = self.invoke(scratch, corpus)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, b"")
                self.assertNotIn(b"PRIVATE_SYNTHETIC", result.stderr)
                self.assertNotIn(str(corpus).encode(), result.stderr)

    def test_false_positives_are_reported_with_failure_exit(self) -> None:
        with tempfile.TemporaryDirectory(prefix="abbey-learning-negative-") as temporary:
            scratch = Path(temporary)
            data = json.loads(self.fixture.read_bytes())
            data["cases"][0]["expected"] = "Abstain"
            corpus = scratch / "adjudicated-negative.json"
            corpus.write_text(json.dumps(data), encoding="utf-8")
            result = self.invoke(scratch, corpus)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(json.loads(result.stdout)["quality"]["total"]["false_positive"], 1)

    def test_dotted_source_is_refused_before_grounding_expansion(self) -> None:
        with tempfile.TemporaryDirectory(prefix="abbey-learning-long-token-") as temporary:
            scratch = Path(temporary)
            data = json.loads(self.fixture.read_bytes())
            data["cases"][0]["sources"][0]["text"] = "v" + ".".join(["1"] * 20000)
            corpus = scratch / "synthetic-long-token.json"
            corpus.write_text(json.dumps(data), encoding="utf-8")
            self.assertLess(corpus.stat().st_size, 1024 * 1024)
            result = self.invoke(scratch, corpus)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, b"")
            self.assertEqual(result.stderr, b"quality_text_limit\n")

    @unittest.skipUnless(hasattr(os, "mkfifo"), "POSIX FIFO regression")
    def test_fifo_input_is_refused_without_waiting_for_a_writer(self) -> None:
        with tempfile.TemporaryDirectory(prefix="abbey-learning-fifo-") as temporary:
            scratch = Path(temporary)
            corpus = scratch / "synthetic-fifo"
            os.mkfifo(corpus)
            result = self.invoke(scratch, corpus)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, b"")
            self.assertEqual(result.stderr, b"corpus_requires_regular_file_at_most_1_mib\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    StartupTests.binary = args.binary.resolve(strict=True)
    unittest.main(argv=[__file__])
