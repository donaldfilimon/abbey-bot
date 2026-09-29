#!/usr/bin/env python3
"""Managed-service contract gate for abbey-bot-zig.

Validates, with the oracle's own (identity-substituted) Python validators in
deploy/:
  * the Zig-published readiness and bootstrap documents (bytes, modes, path),
  * the executable digest in them against the built binary,
  * the launchd plist's launch contract,
  * `serve` credential selection and the refusal beside the live Rust service.
Only temp HOMEs are used; no real token is ever read or sent.
"""
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "deploy"))
import service_installation as installation  # noqa: E402
import service_protocol as protocol  # noqa: E402

BINARY = ROOT / "zig-out/bin/abbey-bot-zig"
LIVE_LABEL = "com.donaldfilimon.abbey-bot"


def fail(message):
    print(f"FAIL: {message}")
    raise SystemExit(1)


def run(args, env):
    return subprocess.run([str(BINARY), *args], env=env, capture_output=True, text=True, timeout=60)


def check_env_contract():
    import re
    keys = re.findall(r"^([A-Z][A-Z0-9_]*)=", (ROOT / ".env.example").read_text(), re.M)
    src = (ROOT / "src/service/config.zig").read_text() + (ROOT / "src/episode/config.zig").read_text()
    unread = [k for k in keys if f'"{k}"' not in src]
    if not keys or unread:
        fail(f".env.example keys with no reader: {unread}")
    print(f".env.example: {len(keys)} keys, each read by src/service/config.zig or src/episode/config.zig")


def main():
    check_env_contract()
    digest = hashlib.sha256(BINARY.read_bytes()).hexdigest()
    with tempfile.TemporaryDirectory(prefix="abbey-bot-zig-managed-") as tmp:
        home = Path(tmp).resolve()
        os.chmod(home, 0o700)
        sample = run(["readiness-sample", str(home)], {"PATH": "/usr/bin:/bin"})
        if sample.returncode != 0:
            fail(f"readiness-sample exited {sample.returncode}: {sample.stdout.strip()}")
        ready = protocol.read_private(home, "readiness")
        boot = protocol.read_private(home, "bootstrap")
        raw = (home / ".local/share/abbey-bot-zig/readiness.json").read_bytes()
        if protocol.encode_document(ready) != raw:
            fail("readiness bytes are not the canonical v1 encoding")
        for doc in (ready, boot):
            if doc["executable_sha256"] != digest:
                fail("executable_sha256 does not match the built binary")
        if protocol.identity(ready) != protocol.identity(boot):
            fail("readiness and bootstrap carry different run identities")
        if ready["phase"] != "starting" or boot["phase"] != "starting":
            fail("sample phases are not starting")
        print("readiness + bootstrap: parsed by service_protocol.read_private (0700 dir, 0600 files), digest matches the binary")

        plist = (ROOT / "deploy/com.donaldfilimon.abbey-bot-zig.plist").read_text()
        installation.validate_managed_plist(plist.replace("__HOME__", str(home)).encode(), home)
        oracle_label = plist.replace("__HOME__", str(home)).replace(
            "<string>com.donaldfilimon.abbey-bot-zig</string>", f"<string>{LIVE_LABEL}</string>")
        try:
            installation.validate_managed_plist(oracle_label.encode(), home)
        except protocol.ProtocolError:
            pass
        else:
            fail("the plist validator accepted the live Rust bot's label")
        print("plist: validate_managed_plist accepts the abbey-bot-zig launch contract and rejects the live label")

        env = {"HOME": str(home), "PATH": "/usr/bin:/bin"}
        missing = run(["serve"], env)
        want = "Neither DISCORD_TOKEN nor DISCORD_BOT_TOKEN is set. Export one bot token; never hardcode it.\n"
        if missing.returncode != 1 or missing.stderr != want:
            fail(f"serve without a token: exit {missing.returncode}, stderr {missing.stderr!r}")
        blank = run(["--managed-service"], {**env, "DISCORD_TOKEN": "  ", "DISCORD_BOT_TOKEN": "x"})
        if blank.returncode != 1 or "refusing to consult DISCORD_BOT_TOKEN" not in blank.stderr:
            fail(f"blank primary: exit {blank.returncode}, stderr {blank.stderr!r}")
        print("serve: missing and blank credentials fail closed with the oracle's messages")

        live = subprocess.run(["/bin/launchctl", "print", f"gui/{os.getuid()}/{LIVE_LABEL}"],
                              capture_output=True, timeout=10).returncode == 0
        if live:
            refused = run(["serve"], {**env, "DISCORD_TOKEN": "placeholder-not-a-token"})
            if refused.returncode != 3 or not refused.stderr.startswith("Refusing to start"):
                fail(f"serve beside the live service: exit {refused.returncode}, stderr {refused.stderr!r}")
            print(f"serve: refused beside the loaded {LIVE_LABEL} before any network I/O (exit 3)")
        else:
            print(f"SKIP: {LIVE_LABEL} is not loaded here; the refusal is covered by the Zig unit test only")


if __name__ == "__main__":
    main()
