#!/usr/bin/env python3
"""Offline regressions for the token-free POSIX smoke drivers."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
DRIVERS = (
    Path(".agents/skills/run-abbey-bot/smoke.sh"),
    Path(".claude/skills/run-abbey-bot/smoke.sh"),
)
SHELL = shutil.which("sh")
CAPABILITIES = ("text", "streaming", "structured_output", "tools", "vision", "ocr")
CREDENTIAL_KEYS = (
    "DISCORD_TOKEN",
    "DISCORD_BOT_TOKEN",
    "OPENAI_API_KEY",
    "GROK_API_KEY",
    "ABBEY_BOT_LLM_API_KEY",
    "ABBEY_VISION_API_KEY",
)
STATE_KEYS = (
    "ABBEY_DATA_DIR",
    "ABBEY_EPISODE_GATE_CONFIG",
    "ABBEY_EPISODE_GATE_ACCEPTANCE_CONFIG",
)
FM_KEYS = ("ABBEY_FM_MODE", "ABBEY_FM_ROLE", "ABBEY_FM_PCC_TIMEOUT_SECS")


def evidence(mode: str | None, status: str = "pass") -> dict:
    identity = {"abbey_binary_sha256": "a" * 64}
    if mode is not None:
        identity["mode"] = mode
    return {
        "configured": True,
        "identity": identity,
        "capabilities": {name: {"status": status} for name in CAPABILITIES},
    }


def report() -> dict:
    system = evidence("system")
    for name in ("vision", "ocr"):
        system["capabilities"][name]["status"] = "fail"
    system["capabilities"]["streaming"]["status"] = "unsupported"
    return {
        "overall_pass": False,
        "primary": evidence(None),
        "fm_server": {"configured": False, "capabilities": {}},
        "fm_cli": evidence("pcc", "fail"),
        "fm_cli_modes": [evidence("pcc", "fail"), system],
    }


@unittest.skipUnless(os.name == "posix" and SHELL, "POSIX shell driver execution only")
class SmokeDriverTests(unittest.TestCase):
    def run_driver(
        self,
        driver: Path,
        document: dict | str,
        cli_exit: int = 1,
        owner: dict[str, str] | None = None,
        inherited: dict[str, str] | None = None,
        timeout_exit: int | None = None,
    ) -> tuple[subprocess.CompletedProcess[str], dict | None, list]:
        with tempfile.TemporaryDirectory(prefix="abbey-smoke-offline-") as temporary:
            scratch = Path(temporary)
            repository = scratch / "repo"
            destination = repository / driver
            destination.parent.mkdir(parents=True)
            shutil.copyfile(ROOT / driver, destination)
            home = scratch / "home"
            env_file = home / ".config/abbey-bot/env"
            env_file.parent.mkdir(parents=True)
            env_file.write_text(
                "".join(f"{key}={value}\n" for key, value in (owner or {}).items()),
                encoding="utf-8",
            )
            env_file.chmod(0o600)
            payload = scratch / "report.json"
            payload.write_text(
                document if isinstance(document, str) else json.dumps(document),
                encoding="utf-8",
            )
            observed = scratch / "observed.json"
            timeout_record = scratch / "timeout.json"
            tools = scratch / "bin"
            tools.mkdir()
            (tools / "python3").symlink_to(sys.executable)

            def executable(path: Path, body: str) -> None:
                source = path.with_suffix(".py")
                source.write_text(body, encoding="utf-8")
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(
                    "#!/bin/sh\nexec "
                    + shlex.quote(sys.executable)
                    + " "
                    + shlex.quote(str(source))
                    + ' "$@"\n',
                    encoding="utf-8",
                )
                path.chmod(0o755)

            binary = repository / "target/release/abbey-bot"
            binary.parent.mkdir(parents=True)
            selected_keys = FM_KEYS + CREDENTIAL_KEYS + STATE_KEYS
            executable(
                binary,
                "import json, os, pathlib, sys\n"
                "assert sys.argv[1:] == ['--provider-self-test', 'all', '--json']\n"
                f"keys = {selected_keys!r}\n"
                f"pathlib.Path({str(observed)!r}).write_text(json.dumps(\n"
                "    {key: os.environ.get(key) for key in keys}))\n"
                f"sys.stdout.write(pathlib.Path({str(payload)!r}).read_text())\n"
                f"sys.exit({cli_exit})\n",
            )
            executable(
                tools / "timeout",
                "import json, os, pathlib, sys\n"
                f"pathlib.Path({str(timeout_record)!r}).write_text(json.dumps(sys.argv[1:]))\n"
                + (f"sys.exit({timeout_exit})\n" if timeout_exit is not None else "")
                + "os.execv(sys.argv[2], sys.argv[2:])\n",
            )
            environment = {
                "PATH": str(tools) + os.pathsep + os.defpath,
                "HOME": str(home),
                "TMPDIR": str(scratch),
                "RUN_ABBEY_BOT_OUT": str(scratch / "out"),
            }
            environment.update(inherited or {})
            result = subprocess.run(
                [str(SHELL), str(destination), "provider", "all"],
                cwd=repository,
                env=environment,
                stdin=subprocess.DEVNULL,
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            if observed.exists():
                captured = json.loads(observed.read_text())
            elif timeout_exit is not None:
                captured = None
            else:
                self.fail("fake CLI did not create its environment capture")
            return result, captured, json.loads(timeout_record.read_text())

    def test_every_fm_mode_is_printed_without_duplicate_legacy_lane(self) -> None:
        for driver in DRIVERS:
            with self.subTest(driver=driver):
                result, _, _ = self.run_driver(driver, report())
                self.assertEqual(result.returncode, 1, result.stderr)
                lines = result.stdout.splitlines()
                pcc = [line for line in lines if "fm_cli[pcc]" in line]
                system = [line for line in lines if "fm_cli[system]" in line]
                self.assertEqual(len(pcc), 1, result.stdout)
                self.assertEqual(len(system), 1, result.stdout)
                self.assertIn("text=fail", pcc[0])
                self.assertIn("text=pass", system[0])
                self.assertIn("structured_output=pass", system[0])
                self.assertIn("tools=pass", system[0])
                self.assertIn("streaming=unsupported", system[0])
                self.assertIn("vision=fail", system[0])
                self.assertLess(lines.index(pcc[0]), lines.index(system[0]))
                self.assertFalse(any(line.strip().startswith("fm_cli ") for line in lines))

    def test_legacy_report_without_nonempty_mode_array_is_supported(self) -> None:
        for driver in DRIVERS:
            for modes in (None, []):
                with self.subTest(driver=driver, modes=modes):
                    document = report()
                    document["overall_pass"] = True
                    document["fm_cli"] = evidence("system")
                    if modes is None:
                        del document["fm_cli_modes"]
                    else:
                        document["fm_cli_modes"] = modes
                    result, _, _ = self.run_driver(driver, document, cli_exit=0)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    legacy = [line for line in result.stdout.splitlines() if "fm_cli " in line]
                    self.assertEqual(len(legacy), 1, result.stdout)
                    self.assertIn("text=pass", legacy[0])

    def test_probe_failure_and_configuration_statuses_are_retained(self) -> None:
        for driver in DRIVERS:
            for cli_exit in (1, 2):
                with self.subTest(driver=driver, cli_exit=cli_exit):
                    result, _, _ = self.run_driver(driver, report(), cli_exit=cli_exit)
                    self.assertEqual(result.returncode, cli_exit, result.stderr)

    def test_unavailable_primary_identity_retains_configuration_exit(self) -> None:
        for driver in DRIVERS:
            for omitted in (False, True):
                with self.subTest(driver=driver, omitted=omitted):
                    document = report()
                    document["primary"]["configured"] = False
                    if omitted:
                        del document["primary"]["identity"]
                    else:
                        document["primary"]["identity"] = None
                    result, _, _ = self.run_driver(driver, document, cli_exit=2)
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertIn("binary sha256: unavailable", result.stdout)
                    self.assertNotIn("Traceback", result.stderr)

    def test_owner_fm_selection_survives_without_credentials_or_state(self) -> None:
        sensitive = {key: "synthetic-disallowed" for key in CREDENTIAL_KEYS + STATE_KEYS}
        owner = {
            **sensitive,
            "ABBEY_FM_MODE": "pcc,system",
            "ABBEY_FM_ROLE": "primary",
            "ABBEY_FM_PCC_TIMEOUT_SECS": "17",
        }
        inherited = {**sensitive, "ABBEY_FM_ROLE": "fallback", "ABBEY_FM_PCC_TIMEOUT_SECS": "1"}
        for driver in DRIVERS:
            with self.subTest(driver=driver):
                result, observed, timeout_record = self.run_driver(
                    driver, report(), owner=owner, inherited=inherited
                )
                self.assertEqual(result.returncode, 1, result.stderr)
                for key in FM_KEYS:
                    with self.subTest(key=key):
                        self.assertEqual(observed[key], owner[key], key)
                for key in CREDENTIAL_KEYS:
                    self.assertIsNone(observed[key], key)
                for key in STATE_KEYS:
                    self.assertEqual(observed[key], "", key)
                self.assertEqual(timeout_record[0], "600")

    def test_inherited_nonsecret_fm_selection_survives(self) -> None:
        inherited = {"ABBEY_FM_ROLE": "primary", "ABBEY_FM_PCC_TIMEOUT_SECS": "29"}
        for driver in DRIVERS:
            with self.subTest(driver=driver):
                result, observed, _ = self.run_driver(driver, report(), inherited=inherited)
                self.assertEqual(result.returncode, 1, result.stderr)
                for key, value in inherited.items():
                    with self.subTest(key=key):
                        self.assertEqual(observed[key], value, key)

    def test_malformed_report_fails(self) -> None:
        for driver in DRIVERS:
            with self.subTest(driver=driver):
                result, _, _ = self.run_driver(driver, "not-json", cli_exit=0)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("JSONDecodeError", result.stderr)

    def test_malformed_report_preserves_nonzero_cli_status(self) -> None:
        for driver in DRIVERS:
            with self.subTest(driver=driver):
                result, _, _ = self.run_driver(driver, "not-json", cli_exit=2)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("JSONDecodeError", result.stderr)

    def test_timeout_before_cli_with_empty_report_retains_timeout_status(self) -> None:
        for driver in DRIVERS:
            with self.subTest(driver=driver):
                result, captured, timeout_record = self.run_driver(
                    driver, report(), timeout_exit=124
                )
                self.assertEqual(result.returncode, 124, result.stderr)
                self.assertIsNone(captured, "the fake CLI must not run after timeout")
                self.assertEqual(timeout_record[0], "600")
                self.assertIn("JSONDecodeError", result.stderr)


if __name__ == "__main__":
    unittest.main()
