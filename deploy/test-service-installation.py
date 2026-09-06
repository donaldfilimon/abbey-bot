"""Synthetic installation artifacts only; no installed service is accessed."""
import errno
import hashlib
import os
from pathlib import Path
import plistlib
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import service_installation as installation
from service_protocol import ProtocolError


class InstallationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="abbey-install-artifacts-")
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name) / "home"
        self.binary = installation.installed_binary(self.home)
        self.binary.parent.mkdir(parents=True, mode=0o700)
        for path in (self.home, self.home / ".local", self.home / ".local/libexec", self.binary.parent):
            path.chmod(0o700)
        self.data = b"PRIVATE_BINARY_CANARY" * 10000
        self.binary.write_bytes(self.data)
        self.binary.chmod(0o700)

    def digest(self, **kwargs):
        return installation.binary_digest(self.home, 100, monotonic=lambda: 0, **kwargs)

    def test_exact_multiblock_digest_without_mutation(self):
        before = self.binary.stat()
        with patch.object(installation.os, "kill", side_effect=AssertionError("no signals")):
            self.assertEqual(self.digest(), hashlib.sha256(self.data).hexdigest())
        after = self.binary.stat()
        self.assertEqual((before.st_size, before.st_mtime_ns, before.st_mode),
                         (after.st_size, after.st_mtime_ns, after.st_mode))
        self.assertEqual(self.binary.read_bytes(), self.data)

    def test_every_symlink_is_rejected_with_valid_target(self):
        for target in (self.home, self.home / ".local", self.home / ".local/libexec",
                       self.binary.parent, self.binary):
            actual = target.with_name(target.name + "-actual")
            target.rename(actual)
            target.symlink_to(actual)
            try:
                with self.subTest(target=target.name), self.assertRaises(ProtocolError):
                    self.digest()
            finally:
                target.unlink()
                actual.rename(target)

    def test_modes_owner_and_nonregular_types_fail(self):
        for target, mode in ((self.binary, 0o600), (self.binary.parent, 0o755),
                             (self.home / ".local", 0o777)):
            target.chmod(mode)
            try:
                with self.assertRaises(ProtocolError):
                    self.digest()
            finally:
                target.chmod(0o700)
        with self.assertRaises(ProtocolError):
            self.digest(uid=os.getuid() + 1)
        self.binary.unlink()
        os.mkfifo(self.binary, 0o700)
        with self.assertRaises(ProtocolError):
            self.digest()

    def test_expired_deadline_opens_nothing_and_read_work_consumes_budget(self):
        with patch.object(installation.os, "open", side_effect=AssertionError("no open")):
            with self.assertRaises(ProtocolError):
                installation.binary_digest(self.home, 10, monotonic=lambda: 10)
        ticks = iter(range(100))
        with self.assertRaises(ProtocolError):
            installation.binary_digest(self.home, 8, monotonic=lambda: next(ticks))

    def test_every_opened_owner_is_independently_checked(self):
        original = os.fstat
        for target in (self.home, self.home / ".local", self.home / ".local/libexec",
                       self.binary.parent, self.binary):
            selected = target.stat()
            def metadata(fd):
                actual = original(fd)
                if (actual.st_dev, actual.st_ino) == (selected.st_dev, selected.st_ino):
                    fields = ("st_uid", "st_mode", "st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns")
                    values = {name: getattr(actual, name) for name in fields}
                    values["st_uid"] += 1
                    return SimpleNamespace(**values)
                return actual
            with self.subTest(target=target.name), patch.object(installation.os, "fstat", side_effect=metadata):
                with self.assertRaises(ProtocolError):
                    self.digest()

    def test_successful_cleanup_still_consumes_deadline(self):
        now = 0
        original = os.close
        def close(fd):
            nonlocal now
            original(fd)
            now = 100
        with patch.object(installation.os, "close", side_effect=close):
            with self.assertRaises(ProtocolError):
                installation.binary_digest(self.home, 100, monotonic=lambda: now)

    def test_changed_file_and_io_error_never_return_a_digest(self):
        original = os.read
        modified = False
        def changed(fd, size):
            nonlocal modified
            chunk = original(fd, size)
            if not modified:
                modified = True
                with self.binary.open("ab") as output:
                    output.write(b"x")
            return chunk
        with patch.object(installation.os, "read", side_effect=changed):
            with self.assertRaises(ProtocolError):
                self.digest()
        with patch.object(installation.os, "read", side_effect=OSError(errno.EIO, "PRIVATE_ERROR")):
            with self.assertRaises(ProtocolError) as error:
                self.digest()
        self.assertNotIn("PRIVATE", repr(error.exception))

    def test_cleanup_failure_is_private_and_all_handles_are_closed(self):
        original = os.close
        closed = []
        def close(fd):
            original(fd)
            closed.append(fd)
            raise OSError(errno.EIO, "PRIVATE_ERROR_CANARY")
        with patch.object(installation.os, "close", side_effect=close):
            with self.assertRaises(ProtocolError) as error:
                self.digest()
        self.assertEqual(len(closed), 5)
        self.assertEqual(len(set(closed)), 5)
        self.assertNotIn("CANARY", repr(error.exception))

    def valid_plist(self):
        return {"Label": installation.LABEL,
                "ProgramArguments": [str(self.binary), "--managed-service"],
                "WorkingDirectory": str(self.home / ".local/share/abbey-bot"),
                "StandardOutPath": "/dev/null", "StandardErrorPath": "/dev/null", "Umask": 63}

    def test_plist_exact_managed_arguments_and_private_outputs(self):
        value = self.valid_plist()
        for fmt in (plistlib.FMT_XML, plistlib.FMT_BINARY):
            installation.validate_managed_plist(plistlib.dumps(value, fmt=fmt), self.home)
        changes = [("ProgramArguments", [str(self.binary)]),
                   ("ProgramArguments", [str(self.binary), "--managed-service", "--managed-service"]),
                   ("ProgramArguments", ["/bin/sh", "-c", "PRIVATE_COMMAND"]),
                   ("Program", "PRIVATE_PROGRAM"), ("BundleProgram", "PRIVATE_BUNDLE"),
                   ("Label", "wrong"), ("WorkingDirectory", "/wrong"),
                   ("StandardOutPath", "PRIVATE_LOG"), ("StandardErrorPath", "PRIVATE_LOG"),
                   ("Umask", True), ("Umask", 0)]
        for key, item in changes:
            with self.subTest(key=key), self.assertRaises(ProtocolError) as error:
                installation.validate_managed_plist(plistlib.dumps({**value, key: item}), self.home)
            self.assertNotIn("PRIVATE", repr(error.exception))

    def test_plist_duplicate_malformed_and_oversize_are_fixed_errors(self):
        raw = plistlib.dumps(self.valid_plist())
        duplicate = raw.replace(b"<dict>", b"<dict><key>Label</key><string>PRIVATE</string>", 1)
        for value in (duplicate, b"<plist>PRIVATE", b"x" * 65537, b"PRIVATE"):
            with self.assertRaises(ProtocolError) as error:
                installation.validate_managed_plist(value, self.home)
            self.assertNotIn("PRIVATE", repr(error.exception))


if __name__ == "__main__":
    unittest.main()
