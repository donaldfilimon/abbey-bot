#!/bin/sh
# Owner-only launchd wrapper for Abbey's unified local Gemma 4 backend.
set -eu
umask 077

VENV_DIR="$HOME/.local/libexec/abbey-bot/mlx-vlm-venv"
CACHE_DIR="$HOME/.local/share/abbey-bot/mlx-vlm/huggingface"
MODEL_REVISION=73bcf09092aa277861d5a191b989b666f7f32e8f
MODEL_DIR="$CACHE_DIR/hub/models--mlx-community--gemma-4-12B-it-4bit/snapshots/$MODEL_REVISION"

if [ ! -d "$MODEL_DIR" ]; then
  echo "pinned MLX-VLM model snapshot is missing: $MODEL_DIR" >&2
  exit 1
fi

# Runtime inference is deliberately network-independent. The installer fetches
# and verifies the exact model revision, then proves an offline restart before
# this launchd service is published.
export HF_HOME="$CACHE_DIR"
export HF_HUB_CACHE="$CACHE_DIR/hub"
export HF_HUB_OFFLINE=1
export TRANSFORMERS_OFFLINE=1
export HF_HUB_DISABLE_TELEMETRY=1
export TOKENIZERS_PARALLELISM=false
export NO_PROXY=127.0.0.1,localhost
export no_proxy=127.0.0.1,localhost
unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy

exec "$VENV_DIR/bin/python" -m mlx_vlm.server \
  --host 127.0.0.1 \
  --port 8282 \
  --model "$MODEL_DIR" \
  --max-tokens 4096 \
  --max-num-seqs 1 \
  --vision-cache-size 4 \
  --log-level INFO
