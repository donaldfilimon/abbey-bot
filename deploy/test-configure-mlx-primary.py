#!/usr/bin/env python3
"""Regression tests for the secret-preserving MLX environment cutover."""

from __future__ import annotations

import copy
import hashlib
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest


SCRIPT = Path(__file__).with_name("configure-mlx-primary.py")
REVISION = "73bcf09092aa277861d5a191b989b666f7f32e8f"
FIXTURE = "abbey-provider-fixtures-v1"
FM_CLI = Path("/usr/bin/fm")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def passed_capabilities() -> dict[str, dict[str, str]]:
    return {
        name: {"status": "pass"}
        for name in ("text", "streaming", "structured_output", "tools", "vision", "ocr")
    }


def skipped_entry() -> dict[str, object]:
    return {
        "configured": False,
        "capabilities": {
            name: {"status": "skipped"} for name in passed_capabilities()
        },
    }


@unittest.skipUnless(
    sys.platform == "darwin" and FM_CLI.is_file(),
    "the qualified Foundation Models cutover is macOS-only",
)
class ConfigureMlxPrimaryTests(unittest.TestCase):
    def fixture(self, root: Path) -> tuple[Path, Path, Path, Path, Path]:
        env = root / "env"
        env.write_text(
            "DISCORD_TOKEN=do-not-print-this\n"
            "export ANTHROPIC_API_KEY=preserve-but-deactivate-this\n"
            "ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434\n"
            "ABBEY_BOT_LLM_MODEL=gpt-oss:20b\n"
            "ABBEY_BOT_LLM_TOOLS=off\n"
            "ABBEY_VISION_KEY=preserve-vision-secret\n"
            "ABBEY_FM_ENDPOINT=http://127.0.0.1:1976\n"
            "ABBEY_FM_CLI=/tmp/old-fm\n",
            encoding="utf-8",
        )
        env.chmod(0o600)
        model = (
            root
            / "models--mlx-community--gemma-4-12B-it-4bit"
            / "snapshots"
            / REVISION
        )
        model.mkdir(parents=True)
        (model / "model.safetensors.index.json").write_text("{}", encoding="utf-8")
        binary = root / "abbey-bot"
        binary.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        binary.chmod(0o700)
        binary_hash = sha256(binary)
        os_build = subprocess.run(
            ["/usr/bin/sw_vers", "-buildVersion"],
            check=True,
            capture_output=True,
            text=True,
            env={},
        ).stdout.strip()
        primary_identity = {
            "endpoint": "http://127.0.0.1:8282",
            "model": str(model.resolve()),
            "abbey_binary_sha256": binary_hash,
            "os_build": os_build,
            "fixture_version": FIXTURE,
        }
        vision_identity = {
            **primary_identity,
            "endpoint": "http://127.0.0.1:8282/v1",
        }
        fm_identity = {
            "cli_path": str(FM_CLI),
            "cli_sha256": sha256(FM_CLI),
            "mode": "system",
            "abbey_binary_sha256": binary_hash,
            "os_build": os_build,
            "fixture_version": FIXTURE,
        }
        manifest = root / "fm-capabilities.json"
        manifest.write_text(
            json.dumps(
                {
                    "version": 1,
                    "fixture_version": FIXTURE,
                    "generated_unix_secs": int(time.time()),
                    "target": "all",
                    "overall_pass": True,
                    "primary": {
                        "configured": True,
                        "identity": primary_identity,
                        "vision_identity": vision_identity,
                        "capabilities": passed_capabilities(),
                    },
                    "fm_server": skipped_entry(),
                    "fm_cli": {
                        "configured": True,
                        "identity": fm_identity,
                        "vision_identity": fm_identity,
                        "capabilities": passed_capabilities(),
                    },
                }
            )
            + "\n",
            encoding="utf-8",
        )
        manifest.chmod(0o600)
        backups = root / "backups"
        return env, manifest, model, backups, binary

    def fake_launchctl(self, root: Path) -> tuple[Path, Path]:
        executable = root / "launchctl"
        executable.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, pathlib, sys, time\n"
            "state_path = pathlib.Path(os.environ['FAKE_LAUNCHCTL_STATE'])\n"
            "state = json.loads(state_path.read_text())\n"
            "if sys.argv[1] == 'print':\n"
            "    print('ABBEY_FAKE_SECRET=launchctl-do-not-print')\n"
            "    print(f\"pid = {state['pid']}\")\n"
            "    raise SystemExit(0)\n"
            "if sys.argv[1:3] != ['kickstart', '-k']:\n"
            "    raise SystemExit(2)\n"
            "state['kickstarts'] += 1\n"
            "mode = os.environ.get('FAKE_LAUNCHCTL_MODE', 'success')\n"
            "if mode == 'wait':\n"
            "    pathlib.Path(os.environ['FAKE_LAUNCHCTL_READY']).write_text('ready')\n"
            "    release = pathlib.Path(os.environ['FAKE_LAUNCHCTL_RELEASE'])\n"
            "    while not release.exists():\n"
            "        time.sleep(0.02)\n"
            "if mode == 'fail_all' or (mode == 'fail_once' and state['kickstarts'] == 1):\n"
            "    state_path.write_text(json.dumps(state))\n"
            "    raise SystemExit(1)\n"
            "state['pid'] += 101\n"
            "state_path.write_text(json.dumps(state))\n"
            "poison_lock = os.environ.get('FAKE_LAUNCHCTL_POISON_LOCK')\n"
            "if poison_lock:\n"
            "    (pathlib.Path(poison_lock) / 'release-blocker').write_text('retain')\n"
            "replace_owner = os.environ.get('FAKE_LAUNCHCTL_REPLACE_LOCK_OWNER')\n"
            "if replace_owner:\n"
            "    (pathlib.Path(replace_owner) / 'pid').write_text('999999\\n')\n"
            "raise SystemExit(0)\n",
            encoding="utf-8",
        )
        executable.chmod(0o700)
        state_file = root / "launchctl-state.json"
        state_file.write_text(json.dumps({"pid": 101, "kickstarts": 0}))
        return executable, state_file

    def script_command(self, *arguments: object) -> list[str]:
        values = [str(value) for value in arguments]
        if "--install-lock" not in values:
            env_index = values.index("--env-file") + 1
            values.extend(("--install-lock", str(Path(values[env_index]).parent / "install.lock")))
        return [sys.executable, str(SCRIPT), *values]

    def run_script(
        self, *arguments: object, environment: dict[str, str] | None = None
    ) -> subprocess.CompletedProcess[str]:
        child_environment = os.environ.copy()
        if environment:
            child_environment.update(environment)
        return subprocess.run(
            self.script_command(*arguments),
            check=False,
            capture_output=True,
            text=True,
            env=child_environment,
        )

    def assert_secret_free(self, result: subprocess.CompletedProcess[str]) -> None:
        output = result.stdout + result.stderr
        self.assertNotIn("do-not-print-this", output)
        self.assertNotIn("preserve-but-deactivate-this", output)
        self.assertNotIn("preserve-vision-secret", output)
        self.assertNotIn("launchctl-do-not-print", output)

    def test_dry_run_prints_only_key_names_and_changes_nothing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            env, manifest, model, backups, binary = self.fixture(Path(directory))
            before = env.read_bytes()
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--dry-run",
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assert_secret_free(result)
            self.assertEqual(env.read_bytes(), before)
            self.assertFalse(backups.exists())

    def test_apply_restarts_and_retains_private_backup_without_printing_secrets(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups, binary = self.fixture(root)
            launchctl, state_file = self.fake_launchctl(root)
            child_environment = {"FAKE_LAUNCHCTL_STATE": str(state_file)}
            before = env.read_bytes()
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--apply-and-restart",
                "--launchctl",
                launchctl,
                environment=child_environment,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assert_secret_free(result)
            self.assertEqual(json.loads(state_file.read_text())["pid"], 202)
            updated = env.read_text(encoding="utf-8")
            self.assertIn("DISCORD_TOKEN=do-not-print-this", updated)
            self.assertRegex(updated, r"(?m)^ANTHROPIC_API_KEY=''$")
            self.assertIn(
                "# export ANTHROPIC_API_KEY=preserve-but-deactivate-this", updated
            )
            self.assertRegex(updated, r"(?m)^ABBEY_VISION_KEY=''$")
            self.assertIn("# ABBEY_VISION_KEY=preserve-vision-secret", updated)
            self.assertIn("ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:8282", updated)
            self.assertIn(f"ABBEY_BOT_LLM_MODEL={model.resolve()}", updated)
            self.assertIn("ABBEY_BOT_LLM_TOOLS=on", updated)
            self.assertRegex(updated, r"(?m)^ABBEY_FM_ENDPOINT=''$")
            self.assertIn("ABBEY_FM_CLI=/usr/bin/fm", updated)
            self.assertEqual(stat.S_IMODE(env.stat().st_mode), 0o600)
            copies = list(backups.iterdir())
            self.assertEqual(len(copies), 1)
            self.assertEqual(copies[0].read_bytes(), before)
            self.assertEqual(stat.S_IMODE(copies[0].stat().st_mode), 0o600)

            first = env.read_bytes()
            repeated = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--apply-and-restart",
                "--launchctl",
                launchctl,
                environment=child_environment,
            )
            self.assertEqual(repeated.returncode, 0, repeated.stderr)
            self.assert_secret_free(repeated)
            self.assertEqual(env.read_bytes(), first)
            self.assertEqual(updated.count("Qualified local providers"), 1)

    def test_failed_candidate_restart_restores_exact_environment_and_old_service(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups, binary = self.fixture(root)
            launchctl, state_file = self.fake_launchctl(root)
            before = env.read_bytes()
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--apply-and-restart",
                "--launchctl",
                launchctl,
                environment={
                    "FAKE_LAUNCHCTL_STATE": str(state_file),
                    "FAKE_LAUNCHCTL_MODE": "fail_once",
                },
            )
            self.assertNotEqual(result.returncode, 0)
            self.assert_secret_free(result)
            self.assertIn("previous environment and service were restored", result.stderr)
            self.assertEqual(env.read_bytes(), before)
            copies = list(backups.iterdir())
            self.assertEqual(len(copies), 1)
            self.assertEqual(copies[0].read_bytes(), before)
            state = json.loads(state_file.read_text())
            self.assertEqual(state["kickstarts"], 2)
            self.assertEqual(state["pid"], 202)

    def test_failed_rollback_restart_fails_closed_and_retains_exact_backup(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups, binary = self.fixture(root)
            launchctl, state_file = self.fake_launchctl(root)
            before = env.read_bytes()
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--apply-and-restart",
                "--launchctl",
                launchctl,
                environment={
                    "FAKE_LAUNCHCTL_STATE": str(state_file),
                    "FAKE_LAUNCHCTL_MODE": "fail_all",
                },
            )
            self.assertNotEqual(result.returncode, 0)
            self.assert_secret_free(result)
            self.assertIn("previous environment was restored", result.stderr)
            self.assertIn("service restart failed", result.stderr)
            self.assertEqual(env.read_bytes(), before)
            copies = list(backups.iterdir())
            self.assertEqual(len(copies), 1)
            self.assertEqual(copies[0].read_bytes(), before)
            self.assertEqual(json.loads(state_file.read_text())["kickstarts"], 2)

    def test_plain_apply_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            env, manifest, model, backups, binary = self.fixture(Path(directory))
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
            )
            self.assertEqual(result.returncode, 2)
            self.assertFalse(backups.exists())

    def test_preexisting_install_lock_rejects_before_any_change(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups, binary = self.fixture(root)
            before = env.read_bytes()
            lock = root / "install.lock"
            lock.mkdir(mode=0o700)
            (lock / "pid").write_text("999999\n", encoding="ascii")
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--dry-run",
            )
            self.assertNotEqual(result.returncode, 0)
            self.assert_secret_free(result)
            self.assertIn("another Abbey install transaction holds", result.stderr)
            self.assertEqual(env.read_bytes(), before)
            self.assertEqual((lock / "pid").read_text(encoding="ascii"), "999999\n")
            self.assertFalse(backups.exists())

    def test_concurrent_configurator_cannot_enter_the_install_transaction(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups, binary = self.fixture(root)
            launchctl, state_file = self.fake_launchctl(root)
            ready = root / "launchctl-ready"
            release = root / "launchctl-release"
            child_environment = os.environ.copy()
            child_environment.update(
                {
                    "FAKE_LAUNCHCTL_STATE": str(state_file),
                    "FAKE_LAUNCHCTL_MODE": "wait",
                    "FAKE_LAUNCHCTL_READY": str(ready),
                    "FAKE_LAUNCHCTL_RELEASE": str(release),
                }
            )
            command = self.script_command(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--apply-and-restart",
                "--launchctl",
                launchctl,
            )
            first = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env=child_environment,
            )
            try:
                deadline = time.monotonic() + 5
                while not ready.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(ready.exists(), "first configurator did not reach restart")
                second = self.run_script(
                    "--env-file",
                    env,
                    "--model-dir",
                    model,
                    "--manifest",
                    manifest,
                    "--binary",
                    binary,
                    "--backup-dir",
                    backups,
                    "--dry-run",
                )
                self.assertNotEqual(second.returncode, 0)
                self.assert_secret_free(second)
                self.assertIn("another Abbey install transaction holds", second.stderr)
                release.write_text("continue\n", encoding="ascii")
                stdout, stderr = first.communicate(timeout=10)
            finally:
                if first.poll() is None:
                    first.kill()
                    first.communicate()
            first_result = subprocess.CompletedProcess(command, first.returncode, stdout, stderr)
            self.assertEqual(first_result.returncode, 0, first_result.stderr)
            self.assert_secret_free(first_result)
            self.assertFalse((root / "install.lock").exists())
            self.assertEqual(len(list(backups.iterdir())), 1)

    def test_pending_termination_is_delivered_only_after_lock_release(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups, binary = self.fixture(root)
            launchctl, state_file = self.fake_launchctl(root)
            ready = root / "signal-ready"
            release = root / "signal-release"
            child_environment = os.environ.copy()
            child_environment.update(
                {
                    "FAKE_LAUNCHCTL_STATE": str(state_file),
                    "FAKE_LAUNCHCTL_MODE": "wait",
                    "FAKE_LAUNCHCTL_READY": str(ready),
                    "FAKE_LAUNCHCTL_RELEASE": str(release),
                }
            )
            command = self.script_command(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--apply-and-restart",
                "--launchctl",
                launchctl,
            )
            process = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env=child_environment,
            )
            try:
                deadline = time.monotonic() + 5
                while not ready.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(ready.exists(), "configurator did not acquire the lock")
                process.send_signal(signal.SIGTERM)
                time.sleep(0.1)
                self.assertIsNone(process.poll(), "termination bypassed the transaction")
                release.write_text("continue\n", encoding="ascii")
                stdout, stderr = process.communicate(timeout=10)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.communicate()
            result = subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
            self.assert_secret_free(result)
            self.assertEqual(process.returncode, -signal.SIGTERM)
            self.assertFalse((root / "install.lock").exists())
            self.assertEqual(len(list(backups.iterdir())), 1)

    def test_release_failure_retains_owned_lock_and_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups, binary = self.fixture(root)
            launchctl, state_file = self.fake_launchctl(root)
            lock = root / "install.lock"
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--apply-and-restart",
                "--launchctl",
                launchctl,
                environment={
                    "FAKE_LAUNCHCTL_STATE": str(state_file),
                    "FAKE_LAUNCHCTL_POISON_LOCK": str(lock),
                },
            )
            self.assertNotEqual(result.returncode, 0)
            self.assert_secret_free(result)
            self.assertIn("owner record restored", result.stderr)
            self.assertTrue(lock.is_dir())
            self.assertTrue((lock / "pid").is_file())
            self.assertEqual(len(list(backups.iterdir())), 1)

    def test_tampered_lock_owner_is_never_released(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups, binary = self.fixture(root)
            launchctl, state_file = self.fake_launchctl(root)
            lock = root / "install.lock"
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--apply-and-restart",
                "--launchctl",
                launchctl,
                environment={
                    "FAKE_LAUNCHCTL_STATE": str(state_file),
                    "FAKE_LAUNCHCTL_REPLACE_LOCK_OWNER": str(lock),
                },
            )
            self.assertNotEqual(result.returncode, 0)
            self.assert_secret_free(result)
            self.assertIn("not owned by this process", result.stderr)
            self.assertTrue(lock.is_dir())
            self.assertEqual((lock / "pid").read_text(encoding="ascii"), "999999\n")
            self.assertEqual(len(list(backups.iterdir())), 1)

    def test_install_lock_parent_must_be_private_and_not_a_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups, binary = self.fixture(root)
            private_parent = root / "private-lock-parent"
            private_parent.mkdir(mode=0o700)
            symlink_parent = root / "lock-parent-link"
            symlink_parent.symlink_to(private_parent, target_is_directory=True)
            insecure_parent = root / "insecure-lock-parent"
            insecure_parent.mkdir(mode=0o755)
            before = env.read_bytes()
            for label, lock in (
                ("symlink", symlink_parent / "install.lock"),
                ("insecure", insecure_parent / "install.lock"),
            ):
                with self.subTest(label=label):
                    result = self.run_script(
                        "--env-file",
                        env,
                        "--model-dir",
                        model,
                        "--manifest",
                        manifest,
                        "--binary",
                        binary,
                        "--backup-dir",
                        backups,
                        "--dry-run",
                        "--install-lock",
                        lock,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assert_secret_free(result)
                    self.assertEqual(env.read_bytes(), before)
                    self.assertFalse(backups.exists())

    def test_untrusted_launchctl_paths_are_rejected_before_publish(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups, binary = self.fixture(root)
            executable, state_file = self.fake_launchctl(root)
            symlink = root / "launchctl-link"
            symlink.symlink_to(executable)
            non_executable = root / "launchctl-data"
            non_executable.write_text("not executable\n", encoding="utf-8")
            for label, launchctl in (("symlink", symlink), ("nonexec", non_executable)):
                with self.subTest(label=label):
                    result = self.run_script(
                        "--env-file",
                        env,
                        "--model-dir",
                        model,
                        "--manifest",
                        manifest,
                        "--binary",
                        binary,
                        "--backup-dir",
                        backups,
                        "--apply-and-restart",
                        "--launchctl",
                        launchctl,
                        environment={"FAKE_LAUNCHCTL_STATE": str(state_file)},
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assert_secret_free(result)
                    self.assertFalse(backups.exists())

    def test_symlink_environment_is_rejected_without_touching_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env, manifest, model, backups, binary = self.fixture(root)
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
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--apply-and-restart",
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("not a symlink", result.stderr)
            self.assertEqual(env.read_bytes(), target)
            self.assertFalse(backups.exists())

    def test_private_but_unqualified_json_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            env, manifest, model, backups, binary = self.fixture(Path(directory))
            manifest.write_text('{"schema_version":1}\n', encoding="utf-8")
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--dry-run",
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(backups.exists())

    def test_manifest_for_a_different_binary_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            env, manifest, model, backups, binary = self.fixture(Path(directory))
            binary.write_text("#!/bin/sh\nexit 1\n", encoding="utf-8")
            result = self.run_script(
                "--env-file",
                env,
                "--model-dir",
                model,
                "--manifest",
                manifest,
                "--binary",
                binary,
                "--backup-dir",
                backups,
                "--dry-run",
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(backups.exists())

    def test_manifest_route_identity_and_capability_mismatches_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            env, manifest, model, backups, binary = self.fixture(Path(directory))
            original = json.loads(manifest.read_text(encoding="utf-8"))
            cases: list[tuple[str, object]] = [
                ("target", lambda report: report.__setitem__("target", "fm")),
                (
                    "primary model",
                    lambda report: report["primary"]["identity"].__setitem__(
                        "model", "different-model"
                    ),
                ),
                (
                    "vision endpoint",
                    lambda report: report["primary"]["vision_identity"].__setitem__(
                        "endpoint", "http://127.0.0.1:8282"
                    ),
                ),
                (
                    "FM CLI path",
                    lambda report: report["fm_cli"]["identity"].__setitem__(
                        "cli_path", "/tmp/fm"
                    ),
                ),
                (
                    "FM mode",
                    lambda report: report["fm_cli"]["identity"].__setitem__(
                        "mode", "pcc"
                    ),
                ),
                (
                    "FM server",
                    lambda report: report["fm_server"].__setitem__(
                        "configured", True
                    ),
                ),
                (
                    "primary tool capability",
                    lambda report: report["primary"]["capabilities"]["tools"].__setitem__(
                        "status", "fail"
                    ),
                ),
            ]
            for label, mutate in cases:
                with self.subTest(label=label):
                    report = copy.deepcopy(original)
                    mutate(report)
                    manifest.write_text(json.dumps(report) + "\n", encoding="utf-8")
                    result = self.run_script(
                        "--env-file",
                        env,
                        "--model-dir",
                        model,
                        "--manifest",
                        manifest,
                        "--binary",
                        binary,
                        "--backup-dir",
                        backups,
                        "--dry-run",
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(backups.exists())


if __name__ == "__main__":
    unittest.main()
