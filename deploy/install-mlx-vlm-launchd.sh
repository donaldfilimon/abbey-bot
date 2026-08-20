#!/bin/sh
# Install Abbey's pinned, loopback-only MLX-VLM Gemma 4 sidecar.
#
#   ./deploy/install-mlx-vlm-launchd.sh
#   ./deploy/install-mlx-vlm-launchd.sh --uninstall
#
# A new Python environment and exact model revision are installed and smoke-
# tested on a temporary loopback port before any healthy live service is
# stopped. The launchd replacement is rolled back if its offline restart or
# text/tool/vision acceptance fails.
set -eu
umask 077
cd "$(dirname "$0")/.."

LABEL=com.donaldfilimon.abbey-mlx-vlm
PLIST_SRC="deploy/$LABEL.plist"
PLIST_DST="$HOME/Library/LaunchAgents/$LABEL.plist"
RUNNER_SRC=deploy/run-mlx-vlm.sh
SMOKE_SRC=deploy/smoke-mlx-vlm.py
REQUIREMENTS=deploy/mlx-vlm-requirements.txt
INSTALL_DIR="$HOME/.local/libexec/abbey-bot"
RUNNER_DST="$INSTALL_DIR/run-mlx-vlm"
VENV_DIR="$INSTALL_DIR/mlx-vlm-venv"
STATE_DIR="$HOME/.local/share/abbey-bot/mlx-vlm"
CACHE_DIR="$STATE_DIR/huggingface"
LOG_DIR="$HOME/Library/Logs/abbey-bot"
INSTALL_LOCK="$STATE_DIR/install.lock"
UID_NUM=$(id -u)
MODEL=mlx-community/gemma-4-12B-it-4bit
MODEL_REVISION=73bcf09092aa277861d5a191b989b666f7f32e8f
MODEL_DIR="$CACHE_DIR/hub/models--mlx-community--gemma-4-12B-it-4bit/snapshots/$MODEL_REVISION"
LIVE_PORT=8282

VENV_STAGE=
RUNNER_STAGE=
PLIST_STAGE=
SMOKE_PID=
LOCK_HELD=0
TRANSITION_ARMED=0
HAD_VENV=0
HAD_RUNNER=0
HAD_PLIST=0

cleanup() {
  if [ "$TRANSITION_ARMED" -eq 1 ]; then
    rollback
  fi
  if [ -n "$SMOKE_PID" ]; then
    kill "$SMOKE_PID" 2>/dev/null || true
    wait "$SMOKE_PID" 2>/dev/null || true
  fi
  if [ -n "$RUNNER_STAGE" ]; then
    rm -f "$RUNNER_STAGE"
  fi
  if [ -n "$PLIST_STAGE" ]; then
    rm -f "$PLIST_STAGE"
  fi
  if [ -n "$VENV_STAGE" ] && [ -d "$VENV_STAGE" ]; then
    echo "staged environment retained for inspection: $VENV_STAGE" >&2
  fi
  if [ "$LOCK_HELD" -eq 1 ]; then
    rmdir "$INSTALL_LOCK" 2>/dev/null || true
  fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

service_pid() {
  SERVICE_OUTPUT=$(launchctl print "gui/$UID_NUM/$LABEL" 2>/dev/null) || return 1
  SERVICE_PID=$(printf '%s\n' "$SERVICE_OUTPUT" | /usr/bin/sed -n \
    's/^[[:space:]]*pid = \([0-9][0-9]*\)$/\1/p')
  case "$SERVICE_PID" in
    ''|*[!0-9]*) return 1 ;;
    *) printf '%s\n' "$SERVICE_PID" ;;
  esac
}

stop_service() {
  launchctl bootout "gui/$UID_NUM/$LABEL" 2>/dev/null || true
  attempts=0
  while launchctl print "gui/$UID_NUM/$LABEL" >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if [ "$attempts" -ge 30 ]; then
      echo "MLX-VLM launchd service did not unload within 30 seconds" >&2
      return 1
    fi
    sleep 1
  done
}

wait_for_health() {
  base_url=$1
  log_file=$2
  deadline=$(($(date +%s) + 900))
  until curl --noproxy '*' --fail --silent --show-error --max-time 2 \
    "$base_url/health" >/dev/null 2>&1; do
    if [ "$(date +%s)" -ge "$deadline" ]; then
      echo "MLX-VLM did not become healthy within 900 seconds" >&2
      tail -n 120 "$log_file" >&2 || true
      return 1
    fi
    sleep 1
  done
}

offline_server() {
  python_bin=$1
  port=$2
  log_file=$3
  HF_HOME="$CACHE_DIR" \
  HF_HUB_CACHE="$CACHE_DIR/hub" \
  HF_HUB_OFFLINE=1 \
  TRANSFORMERS_OFFLINE=1 \
  HF_HUB_DISABLE_TELEMETRY=1 \
  TOKENIZERS_PARALLELISM=false \
  NO_PROXY=127.0.0.1,localhost \
  no_proxy=127.0.0.1,localhost \
  HTTP_PROXY= HTTPS_PROXY= ALL_PROXY= \
  http_proxy= https_proxy= all_proxy= \
  "$python_bin" -m mlx_vlm.server \
    --host 127.0.0.1 \
    --port "$port" \
    --model "$MODEL_DIR" \
    --max-tokens 4096 \
    --max-num-seqs 1 \
    --vision-cache-size 4 \
    --log-level INFO >"$log_file" 2>&1 &
  SMOKE_PID=$!
}

if [ "${1:-}" = "--uninstall" ]; then
  stop_service
  rm -f "$PLIST_DST" "$RUNNER_DST"
  echo "unloaded and removed MLX-VLM launch files; model cache and venv were retained"
  exit 0
fi
if [ "$#" -ne 0 ]; then
  echo "usage: $0 [--uninstall]" >&2
  exit 2
fi

if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
  echo "Abbey's MLX-VLM sidecar requires Apple Silicon macOS" >&2
  exit 1
fi
UV_BIN=$(command -v uv || true)
if [ -z "$UV_BIN" ]; then
  echo "uv is required to install the pinned Python environment" >&2
  exit 1
fi
for required in "$PLIST_SRC" "$RUNNER_SRC" "$SMOKE_SRC" "$REQUIREMENTS"; do
  if [ ! -f "$required" ]; then
    echo "missing required deployment file: $required" >&2
    exit 1
  fi
done

echo "== create owner-only layout and take install lock =="
mkdir -p "$INSTALL_DIR" "$STATE_DIR" "$CACHE_DIR" "$LOG_DIR" "$HOME/Library/LaunchAgents"
chmod 700 "$INSTALL_DIR" "$STATE_DIR" "$CACHE_DIR" "$LOG_DIR"
if ! mkdir "$INSTALL_LOCK" 2>/dev/null; then
  echo "another MLX-VLM installation is already running: $INSTALL_LOCK" >&2
  exit 1
fi
LOCK_HELD=1

echo "== stage hash-locked MLX-VLM environment =="
VENV_STAGE=$(mktemp -d "$INSTALL_DIR/.mlx-vlm-venv.new.XXXXXX")
"$UV_BIN" venv --python 3.11 "$VENV_STAGE"
"$UV_BIN" pip install --python "$VENV_STAGE/bin/python" \
  --require-hashes --only-binary=:all: --requirement "$REQUIREMENTS"

echo "== cache and verify exact Gemma 4 revision =="
HF_HOME="$CACHE_DIR" \
HF_HUB_CACHE="$CACHE_DIR/hub" \
HF_HUB_DISABLE_TELEMETRY=1 \
MODEL="$MODEL" MODEL_REVISION="$MODEL_REVISION" MODEL_DIR="$MODEL_DIR" \
"$VENV_STAGE/bin/python" -c '
import os
from pathlib import Path
from huggingface_hub import snapshot_download

model = os.environ["MODEL"]
expected = os.environ["MODEL_REVISION"]
expected_path = Path(os.environ["MODEL_DIR"])
snapshot = Path(snapshot_download(repo_id=model, revision=expected))
if snapshot.name != expected or snapshot != expected_path:
    raise SystemExit(f"{model} resolved to {snapshot}, expected {expected_path}")
if not (snapshot / "model.safetensors.index.json").is_file():
    raise SystemExit(f"{model} snapshot is incomplete: {snapshot}")
main_ref = snapshot.parent.parent / "refs" / "main"
main_ref.parent.mkdir(parents=True, exist_ok=True)
main_ref.write_text(expected, encoding="utf-8")
if main_ref.read_text(encoding="utf-8") != expected:
    raise SystemExit(f"failed to pin the offline default ref for {model}")
print(f"verified {model}@{expected}")
'

echo "== offline staged text, tool-loop, and vision smoke =="
SMOKE_PORT=$("$VENV_STAGE/bin/python" -c '
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
')
SMOKE_LOG="$LOG_DIR/mlx-vlm-preflight.log"
offline_server "$VENV_STAGE/bin/python" "$SMOKE_PORT" "$SMOKE_LOG"
wait_for_health "http://127.0.0.1:$SMOKE_PORT" "$SMOKE_LOG"
"$VENV_STAGE/bin/python" "$SMOKE_SRC" \
  --base-url "http://127.0.0.1:$SMOKE_PORT" \
  --model "$MODEL_DIR" \
  --timeout 600
kill "$SMOKE_PID" 2>/dev/null || true
wait "$SMOKE_PID" 2>/dev/null || true
SMOKE_PID=

echo "== stage launchd files =="
RUNNER_STAGE=$(mktemp "$INSTALL_DIR/.run-mlx-vlm.new.XXXXXX")
PLIST_STAGE=$(mktemp "$HOME/Library/LaunchAgents/.$LABEL.new.XXXXXX")
/bin/cp "$RUNNER_SRC" "$RUNNER_STAGE"
chmod 700 "$RUNNER_STAGE"
/bin/cp "$PLIST_SRC" "$PLIST_STAGE"
plutil -replace WorkingDirectory -string "$STATE_DIR" "$PLIST_STAGE"
plutil -replace StandardOutPath -string "$LOG_DIR/mlx-vlm.log" "$PLIST_STAGE"
plutil -replace StandardErrorPath -string "$LOG_DIR/mlx-vlm.log" "$PLIST_STAGE"
plutil -lint "$PLIST_STAGE"

STAMP=$(date -u +%Y%m%dT%H%M%SZ)
VENV_BACKUP=
RUNNER_BACKUP=
PLIST_BACKUP=
OLD_SERVICE=0
if launchctl print "gui/$UID_NUM/$LABEL" >/dev/null 2>&1; then
  OLD_SERVICE=1
fi

rollback() {
  TRANSITION_ARMED=0
  echo "MLX-VLM replacement failed; restoring the previous installation" >&2
  if ! stop_service; then
    echo "replacement could not be unloaded; installed files were left untouched" >&2
    echo "manual recovery is required; retained backups end in .backup.$STAMP" >&2
    return 1
  fi
  if [ -n "$VENV_BACKUP" ] && [ -d "$VENV_BACKUP" ]; then
    if [ -d "$VENV_DIR" ]; then
      mv "$VENV_DIR" "$VENV_DIR.failed.$STAMP"
    fi
    mv "$VENV_BACKUP" "$VENV_DIR"
  elif [ "$HAD_VENV" -eq 0 ] && [ -d "$VENV_DIR" ]; then
    mv "$VENV_DIR" "$VENV_DIR.failed.$STAMP"
  fi
  if [ -n "$RUNNER_BACKUP" ] && [ -f "$RUNNER_BACKUP" ]; then
    if [ -f "$RUNNER_DST" ]; then
      mv "$RUNNER_DST" "$RUNNER_DST.failed.$STAMP"
    fi
    mv "$RUNNER_BACKUP" "$RUNNER_DST"
  elif [ "$HAD_RUNNER" -eq 0 ] && [ -f "$RUNNER_DST" ]; then
    mv "$RUNNER_DST" "$RUNNER_DST.failed.$STAMP"
  fi
  if [ -n "$PLIST_BACKUP" ] && [ -f "$PLIST_BACKUP" ]; then
    if [ -f "$PLIST_DST" ]; then
      mv "$PLIST_DST" "$PLIST_DST.failed.$STAMP"
    fi
    mv "$PLIST_BACKUP" "$PLIST_DST"
  elif [ "$HAD_PLIST" -eq 0 ] && [ -f "$PLIST_DST" ]; then
    mv "$PLIST_DST" "$PLIST_DST.failed.$STAMP"
  fi
  if [ "$OLD_SERVICE" -eq 1 ] && [ -f "$PLIST_DST" ]; then
    if ! launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"; then
      echo "previous files were restored, but launchd bootstrap failed" >&2
      return 1
    fi
    if ! wait_for_health "http://127.0.0.1:$LIVE_PORT" "$LOG_DIR/mlx-vlm.log"; then
      echo "previous files were restored, but the restored service is unhealthy" >&2
      return 1
    fi
  fi
}

echo "== publish staged environment with rollback =="
if [ -d "$VENV_DIR" ]; then
  HAD_VENV=1
fi
if [ -f "$RUNNER_DST" ]; then
  HAD_RUNNER=1
fi
if [ -f "$PLIST_DST" ]; then
  HAD_PLIST=1
fi
TRANSITION_ARMED=1
stop_service
if [ -d "$VENV_DIR" ]; then
  VENV_BACKUP="$VENV_DIR.backup.$STAMP"
  mv "$VENV_DIR" "$VENV_BACKUP"
fi
if [ -f "$RUNNER_DST" ]; then
  RUNNER_BACKUP="$RUNNER_DST.backup.$STAMP"
  mv "$RUNNER_DST" "$RUNNER_BACKUP"
fi
if [ -f "$PLIST_DST" ]; then
  PLIST_BACKUP="$PLIST_DST.backup.$STAMP"
  mv "$PLIST_DST" "$PLIST_BACKUP"
fi
mv "$VENV_STAGE" "$VENV_DIR"
VENV_STAGE=
mv "$RUNNER_STAGE" "$RUNNER_DST"
RUNNER_STAGE=
mv "$PLIST_STAGE" "$PLIST_DST"
PLIST_STAGE=

if ! launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"; then
  rollback
  exit 1
fi
if ! wait_for_health "http://127.0.0.1:$LIVE_PORT" "$LOG_DIR/mlx-vlm.log"; then
  rollback
  exit 1
fi
if ! "$VENV_DIR/bin/python" "$SMOKE_SRC" \
  --base-url "http://127.0.0.1:$LIVE_PORT" \
  --model "$MODEL_DIR" \
  --timeout 600; then
  rollback
  exit 1
fi

PID=$(service_pid)
TRANSITION_ARMED=0
echo "MLX-VLM ready: pid $PID, http://127.0.0.1:$LIVE_PORT"
echo "model: $MODEL_DIR"
echo "Abbey profile:"
echo "  ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:$LIVE_PORT"
echo "  ABBEY_BOT_LLM_MODEL=$MODEL_DIR"
echo "  ABBEY_BOT_LLM_CONCURRENCY=1"
echo "  ABBEY_VISION_ENDPOINT=http://127.0.0.1:$LIVE_PORT/v1"
echo "  ABBEY_VISION_MODEL=$MODEL_DIR"
echo "log: $LOG_DIR/mlx-vlm.log"
if [ -n "$VENV_BACKUP" ]; then
  echo "previous environment retained for rollback: $VENV_BACKUP"
fi
