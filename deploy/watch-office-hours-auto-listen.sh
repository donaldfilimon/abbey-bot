#!/bin/sh
# Interim Mac watcher: when Donald is in Office Hours and Abbey is muted/deafened
# in that channel, kickstart abbey-bot once (debounced) so startup auto-listen can run.
# Does not echo token values. Prefer PATH restore after sourcing env.
set -eu

ENV_FILE="${ABBEY_ENV_FILE:-$HOME/.config/abbey-bot/env}"
LOG_DIR="${ABBEY_OH_AUTOLISTEN_LOG_DIR:-$HOME/Library/Logs/abbey-bot}"
LOG_FILE="$LOG_DIR/oh-autolisten.log"
STATE_DIR="${ABBEY_OH_AUTOLISTEN_STATE_DIR:-$HOME/.local/share/abbey-bot/oh-autolisten}"
LAST_KICK="$STATE_DIR/last-kickstart"
DEBOUNCE_SECS="${ABBEY_OH_AUTOLISTEN_DEBOUNCE_SECS:-60}"
POLL_SECS="${ABBEY_OH_AUTOLISTEN_POLL_SECS:-20}"
GUILD_ID="${ABBEY_OH_GUILD_ID:-1275617641620443146}"
CHANNEL_ID="${ABBEY_OH_CHANNEL_ID:-1495755277859815595}"
DONALD_ID="${ABBEY_OH_DONALD_ID:-1122140354737623110}"
LABEL="${ABBEY_OH_BOT_LABEL:-com.donaldfilimon.abbey-bot}"

mkdir -p "$LOG_DIR" "$STATE_DIR"
SAVED_PATH="${PATH:-/usr/bin:/bin:/usr/sbin:/sbin}"

log() {
  # Prefer local wall clock; LaunchAgent logs land in ~/Library/Logs.
  printf '%s %s\n' "$(/bin/date '+%Y-%m-%d %H:%M:%S %Z')" "$*" >>"$LOG_FILE"
}

load_env() {
  if [ ! -f "$ENV_FILE" ]; then
    log "missing env file (names only check); path configured but absent"
    return 1
  fi
  # shellcheck disable=SC1090
  set -a
  # Source without printing. Restore PATH afterward so curl stays /usr/bin/curl.
  . "$ENV_FILE"
  set +a
  PATH="$SAVED_PATH"
  export PATH
  if [ -z "${DISCORD_TOKEN:-${DISCORD_BOT_TOKEN:-}}" ]; then
    log "DISCORD_TOKEN/DISCORD_BOT_TOKEN missing or empty in env file"
    return 1
  fi
  TOKEN="${DISCORD_TOKEN:-$DISCORD_BOT_TOKEN}"
  return 0
}

json_get() {
  # Minimal JSON field pull without python dependency on LaunchAgent PATH.
  # $1=json $2=key → prints value or empty
  /usr/bin/awk -v key="$2" '
    BEGIN { s = ARGV[1]; delete ARGV[1] }
    {
      # fall through if awk sees stdin; we pass json as ARGV
    }
  ' "$1" 2>/dev/null || true
  printf '%s' "$1" | /usr/bin/sed -n "s/.*\"$2\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" | /usr/bin/head -n 1
}

json_bool() {
  printf '%s' "$1" | /usr/bin/sed -n "s/.*\"$2\"[[:space:]]*:[[:space:]]*\\(true\\|false\\).*/\\1/p" | /usr/bin/head -n 1
}

json_null_or_id() {
  # channel_id may be null or a snowflake string/number
  if printf '%s' "$1" | /usr/bin/grep -q "\"$2\"[[:space:]]*:[[:space:]]*null"; then
    printf ''
    return 0
  fi
  val=$(printf '%s' "$1" | /usr/bin/sed -n "s/.*\"$2\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" | /usr/bin/head -n 1)
  if [ -n "$val" ]; then
    printf '%s' "$val"
    return 0
  fi
  printf '%s' "$1" | /usr/bin/sed -n "s/.*\"$2\"[[:space:]]*:[[:space:]]*\\([0-9][0-9]*\\).*/\\1/p" | /usr/bin/head -n 1
}

discord_get() {
  path=$1
  /usr/bin/curl -sS -m 8 \
    -H "Authorization: Bot ${TOKEN}" \
    -H "User-Agent: AbbeyOhAutolisten (local; +https://github.com/donaldfilimon/abbey-bot)" \
    "https://discord.com/api/v10${path}"
}

maybe_kickstart() {
  now=$(/bin/date +%s)
  if [ -f "$LAST_KICK" ]; then
    last=$(/bin/cat "$LAST_KICK" 2>/dev/null || printf '0')
    case "$last" in
      ''|*[!0-9]*) last=0 ;;
    esac
    delta=$((now - last))
    if [ "$delta" -lt "$DEBOUNCE_SECS" ]; then
      log "debounce: skip kickstart (${delta}s < ${DEBOUNCE_SECS}s)"
      return 0
    fi
  fi
  uid=$(/usr/bin/id -u)
  if /bin/launchctl kickstart -k "gui/${uid}/${LABEL}"; then
    printf '%s\n' "$now" >"$LAST_KICK"
    log "kickstart ok: gui/${uid}/${LABEL}"
  else
    log "kickstart failed: gui/${uid}/${LABEL}"
  fi
}

once() {
  if ! load_env; then
    return 0
  fi

  donald_vs=$(discord_get "/guilds/${GUILD_ID}/voice-states/${DONALD_ID}" || true)
  if printf '%s' "$donald_vs" | /usr/bin/grep -q '"code"[[:space:]]*:'; then
    # 10065 = Unknown Voice State (not in any VC). Idle — do not spam the log.
    code=$(printf '%s' "$donald_vs" | /usr/bin/sed -n 's/.*"code"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | /usr/bin/head -n 1)
    case "${code:-}" in
      10065|10004|10013|10057|"")
        return 0
        ;;
      *)
        log "donald voice-state API error code=${code}"
        return 0
        ;;
    esac
  fi
  donald_ch=$(json_null_or_id "$donald_vs" channel_id)
  if [ "$donald_ch" != "$CHANNEL_ID" ]; then
    log "donald not in Office Hours (channel=${donald_ch:-none})"
    return 0
  fi

  me=$(discord_get "/users/@me" || true)
  bot_id=$(json_get "$me" id)
  if [ -z "$bot_id" ]; then
    log "could not resolve bot user id from /users/@me"
    return 0
  fi

  abbey_vs=$(discord_get "/guilds/${GUILD_ID}/voice-states/${bot_id}" || true)
  if printf '%s' "$abbey_vs" | /usr/bin/grep -q '"code"[[:space:]]*:'; then
    code=$(printf '%s' "$abbey_vs" | /usr/bin/sed -n 's/.*"code"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | /usr/bin/head -n 1)
    log "abbey not in guild voice (code=${code:-unknown})"
    return 0
  fi
  abbey_ch=$(json_null_or_id "$abbey_vs" channel_id)
  if [ "$abbey_ch" != "$CHANNEL_ID" ]; then
    log "abbey not in Office Hours (channel=${abbey_ch:-none})"
    return 0
  fi

  self_mute=$(json_bool "$abbey_vs" self_mute)
  self_deaf=$(json_bool "$abbey_vs" self_deaf)
  if [ "$self_mute" != "true" ] && [ "$self_deaf" != "true" ]; then
    log "abbey already unmuted/undeafened in Office Hours; no kickstart"
    return 0
  fi

  log "donald in OH + abbey muted/deafened (self_mute=${self_mute:-?} self_deaf=${self_deaf:-?}); kickstarting"
  maybe_kickstart
}

# Loop when launched as a long-running agent; also supports one-shot via ABBEY_OH_ONCE=1
if [ "${ABBEY_OH_ONCE:-0}" = "1" ]; then
  once
  exit 0
fi

log "watcher start poll=${POLL_SECS}s debounce=${DEBOUNCE_SECS}s guild=${GUILD_ID} channel=${CHANNEL_ID}"
while true; do
  once || log "once() returned non-zero (continuing)"
  /bin/sleep "$POLL_SECS"
done
