#!/bin/sh
# Names-only presence check for the env file launchd actually loads.
# Never prints values. Exit 1 if a required key is missing or blank.
#
#   ./deploy/check-launchd-env.sh ~/.config/abbey-bot/env
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: $0 ENV_FILE" >&2
  exit 2
fi
ENV_FILE=$1

if [ ! -f "$ENV_FILE" ]; then
  echo "missing $ENV_FILE" >&2
  exit 1
fi

key_present() {
  key=$1
  /usr/bin/awk -v key="$key" '
    BEGIN { found = 0 }
    /^[[:space:]]*#/ { next }
    {
      line = $0
      sub(/^[[:space:]]*(export[[:space:]]+)?/, "", line)
      prefix = key "="
      if (index(line, prefix) != 1) next
      val = substr(line, length(prefix) + 1)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", val)
      first = substr(val, 1, 1)
      last = substr(val, length(val), 1)
      if (length(val) >= 2 && ((first == "\"" && last == "\"") || (first == "'"'"'" && last == "'"'"'"))) {
        val = substr(val, 2, length(val) - 2)
      }
      if (val != "") found = 1
    }
    END { exit found ? 0 : 1 }
  ' "$ENV_FILE"
}

report() {
  key=$1
  if key_present "$key"; then
    echo "present: $key"
  else
    echo "missing: $key"
    return 1
  fi
}

status=0
if ! report DISCORD_TOKEN; then
  echo "DISCORD_TOKEN is required in the launchd env file" >&2
  status=1
fi

for key in \
  ABBEY_GUILD_ID \
  ABBEY_BOT_LLM_ENDPOINT \
  ABBEY_BOT_LLM_MODEL \
  ABBEY_VISION_ENDPOINT \
  ABBEY_VISION_MODEL \
  ABBEY_VOICE_GUILD_ID \
  ABBEY_VOICE_CHANNEL_ID \
  ABBEY_VOICE_MODE \
  ABBEY_VOICE_LOCAL_ENDPOINT
do
  report "$key" || true
done

guild=0
channel=0
if key_present ABBEY_VOICE_GUILD_ID; then
  guild=1
fi
if key_present ABBEY_VOICE_CHANNEL_ID; then
  channel=1
fi
if [ "$guild" -ne "$channel" ]; then
  echo "partial voice destination: both ABBEY_VOICE_GUILD_ID and ABBEY_VOICE_CHANNEL_ID are required" >&2
  status=1
fi
if [ "$guild" -eq 1 ] && [ "$channel" -eq 1 ]; then
  if ! key_present ABBEY_BOT_LLM_ENDPOINT; then
    echo "voice destination is set but ABBEY_BOT_LLM_ENDPOINT is missing; local /voice join would fail closed" >&2
    status=1
  fi
  if ! key_present ABBEY_VOICE_MODE; then
    echo "note: ABBEY_VOICE_MODE unset — code defaults to local on macOS" >&2
  fi
fi
if ! key_present ABBEY_BOT_LLM_ENDPOINT; then
  echo "note: no ABBEY_BOT_LLM_ENDPOINT — Abbey will say she has no generation backend" >&2
fi
if ! key_present ABBEY_GUILD_ID; then
  echo "note: no ABBEY_GUILD_ID — slash commands register globally (up to an hour)" >&2
fi
echo "MLX-Audio sidecar is separate: deploy/install-mlx-audio-launchd.sh binds 127.0.0.1:8181 (setuptools 83; webrtcvad patched via importlib.metadata; readiness GET /v1/models)." >&2
exit "$status"
