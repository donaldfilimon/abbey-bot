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
cd "$(dirname "$0")/.."

LABEL=com.donaldfilimon.abbey-mlx-audio
PLIST_SRC="deploy/$LABEL.plist"
PLIST_DST="$HOME/Library/LaunchAgents/$LABEL.plist"
RUNNER_SRC=deploy/run-mlx-audio.sh
INSTALL_DIR="$HOME/.local/libexec/abbey-bot"
RUNNER_DST="$INSTALL_DIR/run-mlx-audio"
VENV_DIR="$INSTALL_DIR/mlx-audio-venv"
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

echo "== stop previous sidecar =="
stop_service

echo "== create owner-only layout =="
mkdir -p "$INSTALL_DIR" "$STATE_DIR" "$CACHE_DIR" "$LOG_DIR" "$HOME/Library/LaunchAgents"
chmod 700 "$INSTALL_DIR" "$STATE_DIR" "$CACHE_DIR" "$LOG_DIR"

echo "== install pinned MLX-Audio environment =="
"$UV_BIN" venv --clear --python 3.11 "$VENV_DIR"
"$UV_BIN" pip install --python "$VENV_DIR/bin/python" \
  'mlx-audio[stt,tts,server]==0.5.0' \
  'misaki[en]==0.9.4' \
  'https://github.com/explosion/spacy-models/releases/download/en_core_web_sm-3.8.0/en_core_web_sm-3.8.0-py3-none-any.whl'

echo "== cache and verify exact model revisions =="
HF_HOME="$CACHE_DIR" \
HF_HUB_DISABLE_TELEMETRY=1 \
STT_MODEL="$STT_MODEL" STT_REVISION="$STT_REVISION" \
TTS_MODEL="$TTS_MODEL" TTS_REVISION="$TTS_REVISION" \
VOICE_MODEL="$VOICE_MODEL" VOICE_REVISION="$VOICE_REVISION" \
"$VENV_DIR/bin/python" -c '
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
    # Resolve the repository current default revision while installation has
    # network access, then pin it by exact snapshot id. This also writes the
    # refs/main entry MLX-Audio consults later in HF_HUB_OFFLINE mode.
    snapshot = Path(snapshot_download(repo_id=model))
    if snapshot.name != expected:
        raise SystemExit(f"{model} resolved to {snapshot.name}, expected pinned {expected}")
    print(f"verified {model}@{expected}")
'

echo "== install launchd files =="
RUNNER_STAGE=$(mktemp "$INSTALL_DIR/.run-mlx-audio.new.XXXXXX")
PLIST_STAGE=$(mktemp "$HOME/Library/LaunchAgents/.$LABEL.new.XXXXXX")
cleanup_stages() {
  rm -f "$RUNNER_STAGE" "$PLIST_STAGE"
}
trap cleanup_stages EXIT
trap 'exit 1' HUP INT TERM
/bin/cp "$RUNNER_SRC" "$RUNNER_STAGE"
chmod 700 "$RUNNER_STAGE"
/bin/cp "$PLIST_SRC" "$PLIST_STAGE"
plutil -replace WorkingDirectory -string "$STATE_DIR" "$PLIST_STAGE"
plutil -replace StandardOutPath -string "$LOG_DIR/mlx-audio.log" "$PLIST_STAGE"
plutil -replace StandardErrorPath -string "$LOG_DIR/mlx-audio.log" "$PLIST_STAGE"
plutil -lint "$PLIST_STAGE"
mv -f "$RUNNER_STAGE" "$RUNNER_DST"
mv -f "$PLIST_STAGE" "$PLIST_DST"

echo "== start loopback-only offline service =="
launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"
attempts=0
until curl --fail --silent --show-error --max-time 2 \
  http://127.0.0.1:8181/v1/models >/dev/null 2>&1; do
  attempts=$((attempts + 1))
  if [ "$attempts" -ge 60 ]; then
    echo "MLX-Audio did not become healthy within 60 seconds" >&2
    tail -n 80 "$LOG_DIR/mlx-audio.log" >&2 || true
    exit 1
  fi
  sleep 1
done

echo "== load models and smoke TTS -> STT entirely offline =="
curl --fail --silent --show-error --max-time 600 --request POST --get \
  --data-urlencode "model_name=$STT_MODEL" \
  http://127.0.0.1:8181/v1/models >/dev/null
curl --fail --silent --show-error --max-time 600 --request POST --get \
  --data-urlencode "model_name=$TTS_MODEL" \
  http://127.0.0.1:8181/v1/models >/dev/null

SMOKE_DIR=$(mktemp -d "${TMPDIR:-/tmp}/abbey-mlx-smoke.XXXXXX")
cleanup_smoke() {
  rm -rf "$SMOKE_DIR"
  cleanup_stages
}
trap cleanup_smoke EXIT
curl --fail --silent --show-error --max-time 600 \
  -H 'Content-Type: application/json' \
  --data '{"model":"mlx-community/Kokoro-82M-bf16","input":"Abbey local voice is ready.","voice":"af_heart","lang_code":"a","response_format":"wav","stream":false}' \
  -o "$SMOKE_DIR/abbey-ready.wav" \
  http://127.0.0.1:8181/v1/audio/speech
curl --fail --silent --show-error --max-time 600 \
  -F "file=@$SMOKE_DIR/abbey-ready.wav;type=audio/wav" \
  -F "model=$STT_MODEL" \
  -F language=en \
  -F response_format=json \
  -o "$SMOKE_DIR/transcript.json" \
  http://127.0.0.1:8181/v1/audio/transcriptions
"$VENV_DIR/bin/python" -c '
import json, pathlib, sys
path = pathlib.Path(sys.argv[1])
payload = json.loads(path.read_text())
text = payload.get("text", "").strip()
if not text:
    raise SystemExit("offline TTS-to-STT smoke returned an empty transcript")
print(f"offline speech smoke transcript: {text}")
' "$SMOKE_DIR/transcript.json"

PID=$(service_pid)
echo "MLX-Audio ready: pid $PID, http://127.0.0.1:8181"
echo "log: $LOG_DIR/mlx-audio.log"
