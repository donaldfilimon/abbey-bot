#!/usr/bin/env python3
"""Offline regressions for Abbey's portable Linux TLS dependency guard."""

from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
import importlib.util
from io import StringIO
from pathlib import Path
import subprocess
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/check-linux-tls-tree.py"

SPEC = importlib.util.spec_from_file_location("check_linux_tls_tree", SCRIPT)
if SPEC is None or SPEC.loader is None:  # pragma: no cover - import machinery guard
    raise RuntimeError(f"unable to load {SCRIPT}")
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)

PORTABLE_TREE = """\
abbey-bot v0.1.0
reqwest v0.12.23
rustls v0.22.4
rustls v0.23.34
rustls-webpki v0.102.8
tokio-rustls v0.25.0
tokio-rustls v0.26.2
tokio-tungstenite v0.21.0
webpki-roots v0.26.11
[build-dependencies]
cc v1.2.38
"""

FORBIDDEN_PACKAGES = (
    "hyper-tls",
    "native-tls",
    "openssl",
    "openssl-macros",
    "openssl-probe",
    "openssl-sys",
    "tokio-native-tls",
)

REQUIRED_PACKAGES = (
    "rustls",
    "tokio-rustls",
    "webpki-roots",
)


class LinuxTlsTreeCheckTests(unittest.TestCase):
    def run_check(
        self,
        tree: str,
        *,
        returncode: int = 0,
        cargo_stderr: str = "",
    ) -> tuple[int, str, str, mock.Mock]:
        cargo_result = subprocess.CompletedProcess(
            args=["cargo", "tree"],
            returncode=returncode,
            stdout=tree,
            stderr=cargo_stderr,
        )
        stdout = StringIO()
        stderr = StringIO()
        with mock.patch.object(
            CHECKER.subprocess,
            "run",
            return_value=cargo_result,
        ) as run:
            with redirect_stdout(stdout), redirect_stderr(stderr):
                status = CHECKER.main()
        return status, stdout.getvalue(), stderr.getvalue(), run

    def test_portable_rustls_tree_passes(self) -> None:
        status, stdout, stderr, run = self.run_check(PORTABLE_TREE)

        self.assertEqual(status, 0)
        self.assertEqual(stderr, "")
        self.assertEqual(
            stdout,
            "linux TLS dependency tree: OK "
            "(Rustls/WebPKI; native TLS and OpenSSL absent)\n",
        )
        run.assert_called_once_with(
            [
                "cargo",
                "tree",
                "--locked",
                "--target",
                "x86_64-unknown-linux-gnu",
                "--edges",
                "normal,build",
                "--prefix",
                "none",
            ],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )

    def test_each_native_tls_or_openssl_package_is_rejected(self) -> None:
        for package in FORBIDDEN_PACKAGES:
            with self.subTest(package=package):
                status, stdout, stderr, _ = self.run_check(
                    PORTABLE_TREE + f"{package} v1.0.0\n"
                )

                self.assertEqual(status, 1)
                self.assertEqual(stdout, "")
                self.assertEqual(
                    stderr,
                    f"linux TLS dependency tree: forbidden packages: {package}\n",
                )

    def test_multiple_forbidden_packages_are_reported_in_stable_order(self) -> None:
        status, stdout, stderr, _ = self.run_check(
            PORTABLE_TREE
            + "openssl-sys v0.9.109\n"
            + "native-tls v0.2.14\n"
            + "openssl v0.10.73\n"
        )

        self.assertEqual(status, 1)
        self.assertEqual(stdout, "")
        self.assertEqual(
            stderr,
            "linux TLS dependency tree: forbidden packages: "
            "native-tls, openssl, openssl-sys\n",
        )

    def test_each_required_portable_tls_package_must_be_present(self) -> None:
        lines = PORTABLE_TREE.splitlines()
        for package in REQUIRED_PACKAGES:
            with self.subTest(package=package):
                tree = "\n".join(
                    line for line in lines if line.split(" ", 1)[0] != package
                )
                status, stdout, stderr, _ = self.run_check(tree + "\n")

                self.assertEqual(status, 1)
                self.assertEqual(stdout, "")
                self.assertEqual(
                    stderr,
                    "linux TLS dependency tree: "
                    f"missing Rustls/WebPKI packages: {package}\n",
                )

    def test_empty_dependency_tree_fails_with_all_missing_requirements(self) -> None:
        status, stdout, stderr, _ = self.run_check("")

        self.assertEqual(status, 1)
        self.assertEqual(stdout, "")
        self.assertEqual(
            stderr,
            "linux TLS dependency tree: missing Rustls/WebPKI packages: "
            "rustls, tokio-rustls, webpki-roots\n",
        )

    def test_similarly_named_packages_do_not_trigger_false_positives(self) -> None:
        status, stdout, stderr, _ = self.run_check(
            PORTABLE_TREE
            + "native-tls-compatibility-notes v1.0.0\n"
            + "openssl-sys-test-fixture v1.0.0\n"
        )

        self.assertEqual(status, 0)
        self.assertEqual(stderr, "")
        self.assertIn("Rustls/WebPKI", stdout)

    def test_cargo_tree_failure_fails_closed_without_echoing_cargo_output(self) -> None:
        private_detail = "DO-NOT-ECHO-CARGO-DIAGNOSTIC"
        status, stdout, stderr, _ = self.run_check(
            PORTABLE_TREE,
            returncode=101,
            cargo_stderr=private_detail,
        )

        self.assertEqual(status, 1)
        self.assertEqual(stdout, "")
        self.assertEqual(stderr, "linux TLS dependency tree: cargo tree failed\n")
        self.assertNotIn(private_detail, stderr)


if __name__ == "__main__":
    unittest.main()
