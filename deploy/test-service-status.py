"""Offline current-status contracts, with no real host observation."""
from contextlib import redirect_stdout
from dataclasses import replace
import io
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import service_status as status
from service_protocol import Failure, ProtocolError

FIXTURES = Path(__file__).resolve().parents[1] / "tests/fixtures/service-protocol"
READY = json.loads((FIXTURES / "readiness-v1.json").read_text())
BOOTSTRAP = json.loads((FIXTURES / "bootstrap-v1.json").read_text())


class Host:
    def __init__(self, documents=None):
        self.documents = documents or [dict(READY), dict(READY)]
        self.bootstrap = None
        self.pid = READY["pid"]
        self.hash = READY["executable_sha256"]
        self.present = True
        self.clock = READY["published_at_unix_ms"]
        self.elapsed = 0
        self.calls = []
        self.replacement = None

    def monotonic(self):
        return self.elapsed

    def wall_ms(self):
        return self.clock

    def service_pid(self, deadline):
        self.calls.append("pid")
        if self.replacement and self.calls.count("pid") >= self.replacement[0]:
            return self.replacement[1]
        return self.pid

    def alive(self, pid):
        self.calls.append("alive")
        return self.present

    def digest(self, deadline):
        self.calls.append("digest")
        return self.hash

    def document(self, kind):
        self.calls.append(kind)
        if kind == "bootstrap":
            return self.bootstrap
        value = self.documents.pop(0)
        if isinstance(value, Exception):
            raise value
        return value


class StatusTests(unittest.TestCase):
    def test_ready_is_two_current_samples_not_installation_acceptance(self):
        host = Host()
        observation = status.observe(host)
        self.assertEqual(observation.kind, status.ObservationKind.READY)
        self.assertEqual(host.calls, ["pid", "alive", "digest", "readiness", "pid", "alive",
                                      "readiness", "pid", "alive"])
        text = status.render(observation)
        self.assertTrue(text.startswith("Abbey service: ready (current observation)"))
        self.assertNotIn("installation", text)
        self.assertNotIn("five seconds", text)

    def test_optional_degraded_and_partial_are_visible_without_losing_ready(self):
        current = {**READY, "slack": "degraded", "telegram": "disabled", "last_persistence": "partial"}
        observation = status.observe(Host([dict(READY), current]))
        self.assertEqual(observation.kind, status.ObservationKind.READY)
        self.assertIn("Slack: degraded", status.render(observation))
        self.assertIn("Last completed persistence: partial", status.render(observation))

    def test_starting_and_draining_never_claim_ready(self):
        for phase in ("starting", "draining"):
            document = {**READY, "phase": phase}
            observation = status.observe(Host([document, document]))
            self.assertEqual(observation.kind, status.ObservationKind.NOT_READY)
        document = {**READY, "phase": "starting"}
        self.assertEqual(status.observe(Host([document, dict(READY)])).kind,
                         status.ObservationKind.NOT_READY)

    def test_identity_change_at_either_read_or_pid_sample_is_unavailable(self):
        for key, value in (("pid", 4321), ("run_nonce", "a" * 64), ("executable_sha256", "b" * 64)):
            for index in (0, 1):
                documents = [dict(READY), dict(READY)]
                documents[index][key] = value
                self.assertEqual(status.observe(Host(documents)).kind, status.ObservationKind.UNAVAILABLE)
        for sample in (2, 3):
            host = Host()
            host.replacement = (sample, 9999)
            self.assertEqual(status.observe(host).kind, status.ObservationKind.UNAVAILABLE)

    def test_exact_freshness_edges_and_no_transaction_floor(self):
        for delta, expected in ((-30001, False), (-30000, True), (2000, True), (2001, False)):
            document = {**READY, "published_at_unix_ms": READY["published_at_unix_ms"] + delta}
            observation = status.observe(Host([document, document]))
            self.assertEqual(observation.kind == status.ObservationKind.READY, expected)

    def test_dead_process_bad_hash_and_invalid_pid_stop_early(self):
        for attribute, value in (("present", False), ("present", 1), ("pid", True), ("pid", 0),
                                  ("hash", "PRIVATE_HASH")):
            host = Host()
            setattr(host, attribute, value)
            self.assertEqual(status.observe(host).kind, status.ObservationKind.UNAVAILABLE)
            if attribute == "pid":
                self.assertEqual(host.calls, ["pid"])

    def test_missing_unsafe_malformed_and_unreadable_remain_unknown(self):
        for value in (None, {}, {**READY, "unknown": "PRIVATE"},
                      ProtocolError(Failure.UNSAFE_FILE), ProtocolError(Failure.INVALID_DOCUMENT),
                      OSError("PRIVATE_PATH_ERROR")):
            observation = status.observe(Host([value, dict(READY)]))
            self.assertEqual(observation.kind, status.ObservationKind.UNAVAILABLE)
            self.assertNotIn("PRIVATE", status.render(observation))

    def test_bootstrap_guidance_requires_matching_fresh_run_identity(self):
        starting = {**READY, "phase": "starting"}
        for code in ("readiness_file", "log_directory", "log_file", "log_writer"):
            host = Host([starting, starting])
            host.bootstrap = {**BOOTSTRAP, "phase": "failed", "code": code}
            self.assertEqual(status.observe(host).kind, status.ObservationKind.BOOTSTRAP_FAILED)
        for key, value in (("pid", 9999), ("run_nonce", "b" * 64), ("executable_sha256", "a" * 64)):
            host = Host([starting, starting])
            host.bootstrap = {**BOOTSTRAP, "phase": "failed", "code": "log_file", key: value}
            self.assertEqual(status.observe(host).kind, status.ObservationKind.NOT_READY)
        host = Host([None, None])
        host.bootstrap = {**BOOTSTRAP, "phase": "failed", "code": "log_file"}
        self.assertEqual(status.observe(host).kind, status.ObservationKind.UNAVAILABLE)
        self.assertNotIn("bootstrap", host.calls)

    def test_budget_exhaustion_never_returns_ready(self):
        host = Host()
        original = host.document
        def read(kind):
            result = original(kind)
            host.elapsed += 5_000_000_000
            return result
        host.document = read
        self.assertEqual(status.observe(host).kind, status.ObservationKind.UNAVAILABLE)

    def test_output_uses_per_field_closed_labels_and_no_canaries(self):
        base = status.ServiceObservation(status.ObservationKind.BOOTSTRAP_FAILED)
        for field in ("discord", "scheduler", "telegram", "slack", "persistence", "bootstrap_code"):
            text = status.render(replace(base, **{field: "PRIVATE_SECRET_PATH_NONCE"}))
            self.assertNotIn("PRIVATE", text)
        self.assertIn("Scheduler: unknown", status.render(replace(base, scheduler="connecting")))
        self.assertIn("Discord: unknown", status.render(replace(base, discord="running")))

    def test_cli_usage_help_platform_and_exit_codes_without_host_access(self):
        def forbidden(_):
            raise AssertionError("host must not be constructed")
        for args, platform, code in ((["--help"], "darwin", 0), (["--help", "PRIVATE"], "darwin", 2),
                                     (["--json"], "darwin", 2), ([], "win32", 2)):
            output = io.StringIO()
            with redirect_stdout(output):
                self.assertEqual(status.main(args, platform=platform, host_factory=forbidden), code)
            self.assertNotIn("PRIVATE", output.getvalue())
        for host, expected in ((Host(), 0), (Host([None, None]), 1)):
            with redirect_stdout(io.StringIO()):
                self.assertEqual(status.main([], platform="darwin", host_factory=lambda _: host,
                                             environ={"HOME": "/synthetic-home"}), expected)

    def test_host_factory_error_is_fixed_and_no_real_io_is_used(self):
        def failed(_):
            raise RuntimeError("PRIVATE_CONSTRUCTOR_ERROR")
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(status.main([], platform="darwin", host_factory=failed,
                                         environ={"HOME": "/synthetic-home"}), 1)
        self.assertNotIn("PRIVATE", output.getvalue())
        with patch.object(status.os, "kill", side_effect=AssertionError("no signal")), \
             patch.object(status, "launchd_pid", side_effect=AssertionError("no launchd")), \
             patch.object(status, "binary_digest", side_effect=AssertionError("no real file")), \
             patch.object(status, "read_optional_private", side_effect=AssertionError("no owner data")):
            self.assertEqual(status.observe(Host()).kind, status.ObservationKind.READY)

    def test_actual_safe_readers_use_only_synthetic_fixed_artifacts(self):
        with tempfile.TemporaryDirectory(prefix="abbey-status-") as directory:
            home = Path(directory) / "home"
            binary = home / ".local/libexec/abbey-bot/abbey-bot"
            readiness = home / ".local/share/abbey-bot/readiness.json"
            for parent in (binary.parent, readiness.parent):
                parent.mkdir(parents=True, mode=0o700)
            for parent in (home, home / ".local", home / ".local/libexec", home / ".local/share"):
                parent.chmod(0o700)
            contents = b"synthetic executable bytes, never executed"
            binary.write_bytes(contents)
            binary.chmod(0o700)
            ready = {**READY, "executable_sha256": hashlib.sha256(contents).hexdigest()}
            readiness.write_text(json.dumps(ready))
            readiness.chmod(0o600)
            with patch.object(status, "launchd_pid", return_value=ready["pid"]) as launchd, \
                 patch.object(status, "process_exists", return_value=True) as alive, \
                 patch.object(status.time, "time_ns", return_value=ready["published_at_unix_ms"] * 1_000_000):
                observation = status.observe(status.LocalHost(home))
                self.assertEqual(observation.kind, status.ObservationKind.READY)
                self.assertEqual(launchd.call_count, 3)
                self.assertEqual(alive.call_count, 3)
                readiness.chmod(0o644)
                self.assertEqual(status.observe(status.LocalHost(home)).kind, status.ObservationKind.UNAVAILABLE)
            self.assertNotIn(str(home), status.render(observation))
            self.assertFalse((home / ".config").exists())
            self.assertFalse((home / "Library").exists())
            self.assertEqual(binary.read_bytes(), contents)

    def test_relocated_bundle_help_and_missing_dependency_are_private(self):
        deploy = Path(__file__).resolve().parent
        names = ("service-status.py", "service_status.py", "service_installation.py",
                 "service_protocol.py", "service_readiness.py", "service-protocol-v1.json")
        with tempfile.TemporaryDirectory(prefix="abbey-status-bundle-") as directory:
            root = Path(directory)
            for name in names:
                shutil.copyfile(deploy / name, root / name)
            args = [sys.executable, "-I", str(root / "service-status.py"), "--help"]
            result = subprocess.run(args, cwd="/", capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0)
            self.assertIn("Read-only current service evidence", result.stdout)
            (root / "service-protocol-v1.json").unlink()
            result = subprocess.run(args, cwd="/", capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 1)
            self.assertNotIn(str(root), result.stdout + result.stderr)
            self.assertNotIn("Traceback", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
