#!/usr/bin/env python3
"""Pin the hardening and secret hygiene of deploy/abbey-bot.service.

This host has no systemd, so nothing else parses the unit. The check is
textual: it proves the reviewed settings are present with the reviewed values,
not that a given systemd version accepts them (`systemd-analyze verify` on the
target host does that).
"""

from __future__ import annotations

import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
UNIT = ROOT / "deploy" / "abbey-bot.service"

REQUIRED_SERVICE = {
    "DynamicUser": "yes",
    "StateDirectory": "abbey-bot",
    "StateDirectoryMode": "0700",
    "UMask": "0077",
    "EnvironmentFile": "/etc/abbey-bot/env",
    "Restart": "on-failure",
    "NoNewPrivileges": "yes",
    "ProtectSystem": "strict",
    "ProtectHome": "yes",
    "PrivateTmp": "yes",
    "PrivateDevices": "yes",
    "ProtectKernelTunables": "yes",
    "ProtectKernelModules": "yes",
    "ProtectKernelLogs": "yes",
    "ProtectControlGroups": "yes",
    "ProtectClock": "yes",
    "ProtectHostname": "yes",
    "ProtectProc": "invisible",
    "RestrictNamespaces": "yes",
    "RestrictRealtime": "yes",
    "RestrictSUIDSGID": "yes",
    "RestrictAddressFamilies": "AF_UNIX AF_INET AF_INET6",
    "LockPersonality": "yes",
    "MemoryDenyWriteExecute": "yes",
    "SystemCallArchitectures": "native",
    "SystemCallFilter": "@system-service",
    "SystemCallErrorNumber": "EPERM",
    "CapabilityBoundingSet": "",
}
SECRET_NAME = re.compile(r"TOKEN|SECRET|PASSWORD|API_?KEY", re.IGNORECASE)
MIN_RESTART_SEC = 30


def parse(text: str) -> dict[str, dict[str, list[str]]]:
    sections: dict[str, dict[str, list[str]]] = {}
    current: dict[str, list[str]] | None = None
    for number, raw in enumerate(text.splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith(("#", ";")):
            continue
        if line.startswith("[") and line.endswith("]"):
            current = sections.setdefault(line[1:-1], {})
            continue
        if current is None or "=" not in line:
            raise ValueError(f"line {number}: not a key=value inside a section")
        key, value = line.split("=", 1)
        current.setdefault(key.strip(), []).append(value.strip())
    return sections


def problems(text: str) -> list[str]:
    try:
        sections = parse(text)
    except ValueError as error:
        return [str(error)]
    found = [f"missing [{name}]" for name in ("Unit", "Service", "Install") if name not in sections]
    service = sections.get("Service", {})
    for key, expected in REQUIRED_SERVICE.items():
        values = service.get(key)
        if values is None:
            found.append(f"{key} missing")
        elif values != [expected]:
            found.append(f"{key}={values!r}, expected exactly {expected!r}")
    exec_start = service.get("ExecStart", [""])
    if len(exec_start) != 1 or not exec_start[0].startswith("/"):
        found.append("ExecStart must be one absolute path")
    restart_sec = service.get("RestartSec", [""])
    if len(restart_sec) != 1 or not restart_sec[0].isdigit() or int(restart_sec[0]) < MIN_RESTART_SEC:
        found.append(f"RestartSec must be one integer >= {MIN_RESTART_SEC}")
    for key in ("Environment", "PassEnvironment", "SetCredential"):
        for value in service.get(key, []):
            if SECRET_NAME.search(value):
                found.append(f"{key} carries a secret-looking name; secrets belong in the EnvironmentFile")
    if sections.get("Install", {}).get("WantedBy") != ["multi-user.target"]:
        found.append("WantedBy must be multi-user.target")
    return found


def main() -> None:
    found = problems(UNIT.read_text(encoding="utf-8"))
    if found:
        for problem in found:
            print(f"systemd unit: {problem}", file=sys.stderr)
        raise SystemExit(1)
    print(f"systemd unit: {UNIT.relative_to(ROOT)} ok ({len(REQUIRED_SERVICE)} pinned settings)")


if __name__ == "__main__":
    main()
