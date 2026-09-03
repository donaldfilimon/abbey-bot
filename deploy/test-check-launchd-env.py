#!/usr/bin/env python3
"""Offline tests for names-only launchd env presence checks."""

from __future__ import annotations

import os
import pathlib
import stat
import subprocess
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[1]
CHECKER = ROOT / "deploy" / "check-launchd-env.sh"


def invoke(contents: str) -> subprocess.CompletedProcess[str]:
    with tempfile.TemporaryDirectory() as tmp:
        env_path = pathlib.Path(tmp) / "env"
        env_path.write_text(contents, encoding="utf-8")
        os.chmod(env_path, stat.S_IRUSR | stat.S_IWUSR)
        return subprocess.run(
            ["sh", str(CHECKER), str(env_path)],
            text=True,
            capture_output=True,
            check=False,
        )


def main() -> int:
    missing_token = invoke("ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434\n")
    assert missing_token.returncode == 1, missing_token.stderr
    assert "missing: DISCORD_TOKEN" in missing_token.stdout
    assert "http://127.0.0.1:11434" not in missing_token.stdout
    assert "http://127.0.0.1:11434" not in missing_token.stderr

    secret = "super-secret-token-must-not-leak"
    voice_without_llm = invoke(
        f"DISCORD_TOKEN={secret}\nABBEY_VOICE_GUILD_ID=1\nABBEY_VOICE_CHANNEL_ID=2\n"
    )
    assert voice_without_llm.returncode == 1, voice_without_llm.stdout
    assert secret not in voice_without_llm.stdout
    assert secret not in voice_without_llm.stderr
    assert "ABBEY_BOT_LLM_ENDPOINT is missing" in voice_without_llm.stderr

    partial = invoke("DISCORD_TOKEN=x\nABBEY_VOICE_GUILD_ID=1\n")
    assert partial.returncode == 1
    assert "partial voice destination" in partial.stderr

    ok = invoke(
        "DISCORD_TOKEN=x\n"
        "ABBEY_GUILD_ID=1\n"
        "ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434\n"
        "ABBEY_BOT_LLM_MODEL=gemma4:12b\n"
        "ABBEY_VOICE_GUILD_ID=1\n"
        "ABBEY_VOICE_CHANNEL_ID=2\n"
        "ABBEY_VOICE_MODE=local\n"
    )
    assert ok.returncode == 0, ok.stderr
    assert "present: DISCORD_TOKEN" in ok.stdout
    assert "present: ABBEY_BOT_LLM_ENDPOINT" in ok.stdout
    assert "setuptools" in ok.stderr
    assert "importlib.metadata" in ok.stderr
    assert "/v1/models" in ok.stderr
    print("check-launchd-env tests passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
