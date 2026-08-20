#!/bin/sh
# Owner-only launchd wrapper for Abbey's local speech service.
set -eu
umask 077

VENV_DIR="$HOME/.local/libexec/abbey-bot/mlx-audio-venv"
CACHE_DIR="$HOME/.local/share/abbey-bot/mlx-audio/huggingface"
LOG_DIR="$HOME/Library/Logs/abbey-bot"
# Runtime is deliberately network-independent. The installer downloads and
# verifies the exact model revisions before launchd ever starts this process.
export HF_HOME="$CACHE_DIR"
export HF_HUB_OFFLINE=1
export TRANSFORMERS_OFFLINE=1
export TOKENIZERS_PARALLELISM=false

exec "$VENV_DIR/bin/mlx_audio.server" \
  --host 127.0.0.1 \
  --port 8181 \
  --log-dir "$LOG_DIR" \
  --allowed-origins http://127.0.0.1
