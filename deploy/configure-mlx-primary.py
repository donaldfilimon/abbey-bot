#!/usr/bin/env python3
"""Atomically select the pinned MLX-VLM primary without exposing env values."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import signal
import stat
import subprocess
import time


MODEL_REVISION = "73bcf09092aa277861d5a191b989b666f7f32e8f"
MODEL_REPOSITORY_DIR = "models--mlx-community--gemma-4-12B-it-4bit"
FM_CLI = Path("/usr/bin/fm")
QUALIFICATION_VERSION = 1
FIXTURE_VERSION = "abbey-provider-fixtures-v1"
MAX_MANIFEST_BYTES = 256 * 1024
MANAGED_HEADER = "# Qualified local providers; managed by deploy/configure-mlx-primary.py."
MANAGED_KEYS = (
    "ANTHROPIC_API_KEY",
    "ABBEY_BOT_LLM_ENDPOINT",
    "ABBEY_BOT_LLM_MODEL",
    "ABBEY_BOT_LLM_TOOLS",
    "ABBEY_VISION_PROVIDER",
    "ABBEY_VISION_ENDPOINT",
    "ABBEY_VISION_MODEL",
    "ABBEY_VISION_KEY",
    "ABBEY_FM_MODE",
    "ABBEY_FM_ENDPOINT",
    "ABBEY_FM_CLI",
    "ABBEY_FM_FALLBACK",
    "ABBEY_FM_CAPABILITY_MANIFEST",
)
DEACTIVATED_SECRET_KEYS = {"ANTHROPIC_API_KEY", "ABBEY_VISION_KEY"}
ASSIGNMENT = re.compile(r"^(?:export[ \t]+)?([A-Za-z_][A-Za-z0-9_]*)=")
LAUNCHCTL = Path("/bin/launchctl")
LAUNCHD_LABEL = "com.donaldfilimon.abbey-bot"
START_TIMEOUT_SECS = 15.0
STABLE_OBSERVATIONS = 3
STABLE_INTERVAL_SECS = 1.0
MASKED_SIGNALS = {
    candidate
    for candidate in (
        getattr(signal, "SIGHUP", None),
        getattr(signal, "SIGINT", None),
        getattr(signal, "SIGTERM", None),
    )
    if candidate is not None
}


def fail(message: str) -> "None":
    raise SystemExit(message)


class LockFailure(RuntimeError):
    pass


def require_private_directory(path: Path, description: str) -> None:
    try:
        metadata = path.lstat()
    except FileNotFoundError as error:
        raise LockFailure(f"{description} is missing: {path}") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise LockFailure(f"{description} must be a directory, not a symlink: {path}")
    if metadata.st_uid != os.getuid():
        raise LockFailure(f"{description} must be owned by the current user: {path}")
    if stat.S_IMODE(metadata.st_mode) & 0o077:
        raise LockFailure(f"{description} must be private and owner-only: {path}")


class InstallLock:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.owner_file = path / "pid"
        self.held = False
        self.old_signal_mask: set[signal.Signals] | None = None

    def _mask_signals(self) -> None:
        try:
            self.old_signal_mask = signal.pthread_sigmask(signal.SIG_BLOCK, MASKED_SIGNALS)
        except (AttributeError, OSError) as error:
            raise LockFailure("could not mask signals for the install transaction") from error

    def _restore_signal_mask(self) -> None:
        if self.old_signal_mask is None:
            return
        signal.pthread_sigmask(signal.SIG_SETMASK, self.old_signal_mask)
        self.old_signal_mask = None

    def _publish_owner(self) -> None:
        temporary = self.path / f".pid.new.{os.getpid()}.{secrets.token_hex(4)}"
        try:
            descriptor = os.open(
                temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600
            )
            with os.fdopen(descriptor, "w", encoding="ascii", newline="\n") as output:
                output.write(f"{os.getpid()}\n")
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, self.owner_file)
            fsync_directory(self.path)
        finally:
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass

    def __enter__(self) -> "InstallLock":
        self._mask_signals()
        created = False
        try:
            if not self.path.is_absolute():
                raise LockFailure("the Abbey install-lock path must be absolute")
            require_private_directory(
                self.path.parent, "the Abbey install-lock parent"
            )
            try:
                self.path.mkdir(mode=0o700)
            except FileExistsError as error:
                raise LockFailure(
                    f"another Abbey install transaction holds {self.path}; "
                    "never remove a stale lock without operator verification"
                ) from error
            created = True
            self.path.chmod(0o700)
            require_private_directory(self.path, "the Abbey install lock")
            self._publish_owner()
            self.held = True
            return self
        except BaseException:
            cleaned = not created
            if created:
                try:
                    self.owner_file.unlink(missing_ok=True)
                    self.path.rmdir()
                    cleaned = True
                except OSError:
                    cleaned = False
                    try:
                        self._publish_owner()
                        self.held = True
                    except BaseException:
                        pass
            if cleaned:
                self._restore_signal_mask()
            raise

    def release(self) -> None:
        if not self.held:
            self._restore_signal_mask()
            return
        try:
            metadata = self.owner_file.lstat()
            owner = self.owner_file.read_text(encoding="ascii")
        except (FileNotFoundError, OSError, UnicodeError) as error:
            raise LockFailure(
                f"refusing to release Abbey install lock without its owner record: {self.path}"
            ) from error
        if (
            stat.S_ISLNK(metadata.st_mode)
            or not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or stat.S_IMODE(metadata.st_mode) & 0o077
            or owner != f"{os.getpid()}\n"
        ):
            raise LockFailure(
                f"refusing to release Abbey install lock not owned by this process: {self.path}"
            )
        try:
            self.owner_file.unlink()
            fsync_directory(self.path)
            self.path.rmdir()
        except OSError as error:
            try:
                self._publish_owner()
            except BaseException as restore_error:
                raise LockFailure(
                    "failed to release the Abbey install lock and could not restore "
                    f"its owner record: {self.path}"
                ) from restore_error
            raise LockFailure(
                f"failed to release the Abbey install lock; owner record restored: {self.path}"
            ) from error
        self.held = False
        self._restore_signal_mask()

    def __exit__(self, _type: object, _value: object, _traceback: object) -> None:
        self.release()


def require_private_regular_file(path: Path, description: str) -> os.stat_result:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        fail(f"{description} is missing: {path}")
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        fail(f"{description} must be a regular file, not a symlink: {path}")
    if metadata.st_uid != os.getuid():
        fail(f"{description} must be owned by the current user: {path}")
    if stat.S_IMODE(metadata.st_mode) & 0o077:
        fail(f"{description} must not be readable or writable by group/other: {path}")
    return metadata


def require_owned_executable(path: Path) -> None:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        fail(f"Abbey binary is missing: {path}")
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        fail(f"Abbey binary must be a regular file, not a symlink: {path}")
    if metadata.st_uid != os.getuid():
        fail(f"Abbey binary must be owned by the current user: {path}")
    if not os.access(path, os.X_OK):
        fail(f"Abbey binary is not executable: {path}")


def validate_launchctl(path: Path, explicit: bool) -> Path:
    if not path.is_absolute():
        fail("the launchctl path must be absolute")
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        fail(f"launchctl is missing: {path}")
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        fail(f"launchctl must be a regular file, not a symlink: {path}")
    expected_owner = os.getuid() if explicit else 0
    if metadata.st_uid != expected_owner:
        owner = "the current user" if explicit else "root"
        fail(f"launchctl must be owned by {owner}: {path}")
    if not os.access(path, os.X_OK):
        fail(f"launchctl is not executable: {path}")
    if not explicit and path != LAUNCHCTL:
        fail(f"production restart requires the pinned launchctl path: {LAUNCHCTL}")
    return path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(64 * 1024), b""):
                digest.update(chunk)
    except OSError:
        fail("could not hash a qualification-bound executable")
    return digest.hexdigest()


def current_os_build() -> str:
    if not FM_CLI.is_file():
        fail("the pinned /usr/bin/fm executable is unavailable")
    try:
        completed = subprocess.run(
            ["/usr/bin/sw_vers", "-buildVersion"],
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env={},
            text=True,
        )
    except OSError:
        fail("could not read the macOS build identity")
    build = completed.stdout.strip()
    if completed.returncode != 0 or not build:
        fail("could not read the macOS build identity")
    return build


def capability_passed(entry: object, name: str) -> bool:
    if not isinstance(entry, dict):
        return False
    capabilities = entry.get("capabilities")
    if not isinstance(capabilities, dict):
        return False
    evidence = capabilities.get(name)
    return isinstance(evidence, dict) and evidence.get("status") == "pass"


def identity_matches(
    identity: object,
    *,
    binary_hash: str,
    os_build: str,
    endpoint: str | None,
    model: str | None,
    cli_path: str | None,
    cli_hash: str | None,
    mode: str | None,
) -> bool:
    return isinstance(identity, dict) and all(
        (
            identity.get("abbey_binary_sha256") == binary_hash,
            identity.get("os_build") == os_build,
            identity.get("fixture_version") == FIXTURE_VERSION,
            identity.get("endpoint") == endpoint,
            identity.get("model") == model,
            identity.get("cli_path") == cli_path,
            identity.get("cli_sha256") == cli_hash,
            identity.get("mode") == mode,
        )
    )


def validate_manifest(manifest: Path, binary: Path, model_dir: Path) -> None:
    metadata = require_private_regular_file(manifest, "FM capability manifest")
    if metadata.st_size > MAX_MANIFEST_BYTES:
        fail("FM capability manifest exceeds the 256 KiB limit")
    try:
        report = json.loads(manifest.read_bytes())
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        fail("FM capability manifest is malformed")
    if not isinstance(report, dict):
        fail("FM capability manifest is malformed")
    if (
        type(report.get("version")) is not int
        or report.get("version") != QUALIFICATION_VERSION
        or report.get("fixture_version") != FIXTURE_VERSION
        or report.get("target") != "all"
        or report.get("overall_pass") is not True
    ):
        fail("FM capability manifest is not passing target=all evidence")
    generated = report.get("generated_unix_secs")
    if (
        type(generated) is not int
        or generated < 0
        or generated > int(time.time()) + 300
    ):
        fail("FM capability manifest timestamp is invalid")

    binary_hash = sha256(binary)
    fm_hash = sha256(FM_CLI)
    os_build = current_os_build()
    model = str(model_dir)
    primary = report.get("primary")
    fm_server = report.get("fm_server")
    fm_cli = report.get("fm_cli")
    if not isinstance(primary, dict) or primary.get("configured") is not True:
        fail("FM capability manifest omits the configured primary route")
    if not identity_matches(
        primary.get("identity"),
        binary_hash=binary_hash,
        os_build=os_build,
        endpoint="http://127.0.0.1:8282",
        model=model,
        cli_path=None,
        cli_hash=None,
        mode=None,
    ):
        fail("FM capability manifest primary identity does not match this cutover")
    if not identity_matches(
        primary.get("vision_identity"),
        binary_hash=binary_hash,
        os_build=os_build,
        endpoint="http://127.0.0.1:8282/v1",
        model=model,
        cli_path=None,
        cli_hash=None,
        mode=None,
    ):
        fail("FM capability manifest primary vision identity does not match this cutover")
    if not all(
        capability_passed(primary, capability)
        for capability in (
            "text",
            "streaming",
            "structured_output",
            "tools",
            "vision",
            "ocr",
        )
    ):
        fail("FM capability manifest lacks a required primary capability pass")

    if not isinstance(fm_server, dict) or fm_server.get("configured") is not False:
        fail("FM capability manifest must not configure an FM server endpoint")
    if not isinstance(fm_cli, dict) or fm_cli.get("configured") is not True:
        fail("FM capability manifest omits the configured FM CLI route")
    expected_fm_identity = dict(
        binary_hash=binary_hash,
        os_build=os_build,
        endpoint=None,
        model=None,
        cli_path=str(FM_CLI),
        cli_hash=fm_hash,
        mode="system",
    )
    if not identity_matches(fm_cli.get("identity"), **expected_fm_identity):
        fail("FM capability manifest FM CLI identity does not match this cutover")
    if not all(
        capability_passed(fm_cli, capability)
        for capability in ("text", "structured_output", "tools")
    ):
        fail("FM capability manifest lacks a required FM CLI capability pass")
    if any(
        capability_passed(fm_cli, capability) for capability in ("vision", "ocr")
    ) and not identity_matches(fm_cli.get("vision_identity"), **expected_fm_identity):
        fail("FM capability manifest FM image identity does not match this cutover")


def validate_model_dir(path: Path) -> Path:
    if not path.is_absolute():
        fail("the MLX-VLM model directory must be absolute")
    try:
        resolved = path.resolve(strict=True)
    except FileNotFoundError:
        fail(f"the pinned MLX-VLM model directory is missing: {path}")
    if not resolved.is_dir() or resolved.name != MODEL_REVISION:
        fail(f"the MLX-VLM model directory must end in the pinned revision {MODEL_REVISION}")
    parts = resolved.parts
    try:
        repository_index = parts.index(MODEL_REPOSITORY_DIR)
    except ValueError:
        fail(f"the MLX-VLM model directory is not from {MODEL_REPOSITORY_DIR}")
    if parts[repository_index + 1 : repository_index + 2] != ("snapshots",):
        fail("the MLX-VLM model directory is not an exact Hugging Face snapshot path")
    required = resolved / "model.safetensors.index.json"
    if not required.is_file():
        fail(f"the pinned MLX-VLM snapshot is incomplete: {required}")
    return resolved


def rendered_environment(original: str, model_dir: Path, manifest: Path) -> str:
    kept: list[str] = []
    seen_managed: set[str] = set()
    for line in original.splitlines():
        if line == MANAGED_HEADER:
            continue
        match = ASSIGNMENT.match(line.strip())
        key = match.group(1) if match else None
        if key in MANAGED_KEYS:
            if key in seen_managed:
                fail(f"the environment contains duplicate managed key {key}")
            seen_managed.add(key)
            prior_value = line.split("=", 1)[1].strip()
            if key in DEACTIVATED_SECRET_KEYS and prior_value not in {"", "''", '""'}:
                kept.extend(
                    (
                        "# Disabled by the MLX primary cutover; preserved for rollback.",
                        f"# {line}",
                    )
                )
            continue
        kept.append(line)

    while kept and not kept[-1].strip():
        kept.pop()
    values = {
        "ANTHROPIC_API_KEY": "",
        "ABBEY_BOT_LLM_ENDPOINT": "http://127.0.0.1:8282",
        "ABBEY_BOT_LLM_MODEL": str(model_dir),
        "ABBEY_BOT_LLM_TOOLS": "on",
        "ABBEY_VISION_PROVIDER": "remote",
        "ABBEY_VISION_ENDPOINT": "http://127.0.0.1:8282/v1",
        "ABBEY_VISION_MODEL": str(model_dir),
        "ABBEY_VISION_KEY": "",
        "ABBEY_FM_MODE": "system",
        "ABBEY_FM_ENDPOINT": "",
        "ABBEY_FM_CLI": str(FM_CLI),
        "ABBEY_FM_FALLBACK": "1",
        "ABBEY_FM_CAPABILITY_MANIFEST": str(manifest),
    }
    kept.extend(
        [
            "",
            MANAGED_HEADER,
            *(f"{key}={shlex.quote(value)}" for key, value in values.items()),
        ]
    )
    return "\n".join(kept) + "\n"


def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def publish(
    env_file: Path, backup_dir: Path, original: bytes, content: str
) -> Path:
    if env_file.read_bytes() != original:
        fail("the Abbey environment changed after validation; no cutover was applied")
    backup_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    backup_metadata = backup_dir.lstat()
    if not stat.S_ISDIR(backup_metadata.st_mode) or stat.S_ISLNK(backup_metadata.st_mode):
        fail(f"the environment backup path must be a directory, not a symlink: {backup_dir}")
    if backup_metadata.st_uid != os.getuid() or stat.S_IMODE(backup_metadata.st_mode) & 0o077:
        fail(f"the environment backup directory must be private and owner-only: {backup_dir}")

    serial = f"{int(time.time())}.{os.getpid()}.{secrets.token_hex(4)}"
    backup = backup_dir / f"env.before-mlx.{serial}"
    temporary = env_file.parent / f".{env_file.name}.new.{serial}"
    replaced = False
    try:
        backup_descriptor = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(backup_descriptor, "wb") as output:
            output.write(original)
            output.flush()
            os.fsync(output.fileno())

        temporary_descriptor = os.open(
            temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600
        )
        with os.fdopen(temporary_descriptor, "w", encoding="utf-8", newline="\n") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, env_file)
        replaced = True
        fsync_directory(env_file.parent)
        fsync_directory(backup_dir)
    except BaseException:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass
        if replaced:
            try:
                restore(env_file, backup)
            except BaseException:
                fail(
                    "environment publication failed after replacement and the previous "
                    f"environment could not be restored; rollback copy retained: {backup}"
                )
        raise
    return backup


def restore(env_file: Path, backup: Path) -> None:
    require_private_regular_file(backup, "Abbey environment rollback copy")
    serial = f"{int(time.time())}.{os.getpid()}.{secrets.token_hex(4)}"
    temporary = env_file.parent / f".{env_file.name}.restore.{serial}"
    try:
        original = backup.read_bytes()
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            output.write(original)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, env_file)
        fsync_directory(env_file.parent)
    except BaseException:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass
        raise


def service_target() -> str:
    return f"gui/{os.getuid()}/{LAUNCHD_LABEL}"


def service_pid(launchctl: Path) -> int | None:
    try:
        completed = subprocess.run(
            [str(launchctl), "print", service_target()],
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
    except OSError:
        return None
    if completed.returncode != 0:
        return None
    for line in completed.stdout.splitlines():
        match = re.fullmatch(r"\s*pid = ([1-9][0-9]*)\s*", line)
        if match:
            return int(match.group(1))
    return None


def restart_and_require_stable(launchctl: Path, previous_pid: int) -> int:
    try:
        restarted = subprocess.run(
            [str(launchctl), "kickstart", "-k", service_target()],
            check=False,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except OSError as error:
        raise RuntimeError("launchd restart could not be invoked") from error
    if restarted.returncode != 0:
        raise RuntimeError("launchd rejected the restart")

    deadline = time.monotonic() + START_TIMEOUT_SECS
    candidate = None
    while time.monotonic() < deadline:
        observed = service_pid(launchctl)
        if observed is not None and observed != previous_pid:
            candidate = observed
            break
        time.sleep(STABLE_INTERVAL_SECS)
    if candidate is None:
        raise RuntimeError("launchd did not publish a new Abbey process")

    for _ in range(STABLE_OBSERVATIONS - 1):
        time.sleep(STABLE_INTERVAL_SECS)
        if service_pid(launchctl) != candidate:
            raise RuntimeError("the restarted Abbey process was not stable")
    return candidate


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="atomically configure Abbey for the pinned MLX-VLM primary"
    )
    parser.add_argument(
        "--env-file",
        type=Path,
        default=Path.home() / ".config" / "abbey-bot" / "env",
    )
    parser.add_argument("--model-dir", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument(
        "--backup-dir",
        type=Path,
        default=Path.home() / ".local" / "share" / "abbey-bot" / "env-backups",
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--dry-run",
        action="store_true",
        help="validate and report managed key names without changing the environment",
    )
    mode.add_argument(
        "--apply-and-restart",
        action="store_true",
        help="atomically publish the environment and verify a stable launchd restart",
    )
    parser.add_argument(
        "--launchctl",
        type=Path,
        help=argparse.SUPPRESS,
    )
    parser.add_argument(
        "--install-lock",
        type=Path,
        default=Path.home() / ".local" / "share" / "abbey-bot" / "install.lock",
        help=argparse.SUPPRESS,
    )
    return parser.parse_args()


def run_locked(args: argparse.Namespace) -> None:
    env_file = args.env_file
    if (
        not env_file.is_absolute()
        or not args.manifest.is_absolute()
        or not args.binary.is_absolute()
    ):
        fail("the environment, qualification manifest, and binary paths must be absolute")
    require_private_regular_file(env_file, "Abbey environment")
    require_owned_executable(args.binary)
    model_dir = validate_model_dir(args.model_dir)
    validate_manifest(args.manifest, args.binary, model_dir)
    original = env_file.read_bytes()
    content = rendered_environment(
        original.decode("utf-8"), model_dir, args.manifest.resolve(strict=True)
    )
    if args.dry_run:
        print("validated MLX primary cutover")
        print("managed keys: " + ", ".join(MANAGED_KEYS))
        return
    launchctl = validate_launchctl(args.launchctl or LAUNCHCTL, args.launchctl is not None)
    previous_pid = service_pid(launchctl)
    if previous_pid is None:
        fail("the existing Abbey launchd service has no running process")
    backup = publish(env_file, args.backup_dir, original, content)
    try:
        new_pid = restart_and_require_stable(launchctl, previous_pid)
    except BaseException:
        try:
            restore(env_file, backup)
        except BaseException:
            fail(
                "candidate restart failed and the previous environment could not be "
                f"restored; rollback copy retained: {backup}"
            )
        rollback_pid = service_pid(launchctl)
        if rollback_pid is None:
            rollback_pid = previous_pid
        try:
            restart_and_require_stable(launchctl, rollback_pid)
        except BaseException:
            fail(
                "candidate restart failed; the previous environment was restored but "
                f"its service restart failed; rollback copy retained: {backup}"
            )
        fail(
            "candidate restart failed; the previous environment and service were "
            f"restored; rollback copy retained: {backup}"
        )
    print(f"updated owner-only Abbey environment: {env_file}")
    print(f"verified stable Abbey launchd process: {new_pid}")
    print(f"retained owner-only rollback copy: {backup}")


def main() -> None:
    args = arguments()
    try:
        with InstallLock(args.install_lock):
            run_locked(args)
    except LockFailure as error:
        fail(str(error))


if __name__ == "__main__":
    main()
