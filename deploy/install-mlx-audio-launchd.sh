#!/bin/sh
# Install Abbey's pinned, loopback-only MLX-Audio speech sidecar.
#
#   ./deploy/install-mlx-audio-launchd.sh
#   ./deploy/install-mlx-audio-launchd.sh --uninstall
#
# Network access is used only while installing Python packages and exact model
# snapshots. The launchd service itself sets both Hugging Face and Transformers
# to offline mode and binds only 127.0.0.1:8181.
set -eu
umask 077
cd "$(dirname "$0")/.."

LABEL=com.donaldfilimon.abbey-mlx-audio
PLIST_SRC="deploy/$LABEL.plist"
PLIST_DST="$HOME/Library/LaunchAgents/$LABEL.plist"
RUNNER_SRC=deploy/run-mlx-audio.sh
INSTALL_DIR="$HOME/.local/libexec/abbey-bot"
RUNNER_DST="$INSTALL_DIR/run-mlx-audio"
STATE_DIR="$HOME/.local/share/abbey-bot/mlx-audio"
CACHE_DIR="$STATE_DIR/huggingface"
LOG_DIR="$HOME/Library/Logs/abbey-bot"
UID_NUM=$(id -u)
STT_MODEL=mlx-community/whisper-large-v3-turbo-asr-fp16
STT_REVISION=624c19c9af5603fa73b83bce14d4aeea96156d18
TTS_MODEL=mlx-community/Kokoro-82M-bf16
TTS_REVISION=a71e4d38b236d968966a2002c4c895dbd12b1c3c
VOICE_MODEL=prince-canuma/Kokoro-82M
VOICE_REVISION=e02c9eada7ce7416798af36b190a8a2dd2ecd566

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
    if [ "$attempts" -ge 20 ]; then
      echo "MLX-Audio launchd service did not unload within 20 seconds" >&2
      return 1
    fi
    sleep 1
  done
}

wait_for_health() {
  health_port=$1
  health_attempts=$2
  health_name=$3
  attempts=0
  until curl --noproxy '*' --fail --silent --show-error --max-time 2 \
    "http://127.0.0.1:$health_port/v1/models" >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if [ "$attempts" -ge "$health_attempts" ]; then
      echo "$health_name did not become healthy within $health_attempts seconds" >&2
      return 1
    fi
    sleep 1
  done
}

if [ "${1:-}" = "--uninstall" ]; then
  stop_service
  rm -f "$PLIST_DST" "$RUNNER_DST"
  echo "unloaded and removed MLX-Audio launch files; model cache and venv were retained"
  exit 0
fi
if [ "$#" -ne 0 ]; then
  echo "usage: $0 [--uninstall]" >&2
  exit 2
fi

if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
  echo "Abbey's MLX-Audio sidecar requires Apple Silicon macOS" >&2
  exit 1
fi
UV_BIN=$(command -v uv || true)
if [ -z "$UV_BIN" ]; then
  echo "uv is required to install the pinned Python environment" >&2
  exit 1
fi

echo "== create owner-only staging layout =="
mkdir -p "$INSTALL_DIR" "$STATE_DIR" "$LOG_DIR" "$HOME/Library/LaunchAgents"
chmod 700 "$INSTALL_DIR" "$STATE_DIR" "$LOG_DIR"
STAGE_VENV=$(mktemp -d "$INSTALL_DIR/.mlx-audio-venv.release.XXXXXX")
STAGE_CACHE=$(mktemp -d "$STATE_DIR/.huggingface-stage.XXXXXX")
BACKUP_ROOT=$(mktemp -d "$INSTALL_DIR/.mlx-audio-backup.XXXXXX")
RUNNER_STAGE=$(mktemp "$INSTALL_DIR/.run-mlx-audio.new.XXXXXX")
PLIST_STAGE=$(mktemp "$HOME/Library/LaunchAgents/.$LABEL.new.XXXXXX")
SMOKE_DIR=$(mktemp -d "${TMPDIR:-/tmp}/abbey-mlx-smoke.XXXXXX")
STAGED_PID=
SWITCH_STARTED=0
PREVIOUS_LOADED=0
HAD_CACHE=0
HAD_RUNNER=0
HAD_PLIST=0
NEW_CACHE_INSTALLED=0
NEW_RUNNER_INSTALLED=0
NEW_PLIST_INSTALLED=0
KEEP_STAGE_VENV=0

stop_staged_service() {
  if [ -n "$STAGED_PID" ]; then
    kill "$STAGED_PID" 2>/dev/null || true
    wait "$STAGED_PID" 2>/dev/null || true
    STAGED_PID=
  fi
}

restore_previous_install() {
  echo "== candidate failed; restore previous MLX-Audio install ==" >&2
  stop_service || true
  if [ "$NEW_CACHE_INSTALLED" -eq 1 ]; then
    rm -rf "$CACHE_DIR"
  fi
  if [ "$NEW_RUNNER_INSTALLED" -eq 1 ]; then
    rm -f "$RUNNER_DST"
  fi
  if [ "$NEW_PLIST_INSTALLED" -eq 1 ]; then
    rm -f "$PLIST_DST"
  fi
  if [ "$HAD_CACHE" -eq 1 ] && [ -d "$BACKUP_ROOT/cache" ]; then
    mv "$BACKUP_ROOT/cache" "$CACHE_DIR"
  fi
  if [ "$HAD_RUNNER" -eq 1 ] && [ -f "$BACKUP_ROOT/runner" ]; then
    mv "$BACKUP_ROOT/runner" "$RUNNER_DST"
  fi
  if [ "$HAD_PLIST" -eq 1 ] && [ -f "$BACKUP_ROOT/plist" ]; then
    mv "$BACKUP_ROOT/plist" "$PLIST_DST"
  fi
  if [ "$PREVIOUS_LOADED" -eq 1 ] && [ -f "$PLIST_DST" ]; then
    if launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST" && \
      wait_for_health 8181 60 "restored MLX-Audio"; then
      echo "previous MLX-Audio service restored" >&2
    else
      echo "warning: previous files were restored but the service did not recover" >&2
    fi
  fi
}

cleanup() {
  stop_staged_service
  if [ "$KEEP_STAGE_VENV" -eq 0 ]; then
    rm -rf "$STAGE_VENV"
  fi
  rm -rf "$STAGE_CACHE" "$SMOKE_DIR" "$BACKUP_ROOT"
  rm -f "$RUNNER_STAGE" "$PLIST_STAGE"
}

on_exit() {
  status=$1
  trap - EXIT HUP INT TERM
  if [ "$status" -ne 0 ] && [ "$SWITCH_STARTED" -eq 1 ]; then
    restore_previous_install
  fi
  cleanup
  exit "$status"
}
trap 'on_exit $?' EXIT
trap 'exit 1' HUP INT TERM

echo "== install pinned MLX-Audio environment in sibling venv =="
"$UV_BIN" venv --clear --python 3.11 "$STAGE_VENV"
"$UV_BIN" pip install --python "$STAGE_VENV/bin/python" \
  'mlx-audio[stt,tts,server]==0.5.0' \
  'misaki[en]==0.9.4' \
  'https://github.com/explosion/spacy-models/releases/download/en_core_web_sm-3.8.0/en_core_web_sm-3.8.0-py3-none-any.whl'

echo "== cache and verify exact model revisions in sibling cache =="
HF_HOME="$STAGE_CACHE" \
HF_HUB_CACHE="$STAGE_CACHE/hub" \
HF_HUB_DISABLE_TELEMETRY=1 \
STT_MODEL="$STT_MODEL" STT_REVISION="$STT_REVISION" \
TTS_MODEL="$TTS_MODEL" TTS_REVISION="$TTS_REVISION" \
VOICE_MODEL="$VOICE_MODEL" VOICE_REVISION="$VOICE_REVISION" \
"$STAGE_VENV/bin/python" -c '
import os
from pathlib import Path
from huggingface_hub import snapshot_download

for model_key, revision_key in (
    ("STT_MODEL", "STT_REVISION"),
    ("TTS_MODEL", "TTS_REVISION"),
    ("VOICE_MODEL", "VOICE_REVISION"),
):
    model = os.environ[model_key]
    expected = os.environ[revision_key]
    # Pin the network request itself: resolving a moving default and checking
    # it afterwards may already have downloaded an unapproved revision.
    snapshot = Path(snapshot_download(repo_id=model, revision=expected))
    if snapshot.name != expected:
        raise SystemExit(f"{model} resolved to {snapshot.name}, expected pinned {expected}")
    if snapshot.parent.name != "snapshots":
        raise SystemExit(f"{model} returned an unexpected cache path: {snapshot}")

    # MLX-Audio opens models by repository id while offline, so make the
    # staged cache default ref point at the exact verified snapshot.
    main_ref = snapshot.parent.parent / "refs" / "main"
    main_ref.parent.mkdir(parents=True, exist_ok=True)
    main_ref.write_text(expected, encoding="utf-8")
    if main_ref.read_text(encoding="utf-8") != expected:
        raise SystemExit(f"failed to pin the offline default ref for {model}")
    offline = Path(snapshot_download(repo_id=model, revision="main", local_files_only=True))
    if offline.name != expected:
        raise SystemExit(f"offline {model} resolved to {offline.name}, expected {expected}")
    print(f"verified {model}@{expected}")
'

echo "== stage launchd files without changing the live install =="
/bin/cp "$RUNNER_SRC" "$RUNNER_STAGE"
RUNNER_VENV="$STAGE_VENV" "$STAGE_VENV/bin/python" -c '
import os, pathlib, shlex, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
needle = "VENV_DIR=\"$HOME/.local/libexec/abbey-bot/mlx-audio-venv\""
replacement = "VENV_DIR=" + shlex.quote(os.environ["RUNNER_VENV"])
if text.count(needle) != 1:
    raise SystemExit("runner no longer contains the expected venv assignment")
path.write_text(text.replace(needle, replacement), encoding="utf-8")
' "$RUNNER_STAGE"
chmod 700 "$RUNNER_STAGE"
/bin/cp "$PLIST_SRC" "$PLIST_STAGE"
plutil -replace WorkingDirectory -string "$STATE_DIR" "$PLIST_STAGE"
plutil -replace StandardOutPath -string "$LOG_DIR/mlx-audio.log" "$PLIST_STAGE"
plutil -replace StandardErrorPath -string "$LOG_DIR/mlx-audio.log" "$PLIST_STAGE"
plutil -lint "$PLIST_STAGE"

echo "== smoke staged venv and model cache entirely offline =="
SMOKE_PORT=$("$STAGE_VENV/bin/python" -c '
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
')
mkdir -p "$SMOKE_DIR/logs"
HF_HOME="$STAGE_CACHE" \
HF_HUB_CACHE="$STAGE_CACHE/hub" \
HF_HUB_OFFLINE=1 \
TRANSFORMERS_OFFLINE=1 \
TOKENIZERS_PARALLELISM=false \
"$STAGE_VENV/bin/mlx_audio.server" \
  --host 127.0.0.1 \
  --port "$SMOKE_PORT" \
  --log-dir "$SMOKE_DIR/logs" \
  --allowed-origins http://127.0.0.1 \
  >"$SMOKE_DIR/server.log" 2>&1 &
STAGED_PID=$!
if ! wait_for_health "$SMOKE_PORT" 60 "staged MLX-Audio"; then
  tail -n 80 "$SMOKE_DIR/server.log" >&2 || true
  exit 1
fi
if ! kill -0 "$STAGED_PID" 2>/dev/null; then
  echo "staged MLX-Audio exited before its offline smoke" >&2
  tail -n 80 "$SMOKE_DIR/server.log" >&2 || true
  exit 1
fi
curl --noproxy '*' --fail --silent --show-error --max-time 600 --request POST --get \
  --data-urlencode "model_name=$STT_MODEL" \
  "http://127.0.0.1:$SMOKE_PORT/v1/models" >/dev/null
curl --noproxy '*' --fail --silent --show-error --max-time 600 --request POST --get \
  --data-urlencode "model_name=$TTS_MODEL" \
  "http://127.0.0.1:$SMOKE_PORT/v1/models" >/dev/null
curl --noproxy '*' --fail --silent --show-error --max-time 600 \
  -H 'Content-Type: application/json' \
  --data '{"model":"mlx-community/Kokoro-82M-bf16","input":"Abbey local voice is ready.","voice":"af_heart","lang_code":"a","response_format":"wav","stream":false}' \
  -o "$SMOKE_DIR/abbey-ready.wav" \
  "http://127.0.0.1:$SMOKE_PORT/v1/audio/speech"
curl --noproxy '*' --fail --silent --show-error --max-time 600 \
  -F "file=@$SMOKE_DIR/abbey-ready.wav;type=audio/wav" \
  -F "model=$STT_MODEL" \
  -F language=en \
  -F response_format=json \
  -o "$SMOKE_DIR/transcript.json" \
  "http://127.0.0.1:$SMOKE_PORT/v1/audio/transcriptions"
"$STAGE_VENV/bin/python" -c '
import json, pathlib, re, sys
path = pathlib.Path(sys.argv[1])
payload = json.loads(path.read_text())
text = payload.get("text", "").strip()
if not text:
    raise SystemExit("offline TTS-to-STT smoke returned an empty transcript")
words = set(re.findall(r"[a-z]+", text.lower()))
if not {"local", "voice", "ready"}.issubset(words) or not ({"abbey", "abby"} & words):
    raise SystemExit(f"offline TTS-to-STT smoke did not recover the test phrase: {text!r}")
print(f"offline speech smoke transcript: {text}")
' "$SMOKE_DIR/transcript.json"
stop_staged_service

echo "== atomically switch the validated candidate into place =="
if launchctl print "gui/$UID_NUM/$LABEL" >/dev/null 2>&1; then
  PREVIOUS_LOADED=1
fi
SWITCH_STARTED=1
stop_service
if [ -e "$CACHE_DIR" ]; then
  mv "$CACHE_DIR" "$BACKUP_ROOT/cache"
  HAD_CACHE=1
fi
if [ -e "$RUNNER_DST" ]; then
  mv "$RUNNER_DST" "$BACKUP_ROOT/runner"
  HAD_RUNNER=1
fi
if [ -e "$PLIST_DST" ]; then
  mv "$PLIST_DST" "$BACKUP_ROOT/plist"
  HAD_PLIST=1
fi
mv "$STAGE_CACHE" "$CACHE_DIR"
NEW_CACHE_INSTALLED=1
mv "$RUNNER_STAGE" "$RUNNER_DST"
NEW_RUNNER_INSTALLED=1
mv "$PLIST_STAGE" "$PLIST_DST"
NEW_PLIST_INSTALLED=1

echo "== start and verify the switched loopback-only offline service =="
launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"
if ! wait_for_health 8181 60 "MLX-Audio"; then
  tail -n 80 "$LOG_DIR/mlx-audio.log" >&2 || true
  exit 1
fi
curl --noproxy '*' --fail --silent --show-error --max-time 600 --request POST --get \
  --data-urlencode "model_name=$STT_MODEL" \
  http://127.0.0.1:8181/v1/models >/dev/null
curl --noproxy '*' --fail --silent --show-error --max-time 600 --request POST --get \
  --data-urlencode "model_name=$TTS_MODEL" \
  http://127.0.0.1:8181/v1/models >/dev/null

PID=$(service_pid)
KEEP_STAGE_VENV=1
SWITCH_STARTED=0
echo "MLX-Audio ready: pid $PID, http://127.0.0.1:8181"
echo "log: $LOG_DIR/mlx-audio.log"
