#!/usr/bin/env python3
"""Regression tests for the secret-preserving MLX environment cutover."""

from __future__ import annotations

import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("configure-mlx-primary.py")
REVISION = "73bcf09092aa277861d5a191b989b666f7f32e8f"


@unittest.skipUnless(hasattr(os, "getuid"), "the launchd cutover is POSIX-only")
class ConfigureMlxPrimaryTests(unittest.TestCase):
    def fixture(self, root: Path) -> tuple[Path, Path, Path, Path]:
        env = root / "env"
        env.write_text(
            "DISCORD_TOKEN=do-not-print-this\n"
            "export ANTHROPIC_API_KEY=preserve-but-deactivate-this\n"
            "ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434\n"
            "ABBEY_BOT_LLM_MODEL=gpt-oss:20b\n",
            encoding="utf-8",
        )
        env.chmod(0o600)
        manifest = root / "fm-capabilities.json"
        manifest.write_text('{"schema_version":1}\n', encoding="utf-8")
        manifest.chmod(0o600)
        model = (
            root
            / "models--mlx-community--gemma-4-12B-it-4bit"
            / "snapshots"
            / REVISION
        )
        model.mkdir(parents=True)
        (model / "model.safetensors.index.json").write_text("{}", encoding="utf-8")
        backups = root / "backups"
        return env, manifest, model, backups

    def run_script(self, *arguments: object) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), *(str(value) for value in arguments)],
            check=False,
            capture_output=True,
            text=True,
        )

    def test_dry_run_prints_only_key_names_and_changes_nothing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            env, manifest, model, backups = self.fixture(Path(directory))
            before = env.read_bytes()
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--backup-dir",
                backups,
                "--dry-run",
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn("do-not-print-this", result.stdout + result.stderr)
            self.assertNotIn(
                "preserve-but-deactivate-this", result.stdout + result.stderr
            )
            self.assertEqual(env.read_bytes(), before)
            self.assertFalse(backups.exists())

    def test_publish_preserves_secrets_and_retains_private_backup(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            env, manifest, model, backups = self.fixture(Path(directory))
            before = env.read_bytes()
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--backup-dir",
                backups,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn("do-not-print-this", result.stdout + result.stderr)
            self.assertNotIn(
                "preserve-but-deactivate-this", result.stdout + result.stderr
            )
            updated = env.read_text(encoding="utf-8")
            self.assertIn("DISCORD_TOKEN=do-not-print-this", updated)
            self.assertNotRegex(
                updated,
                r"(?m)^(?:export[ \t]+)?ANTHROPIC_API_KEY=",
            )
            self.assertIn(
                "# export ANTHROPIC_API_KEY=preserve-but-deactivate-this", updated
            )
            self.assertIn("ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:8282", updated)
            self.assertIn(f"ABBEY_BOT_LLM_MODEL={model.resolve()}", updated)
            self.assertNotIn("ABBEY_FM_ENDPOINT=", updated)
            self.assertEqual(stat.S_IMODE(env.stat().st_mode), 0o600)
            copies = list(backups.iterdir())
            self.assertEqual(len(copies), 1)
            self.assertEqual(copies[0].read_bytes(), before)
            self.assertEqual(stat.S_IMODE(copies[0].stat().st_mode), 0o600)

    def test_symlink_environment_is_rejected_without_touching_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups = self.fixture(root)
            target = env.read_bytes()
            link = root / "env-link"
            link.symlink_to(env)
            result = self.run_script(
                "--env-file",
                link,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--backup-dir",
                backups,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("not a symlink", result.stderr)
            self.assertEqual(env.read_bytes(), target)
            self.assertFalse(backups.exists())


if __name__ == "__main__":
    unittest.main()
