"""Offline conformance tests. No process signals, launchd, or real owner data."""
import errno
import importlib.util
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("service_protocol", ROOT / "deploy/service_protocol.py")
protocol = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = protocol
spec.loader.exec_module(protocol)
FIXTURES = ROOT / "tests/fixtures/service-protocol"


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.raw = (FIXTURES / "readiness-v1.json").read_bytes()
        self.ready = protocol.parse_document(self.raw)

    def test_shared_document_corpus(self):
        cases = json.loads((FIXTURES / "documents-v1.json").read_text())
        self.assertGreaterEqual(len(cases), 200)
        for case in cases:
            with self.subTest(case=case["name"]):
                raw = case["document"].encode()
                if case["valid"]:
                    result = protocol.parse_document(raw, case["kind"])
                    self.assertEqual(protocol.parse_document(protocol.encode_document(result, case["kind"]),
                                                            case["kind"]), result)
                else:
                    with self.assertRaises(protocol.ProtocolError) as error:
                        protocol.parse_document(raw, case["kind"])
                    self.assertEqual(str(error.exception), "invalid_document")

    def test_exact_canonical_wire_and_field_counts(self):
        for kind, count in (("readiness", 11), ("bootstrap", 6)):
            raw = (FIXTURES / f"{kind}-v1.json").read_bytes()
            parsed = protocol.parse_document(raw, kind)
            self.assertEqual(len(parsed), count)
            self.assertEqual(protocol.encode_document(dict(reversed(list(parsed.items()))), kind), raw)
        self.assertLessEqual(len((FIXTURES / "bootstrap-v1.json").read_bytes()), 512)

    def test_invalid_utf8_and_private_debug(self):
        with self.assertRaises(protocol.ProtocolError) as error:
            protocol.parse_document(b"\xffPRIVATE_CANARY")
        self.assertEqual(repr(protocol.identity(self.ready)), "RunIdentity(<private>)")
        self.assertNotIn("CANARY", repr(error.exception))
        self.assertIsNone(error.exception.__cause__)

    def test_freshness_shared_boundaries(self):
        for case in json.loads((FIXTURES / "freshness-v1.json").read_text()):
            with self.subTest(case=case["name"]):
                self.assertEqual(protocol.fresh(case["published"], case["start"], case["now"]), case["fresh"])

    def test_ready_predicate_requires_every_gate(self):
        calls = []
        def alive(pid):
            calls.append(pid)
            return True
        args = dict(transaction_start_ms=self.ready["published_at_unix_ms"],
                    now_ms=self.ready["published_at_unix_ms"], launchd_pid=self.ready["pid"],
                    expected_sha256=self.ready["executable_sha256"], alive=alive)
        self.assertEqual(protocol.validate_ready(self.ready, **args), protocol.identity(self.ready))
        self.assertEqual(calls, [4242])
        for key, value, category in (("pid", 4243, protocol.Failure.IDENTITY_MISMATCH),
                                     ("executable_sha256", "0" * 64, protocol.Failure.IDENTITY_MISMATCH),
                                     ("phase", "draining", protocol.Failure.NOT_READY),
                                     ("discord", "connecting", protocol.Failure.NOT_READY),
                                     ("scheduler", "stopped", protocol.Failure.NOT_READY),
                                     ("published_at_unix_ms", 0, protocol.Failure.STALE)):
            calls.clear()
            with self.subTest(key=key), self.assertRaises(protocol.ProtocolError) as error:
                protocol.validate_ready({**self.ready, key: value}, **args)
            self.assertEqual(error.exception.category, category)
            self.assertEqual(calls, [])
        for value in (False, None, 1):
            with self.assertRaises(protocol.ProtocolError) as error:
                protocol.validate_ready(self.ready, **{**args, "alive": lambda _pid: value})
            self.assertEqual(error.exception.category, protocol.Failure.PROCESS_UNAVAILABLE)
        for code in (errno.EPERM, errno.ESRCH):
            with patch.object(protocol.os, "kill", side_effect=OSError(code, "PRIVATE_CANARY")) as kill:
                self.assertFalse(protocol.process_exists(4242))
                kill.assert_called_once_with(4242, 0)


    def test_liveness_never_probes_noncanonical_pid_values(self):
        with patch.object(protocol.os, "kill", side_effect=AssertionError("no signal for invalid PID")) as kill:
            for value in (0, -1, True, "4242", 4242.0, 2147483648):
                self.assertFalse(protocol.process_exists(value))
            kill.assert_not_called()

    def test_optional_degraded_states_are_not_false_failures(self):
        args = dict(transaction_start_ms=0, now_ms=self.ready["published_at_unix_ms"],
                    launchd_pid=4242, expected_sha256=self.ready["executable_sha256"], alive=lambda _: True)
        for persistence in ("not_attempted", "memory_only", "complete", "partial", "failed"):
            result = protocol.validate_ready({**self.ready, "slack": "degraded", "telegram": "disabled",
                                              "last_persistence": persistence}, **args)
            self.assertEqual(result, protocol.identity(self.ready))


class PrivateFileTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="abbey-protocol-test-")
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name) / "home"
        self.parent = self.home / ".local/share/abbey-bot"
        self.parent.mkdir(parents=True, mode=0o700)
        for path in (self.home, self.home / ".local", self.home / ".local/share", self.parent):
            path.chmod(0o700)
        self.path = self.parent / "readiness.json"
        self.raw = (FIXTURES / "readiness-v1.json").read_bytes()
        self.path.write_bytes(self.raw)
        self.path.chmod(0o600)

    def test_fixed_private_file_success_without_process_probe(self):
        with patch.object(protocol.os, "kill", side_effect=AssertionError("no signals")):
            self.assertEqual(protocol.read_private(self.home), protocol.parse_document(self.raw))
        self.assertEqual(self.path.read_bytes(), self.raw)

    def test_mode_and_owner_fail_closed(self):
        for target, bad_mode in ((self.path, 0o644), (self.parent, 0o755), (self.home / ".local", 0o777)):
            original = stat.S_IMODE(target.stat().st_mode)
            target.chmod(bad_mode)
            try:
                with self.subTest(target=target.name):
                    with self.assertRaises(protocol.ProtocolError) as error:
                        protocol.read_private(self.home)
                    self.assertEqual(error.exception.category, protocol.Failure.UNSAFE_FILE)
            finally:
                target.chmod(original)
        with self.assertRaises(protocol.ProtocolError) as error:
            protocol.read_private(self.home, uid=os.getuid() + 1)
        self.assertEqual(error.exception.category, protocol.Failure.UNSAFE_FILE)

    def test_symlink_directory_fifo_and_missing_do_not_block_or_follow(self):
        self.path.unlink()
        secret = self.home / "secret"
        secret.write_bytes(self.raw)
        secret.chmod(0o600)
        self.path.symlink_to(secret)
        with self.assertRaises(protocol.ProtocolError):
            protocol.read_private(self.home)
        self.assertEqual(secret.read_bytes(), self.raw)
        self.path.unlink()
        self.path.mkdir()
        with self.assertRaises(protocol.ProtocolError):
            protocol.read_private(self.home)
        self.path.rmdir()
        os.mkfifo(self.path, 0o600)
        with self.assertRaises(protocol.ProtocolError):
            protocol.read_private(self.home)
        self.path.unlink()
        with self.assertRaises(protocol.ProtocolError):
            protocol.read_private(self.home)
        self.parent.rmdir()
        alternate = self.home / "alternate"
        alternate.mkdir(mode=0o700)
        (alternate / "readiness.json").write_bytes(self.raw)
        (alternate / "readiness.json").chmod(0o600)
        self.parent.symlink_to(alternate)
        with self.assertRaises(protocol.ProtocolError) as error:
            protocol.read_private(self.home)
        self.assertEqual(error.exception.category, protocol.Failure.UNSAFE_FILE)


    def test_every_ancestor_symlink_rejects_an_otherwise_valid_document(self):
        for target in (self.home, self.home / ".local", self.home / ".local/share", self.parent, self.path):
            actual = target.with_name(target.name + "-actual")
            target.rename(actual)
            target.symlink_to(actual)
            try:
                with self.subTest(target=target.name):
                    with self.assertRaises(protocol.ProtocolError) as error:
                        protocol.read_private(self.home)
                    self.assertEqual(error.exception.category, protocol.Failure.UNSAFE_FILE)
            finally:
                target.unlink()
                actual.rename(target)


    def test_each_opened_owner_is_checked_independently(self):
        original = os.fstat
        for target in (self.home, self.home / ".local", self.home / ".local/share", self.parent, self.path):
            selected = target.stat()
            def fstat(fd):
                metadata = original(fd)
                if (metadata.st_dev, metadata.st_ino) == (selected.st_dev, selected.st_ino):
                    fields = ("st_uid", "st_mode", "st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns")
                    altered = {key: getattr(metadata, key) for key in fields}
                    altered["st_uid"] += 1
                    return SimpleNamespace(**altered)
                return metadata
            with self.subTest(target=target.name), patch.object(protocol.os, "fstat", side_effect=fstat):
                with self.assertRaises(protocol.ProtocolError) as error:
                    protocol.read_private(self.home)
                self.assertEqual(error.exception.category, protocol.Failure.UNSAFE_FILE)

    def test_close_failures_are_private_and_all_descriptors_are_attempted(self):
        original = os.close
        for invalid in (False, True):
            closed = []
            if invalid:
                self.path.write_bytes(b"PRIVATE_CANARY")
            def close(fd):
                original(fd)
                closed.append(fd)
                raise OSError(errno.EIO, "PRIVATE_CANARY")
            with patch.object(protocol.os, "close", side_effect=close):
                with self.assertRaises(protocol.ProtocolError) as error:
                    protocol.read_private(self.home)
            expected = protocol.Failure.INVALID_DOCUMENT if invalid else protocol.Failure.UNAVAILABLE
            self.assertEqual(error.exception.category, expected)
            self.assertNotIn("CANARY", repr(error.exception))
            self.assertEqual(len(closed), 5)
            self.assertEqual(len(set(closed)), 5)
            for fd in closed:
                with self.assertRaises(OSError):
                    os.fstat(fd)


    def test_close_failure_is_not_hidden_by_a_callers_except_context(self):
        original = os.close
        closed = []
        def close(fd):
            original(fd)
            closed.append(fd)
            raise OSError(errno.EIO, "PRIVATE_CANARY")
        try:
            raise RuntimeError("caller is handling an unrelated error")
        except RuntimeError:
            with patch.object(protocol.os, "close", side_effect=close):
                with self.assertRaises(protocol.ProtocolError) as error:
                    protocol.read_private(self.home)
            self.assertEqual(error.exception.category, protocol.Failure.UNAVAILABLE)
        self.assertEqual(len(closed), 5)

    def test_bootstrap_cap_and_file_mapping(self):
        bootstrap = self.parent / "bootstrap-status.json"
        raw = (FIXTURES / "bootstrap-v1.json").read_bytes()
        bootstrap.write_bytes(raw)
        bootstrap.chmod(0o600)
        self.assertEqual(protocol.read_private(self.home, "bootstrap"), protocol.parse_document(raw, "bootstrap"))
        bootstrap.write_bytes(b" " * 513)
        with self.assertRaises(protocol.ProtocolError) as error:
            protocol.read_private(self.home, "bootstrap")
        self.assertEqual(error.exception.category, protocol.Failure.INVALID_DOCUMENT)

    def test_in_place_change_is_rejected(self):
        original = os.read
        changed = False
        def read(fd, size):
            nonlocal changed
            result = original(fd, size)
            if not changed:
                changed = True
                with self.path.open("ab") as output:
                    output.write(b" ")
            return result
        with patch.object(protocol.os, "read", side_effect=read):
            with self.assertRaises(protocol.ProtocolError) as error:
                protocol.read_private(self.home)
        self.assertEqual(error.exception.category, protocol.Failure.UNAVAILABLE)


if __name__ == "__main__":
    unittest.main()
