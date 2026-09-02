#!/usr/bin/env python3
"""Fail unless Abbey's Linux dependency graph is Rustls/WebPKI-only."""

from __future__ import annotations

import pathlib
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parent.parent
LINUX_TARGET = "x86_64-unknown-linux-gnu"
FORBIDDEN = {
    "hyper-tls",
    "native-tls",
    "openssl",
    "openssl-macros",
    "openssl-probe",
    "openssl-sys",
    "tokio-native-tls",
}
REQUIRED = {"rustls", "tokio-rustls", "webpki-roots"}


def main() -> int:
    result = subprocess.run(
        [
            "cargo",
            "tree",
            "--locked",
            "--target",
            LINUX_TARGET,
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
    if result.returncode != 0:
        print("linux TLS dependency tree: cargo tree failed", file=sys.stderr)
        return 1

    packages = {
        line.split(" ", 1)[0]
        for line in result.stdout.splitlines()
        if line and not line.startswith("[")
    }
    forbidden = sorted(packages & FORBIDDEN)
    if forbidden:
        print(
            "linux TLS dependency tree: forbidden packages: " + ", ".join(forbidden),
            file=sys.stderr,
        )
        return 1

    missing = sorted(REQUIRED - packages)
    if missing:
        print(
            "linux TLS dependency tree: missing Rustls/WebPKI packages: "
            + ", ".join(missing),
            file=sys.stderr,
        )
        return 1

    print("linux TLS dependency tree: OK (Rustls/WebPKI; native TLS and OpenSSL absent)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
