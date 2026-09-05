#!/bin/sh
# Build/install only when the operator invokes this script deliberately.
# This job starts idle; /health never starts capture or requests TCC permission.
# Source gates run the fake-home tests, never this installer against real HOME.
set -eu
umask 077
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec python3 - "$SCRIPT_DIR/.." "$@" <<'PY'
"""Owner-only, serialized audio-tap installation with fail-closed rollback."""

import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time

LABEL = "com.donaldfilimon.abbey-audio-tap"
ADDRESS = "127.0.0.1:8182"
REPO = Path(sys.argv[1]).resolve()
UID = os.getuid()
DOMAIN = f"gui/{UID}"
SERVICE = f"{DOMAIN}/{LABEL}"
HOME_DIR = Path(os.environ["HOME"]).resolve()
INSTALL_DIR = HOME_DIR / ".local/libexec/abbey-bot/audio-tap"
STATE_DIR = HOME_DIR / ".local/share/abbey-bot/audio-tap"
LOG_DIR = HOME_DIR / "Library/Logs/abbey-bot/audio-tap"
AGENTS_DIR = HOME_DIR / "Library/LaunchAgents"
BINARY = INSTALL_DIR / "abbey-audio-tap"
PLIST = AGENTS_DIR / f"{LABEL}.plist"
LOCK = STATE_DIR / "install.lock"


class InstallError(Exception):
    pass


def command(args, *, check=True, timeout=15, env=None):
    result = subprocess.run(
        args, capture_output=True, text=True, timeout=timeout, env=env
    )
    if check and result.returncode:
        # Do not print arbitrary command output or the owner's environment.
        raise InstallError(f"{Path(args[0]).name} {args[1]} failed")
    return result


def present(target):
    return target.exists() or target.is_symlink()


def owned_path(target, *, directory=False, private=False):
    info = target.lstat()
    expected = stat.S_ISDIR if directory else stat.S_ISREG
    if not expected(info.st_mode) or info.st_uid != UID:
        raise InstallError(f"expected an owned {'directory' if directory else 'file'}: {target}")
    forbidden = 0o077 if private else 0o022
    if info.st_mode & forbidden:
        raise InstallError(f"unsafe permissions on {target}")
    if not directory and info.st_nlink != 1:
        raise InstallError(f"refusing multiply linked file: {target}")


def directory(target, *, private=False):
    # Inspect each home-relative ancestor before descending, never follow links
    # or chmod shared Abbey directories to repair an unexpected layout.
    owned_path(HOME_DIR, directory=True)
    current = HOME_DIR
    for part in target.relative_to(HOME_DIR).parts:
        current = current / part
        if not present(current):
            current.mkdir(mode=0o700)
        owned_path(current, directory=True, private=private and current == target)


def private_file(target):
    if present(target):
        owned_path(target, private=True)


def service_info():
    result = command(["launchctl", "print", SERVICE], check=False)
    if result.returncode:
        if result.returncode != 113 or (
            f'Could not find service "{LABEL}"' not in result.stderr
        ):
            raise InstallError("could not inspect audio-tap job; its unloaded state is unknown")
        # Failure of the domain itself is not evidence that a job is unloaded.
        command(["launchctl", "print", DOMAIN])
        return None
    return result.stdout


def service_pid():
    info = service_info()
    match = re.search(r"^\s*pid = ([1-9][0-9]*)\s*$", info or "", re.MULTILINE)
    return match.group(1) if match else None


def stop_service():
    if service_info() is None:
        return
    command(["launchctl", "bootout", SERVICE], check=False)
    for _ in range(20):
        if service_info() is None:
            return
        time.sleep(0.25)
    raise InstallError("audio-tap job is still loaded; refusing to mutate its files")


def listener_owned(pid):
    result = command(
        ["lsof", "-nP", "-a", "-p", pid, "-iTCP", "-sTCP:LISTEN", "-Fpn"],
        check=False,
    )
    rows = result.stdout.splitlines()
    return result.returncode == 0 and f"p{pid}" in rows and {
        row for row in rows if row.startswith("n")
    } == {f"n{ADDRESS}"}


def healthy_idle_service():
    # This only verifies the installed process and capture-free HTTP contract.
    # Never request /stream, run a capture smoke, or infer TCC/audio readiness.
    expected_pid = None
    for _ in range(20):
        pid = service_pid()
        if expected_pid is not None and pid != expected_pid:
            raise InstallError("audio-tap process changed during startup verification")
        if pid is not None:
            expected_pid = pid
            if listener_owned(pid):
                response = command(
                    # --disable must be first: owner curl config can add URLs
                    # or output writes before our fixed health-only transfer.
                    ["curl", "--disable", "--noproxy", "*", "--fail", "--silent", "--show-error",
                     "--connect-timeout", "1", "--max-time", "1",
                     f"http://{ADDRESS}/health"],
                    check=False,
                )
                if response.returncode == 0:
                    try:
                        health = json.loads(response.stdout)
                    except json.JSONDecodeError:
                        health = None
                    if (
                        isinstance(health, dict)
                        and health.get("service") == "abbey-audio-tap"
                        and type(health.get("protocol_version")) is int
                        and health["protocol_version"] == 1
                        and health.get("status") == "idle"
                        and health.get("ready") is False
                        and health.get("audio") == {
                            "sample_rate": 48000, "channels": 2, "format": "s16le"
                        }
                        and health.get("stream_path") == "/stream"
                    ):
                        if service_pid() != pid or not listener_owned(pid):
                            raise InstallError("audio-tap process/listener changed after health")
                        return
                    raise InstallError("audio-tap health did not identify an idle protocol-v1 service")
        time.sleep(0.25)
    raise InstallError("audio-tap did not publish a verified idle loopback service")


def interrupt(signum, _frame):
    raise InstallError(f"installation interrupted by signal {signum}")


def set_signal_handlers(handler):
    for sig in (signal.SIGHUP, signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, handler)


def copy_private(source, destination, mode):
    with source.open("rb") as incoming, destination.open("xb") as outgoing:
        shutil.copyfileobj(incoming, outgoing)
        outgoing.flush()
        os.fsync(outgoing.fileno())
    destination.chmod(mode)


def write_journal(transaction, phase):
    staged = transaction / "journal.tmp"
    with staged.open("w", encoding="utf-8") as handle:
        json.dump({"phase": phase, "pid": os.getpid()}, handle)
        handle.flush()
        os.fsync(handle.fileno())
    staged.replace(transaction / "journal.json")


def main():
    if sys.argv[2:] not in ([], ["--uninstall"]):
        raise InstallError("usage: deploy/install-audio-tap-launchd.sh [--uninstall]")
    uninstall = sys.argv[2:] == ["--uninstall"]
    if command(["uname", "-s"]).stdout.strip() != "Darwin":
        raise InstallError("Abbey's audio tap requires macOS 14 or later")
    version = command(["sw_vers", "-productVersion"]).stdout.strip().split(".")
    if not version[0].isdigit() or int(version[0]) < 14:
        raise InstallError("Abbey's audio tap requires macOS 14 or later")
    command(["launchctl", "print", DOMAIN])
    directory(STATE_DIR, private=True)
    # Ignore ordinary termination only while claiming/publishing the lock;
    # SIGKILL still leaves a recovery lock which must be inspected explicitly.
    set_signal_handlers(signal.SIG_IGN)
    try:
        LOCK.mkdir(mode=0o700)
    except FileExistsError as error:
        raise InstallError(f"an install/recovery lock exists: {LOCK}; inspect it before retrying") from error

    transaction = None
    switching = False
    keep_recovery = False
    previous_loaded = False
    previous = {}
    try:
        (LOCK / "pid").write_text(f"{os.getpid()}\n", encoding="ascii")
        set_signal_handlers(interrupt)
        directory(INSTALL_DIR, private=True)
        directory(AGENTS_DIR)
        directory(LOG_DIR, private=True)
        private_file(LOG_DIR / "service.log")
        if not present(LOG_DIR / "service.log"):
            (LOG_DIR / "service.log").touch(mode=0o600, exist_ok=False)
        for target in (BINARY, PLIST):
            private_file(target)
        previous_loaded = service_info() is not None
        if previous_loaded and not all(present(target) for target in (BINARY, PLIST)):
            raise InstallError("loaded audio-tap job has no complete owned install; refusing replacement")
        transaction = Path(tempfile.mkdtemp(prefix="install-", dir=STATE_DIR))
        write_journal(transaction, "staging")
        for target in (BINARY, PLIST):
            previous[target] = present(target)
            if previous[target]:
                copy_private(target, transaction / f"previous-{target.name}",
                             0o700 if target == BINARY else 0o600)

        if not uninstall:
            build = transaction / "build"
            build_env = dict(os.environ)
            build_env.pop("TOOLCHAINS", None)
            args = ["swift", "build", "--package-path", str(REPO / "tools/abbey-audio-tap"),
                    "--scratch-path", str(build), "--configuration", "release"]
            command([*args, "--product", "abbey-audio-tap"], timeout=1200, env=build_env)
            bin_dir = Path(command([*args, "--show-bin-path"], env=build_env).stdout.strip())
            product = bin_dir / "abbey-audio-tap"
            if not product.resolve().is_relative_to(build.resolve()):
                raise InstallError("Swift product escaped the transaction's build directory")
            owned_path(product)
            copy_private(product, transaction / "candidate-binary", 0o700)
            version_output = command([str(transaction / "candidate-binary"), "--version"]).stdout
            if not version_output.startswith("abbey-audio-tap "):
                raise InstallError("built product did not identify itself as abbey-audio-tap")
            template = plistlib.loads((REPO / "deploy" / f"{LABEL}.plist").read_bytes())
            def substitute(value):
                if isinstance(value, str):
                    return value.replace("__HOME__", str(HOME_DIR))
                if isinstance(value, list):
                    return [substitute(item) for item in value]
                if isinstance(value, dict):
                    return {key: substitute(item) for key, item in value.items()}
                return value
            candidate_plist = transaction / "candidate-plist"
            with candidate_plist.open("xb") as handle:
                plistlib.dump(substitute(template), handle)
            command(["plutil", "-lint", str(candidate_plist)])

        write_journal(transaction, "switching")
        switching = True
        stop_service()
        for target in (BINARY, PLIST):
            private_file(target)
        if uninstall:
            for target in (PLIST, BINARY):
                if present(target):
                    target.unlink()
        else:
            (transaction / "candidate-binary").replace(BINARY)
            (transaction / "candidate-plist").replace(PLIST)
            command(["launchctl", "bootstrap", DOMAIN, str(PLIST)])
            healthy_idle_service()
        write_journal(transaction, "uninstalled" if uninstall else "installed-idle")
        switching = False
        print("Audio tap uninstalled; logs and previous artifacts retained." if uninstall else
              "Audio tap installed, idle at http://127.0.0.1:8182. "
              "No audio capture, macOS permission, or Discord playback was verified.")
        if any(previous.values()):
            print(f"Previous artifacts retained at {transaction}")
    except BaseException:
        if switching:
            set_signal_handlers(signal.SIG_IGN)
            try:
                stop_service()
                write_journal(transaction, "restoring")
                for target, existed in previous.items():
                    private_file(target)
                    if existed:
                        backup = transaction / f"previous-{target.name}"
                        owned_path(backup, private=True)
                        backup.replace(target)
                    elif present(target):
                        target.unlink()
                if previous_loaded:
                    command(["launchctl", "bootstrap", DOMAIN, str(PLIST)])
                    healthy_idle_service()
                write_journal(transaction, "restored")
            except BaseException as recovery_error:
                keep_recovery = True
                print(f"Rollback incomplete: {recovery_error}. "
                      f"Recovery files retained at {transaction}; lock retained at {LOCK}.",
                      file=sys.stderr)
        raise
    finally:
        set_signal_handlers(signal.SIG_IGN)
        if not keep_recovery:
            if transaction is not None:
                # Only our unique scratch is removable, never a shared Swift cache.
                shutil.rmtree(transaction / "build", ignore_errors=True)
                if not any(previous.values()):
                    shutil.rmtree(transaction)
            (LOCK / "pid").unlink(missing_ok=True)
            LOCK.rmdir()


try:
    main()
except (InstallError, OSError, subprocess.SubprocessError, ValueError) as error:
    print(f"audio-tap install failed: {error}", file=sys.stderr)
    sys.exit(1)
PY
