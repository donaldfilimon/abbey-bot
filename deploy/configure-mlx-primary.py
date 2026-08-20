#!/usr/bin/env python3
"""Atomically select the pinned MLX-VLM primary without exposing env values."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import secrets
import shlex
import stat
import time


MODEL_REVISION = "73bcf09092aa277861d5a191b989b666f7f32e8f"
MODEL_REPOSITORY_DIR = "models--mlx-community--gemma-4-12B-it-4bit"
MANAGED_KEYS = (
    "ANTHROPIC_API_KEY",
    "ABBEY_BOT_LLM_ENDPOINT",
    "ABBEY_BOT_LLM_MODEL",
    "ABBEY_VISION_PROVIDER",
    "ABBEY_VISION_ENDPOINT",
    "ABBEY_VISION_MODEL",
    "ABBEY_FM_MODE",
    "ABBEY_FM_ENDPOINT",
    "ABBEY_FM_FALLBACK",
    "ABBEY_FM_CAPABILITY_MANIFEST",
)
DEACTIVATED_PRIMARY_KEYS = {"ANTHROPIC_API_KEY"}
ASSIGNMENT = re.compile(r"^(?:export[ \t]+)?([A-Za-z_][A-Za-z0-9_]*)=")


def fail(message: str) -> "None":
    raise SystemExit(message)


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
        match = ASSIGNMENT.match(line.strip())
        key = match.group(1) if match else None
        if key in MANAGED_KEYS:
            if key in seen_managed:
                fail(f"the environment contains duplicate managed key {key}")
            seen_managed.add(key)
            if key in DEACTIVATED_PRIMARY_KEYS:
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
        "ABBEY_BOT_LLM_ENDPOINT": "http://127.0.0.1:8282",
        "ABBEY_BOT_LLM_MODEL": str(model_dir),
        "ABBEY_VISION_PROVIDER": "remote",
        "ABBEY_VISION_ENDPOINT": "http://127.0.0.1:8282/v1",
        "ABBEY_VISION_MODEL": str(model_dir),
        "ABBEY_FM_MODE": "system",
        "ABBEY_FM_FALLBACK": "1",
        "ABBEY_FM_CAPABILITY_MANIFEST": str(manifest),
    }
    kept.extend(
        [
            "",
            "# Qualified local providers; managed by deploy/configure-mlx-primary.py.",
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


def publish(env_file: Path, backup_dir: Path, content: str) -> Path:
    backup_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    backup_metadata = backup_dir.lstat()
    if not stat.S_ISDIR(backup_metadata.st_mode) or stat.S_ISLNK(backup_metadata.st_mode):
        fail(f"the environment backup path must be a directory, not a symlink: {backup_dir}")
    if backup_metadata.st_uid != os.getuid() or stat.S_IMODE(backup_metadata.st_mode) & 0o077:
        fail(f"the environment backup directory must be private and owner-only: {backup_dir}")

    serial = f"{int(time.time())}.{os.getpid()}.{secrets.token_hex(4)}"
    backup = backup_dir / f"env.before-mlx.{serial}"
    temporary = env_file.parent / f".{env_file.name}.new.{serial}"
    original = env_file.read_bytes()
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
        fsync_directory(env_file.parent)
        fsync_directory(backup_dir)
    except BaseException:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass
        raise
    return backup


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
    parser.add_argument(
        "--backup-dir",
        type=Path,
        default=Path.home() / ".local" / "share" / "abbey-bot" / "env-backups",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="validate and report managed key names without changing the environment",
    )
    return parser.parse_args()


def main() -> None:
    args = arguments()
    env_file = args.env_file
    if not env_file.is_absolute() or not args.manifest.is_absolute():
        fail("the environment and qualification manifest paths must be absolute")
    require_private_regular_file(env_file, "Abbey environment")
    require_private_regular_file(args.manifest, "FM capability manifest")
    model_dir = validate_model_dir(args.model_dir)
    original = env_file.read_text(encoding="utf-8")
    content = rendered_environment(original, model_dir, args.manifest.resolve(strict=True))
    if args.dry_run:
        print("validated MLX primary cutover")
        print("managed keys: " + ", ".join(MANAGED_KEYS))
        return
    backup = publish(env_file, args.backup_dir, content)
    print(f"updated owner-only Abbey environment: {env_file}")
    print(f"retained owner-only rollback copy: {backup}")


if __name__ == "__main__":
    main()
