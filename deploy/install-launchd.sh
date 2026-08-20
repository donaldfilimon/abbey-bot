#!/bin/sh
# Install or update Abbey Bot as a launchd user agent on this Mac.
#
#   ./deploy/install-launchd.sh            # build --release --locked, install, (re)load
#   ./deploy/install-launchd.sh --uninstall
#
# Layout (all under $HOME, nothing needs sudo):
#   ~/.config/abbey-bot/env                the secrets (you write this; chmod 600)
#   ~/.local/libexec/abbey-bot/abbey-bot   the binary
#   ~/.local/share/abbey-bot/data          ABBEY_DATA_DIR (learning, memory, WDBX)
#   ~/Library/Logs/abbey-bot/abbey-bot.log stdout+stderr
#   ~/Library/LaunchAgents/com.donaldfilimon.abbey-bot.plist
#
# No pipe decides an exit status below (`cmd | tail` would report tail's).
set -eu
cd "$(dirname "$0")/.."

LABEL=com.donaldfilimon.abbey-bot
PLIST_SRC=deploy/$LABEL.plist
PLIST_DST="$HOME/Library/LaunchAgents/$LABEL.plist"
BIN_DIR="$HOME/.local/libexec/abbey-bot"
BIN_DST="$BIN_DIR/abbey-bot"
DATA_DIR="$HOME/.local/share/abbey-bot/data"
LOG_DIR="$HOME/Library/Logs/abbey-bot"
ENV_FILE="$HOME/.config/abbey-bot/env"
UID_NUM=$(id -u)

service_pid() {
  SERVICE_OUTPUT=$(launchctl print "gui/$UID_NUM/$LABEL" 2>/dev/null) || return 1
  SERVICE_PID=$(printf '%s\n' "$SERVICE_OUTPUT" | /usr/bin/sed -n \
    's/^[[:space:]]*pid = \([0-9][0-9]*\)$/\1/p')
  case "$SERVICE_PID" in
    ''|*[!0-9]*) return 1 ;;
    *) printf '%s\n' "$SERVICE_PID" ;;
  esac
}

wait_for_pid_exit() {
  pid=$1
  attempts=0
  while kill -0 "$pid" 2>/dev/null; do
    attempts=$((attempts + 1))
    if [ "$attempts" -ge 10 ]; then
      echo "abbey-bot process $pid did not exit within 10 seconds" >&2
      return 1
    fi
    sleep 1
  done
}

wait_until_unloaded() {
  attempts=0
  while launchctl print "gui/$UID_NUM/$LABEL" >/dev/null 2>&1; do
    attempts=$((attempts + 1))
    if [ "$attempts" -ge 10 ]; then
      echo "launchd service did not unload within 10 seconds" >&2
      return 1
    fi
    sleep 1
  done
}

wait_for_service_pid() {
  attempts=0
  while [ "$attempts" -lt 10 ]; do
    started_pid=$(service_pid || true)
    if [ -n "$started_pid" ]; then
      printf '%s\n' "$started_pid"
      return 0
    fi
    attempts=$((attempts + 1))
    sleep 1
  done
  echo "launchd service did not acquire a PID within 10 seconds" >&2
  return 1
}

stop_service() {
  old_pid=$(service_pid || true)
  launchctl bootout "gui/$UID_NUM/$LABEL" 2>/dev/null || true
  if [ -n "$old_pid" ]; then
    wait_for_pid_exit "$old_pid"
  fi
  wait_until_unloaded
}

if [ "${1:-}" = "--uninstall" ]; then
  stop_service
  rm -f "$PLIST_DST"
  echo "unloaded and removed $PLIST_DST (binary, data, and env left in place)"
  exit 0
fi

if [ ! -f "$ENV_FILE" ]; then
  cat >&2 <<MSG
missing $ENV_FILE — create it (chmod 600) from .env.example. At least:
  DISCORD_TOKEN=...
and for a bot that answers (not the honest 'no backend' line):
  ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434  ABBEY_BOT_LLM_MODEL=gpt-oss:20b
optional, as you ran it by hand: ABBEY_VISION_ENDPOINT/ABBEY_VISION_MODEL,
  ABBEY_GUILD_ID, ABBEY_MESSAGE_CONTENT, ABBEY_QUIET, RUST_LOG; live voice also
  needs ABBEY_VOICE_GUILD_ID, ABBEY_VOICE_CHANNEL_ID, and OPENAI_API_KEY
(launchd always uses $DATA_DIR; ABBEY_DATA_DIR in this file is ignored.)
MSG
  exit 1
fi

echo "== build"
cargo build --release --locked

echo "== install"
mkdir -p "$BIN_DIR" "$DATA_DIR" "$LOG_DIR" "$HOME/Library/LaunchAgents" "$HOME/.local/share/abbey-bot"
chmod 600 "$ENV_FILE"
chmod 700 "$BIN_DIR" "$DATA_DIR" "$LOG_DIR" "$HOME/.local/share/abbey-bot"

BIN_STAGE=
PLIST_STAGE=
BIN_BACKUP=
PLIST_BACKUP=
HAD_BIN=0
HAD_PLIST=0
ROLLBACK=0

cleanup() {
  if [ -n "$BIN_STAGE" ]; then
    rm -f "$BIN_STAGE"
  fi
  if [ -n "$PLIST_STAGE" ]; then
    rm -f "$PLIST_STAGE"
  fi
  if [ "$ROLLBACK" -eq 1 ]; then
    if ! stop_service; then
      echo "automatic rollback is unsafe while the replacement is still running" >&2
      echo "backups retained: $BIN_BACKUP $PLIST_BACKUP" >&2
      return
    fi
    if [ "$HAD_BIN" -eq 1 ]; then
      mv -f "$BIN_BACKUP" "$BIN_DST"
    else
      rm -f "$BIN_DST"
    fi
    if [ "$HAD_PLIST" -eq 1 ]; then
      mv -f "$PLIST_BACKUP" "$PLIST_DST"
      if ! launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"; then
        echo "prior installation restored, but launchd could not restart it" >&2
      fi
    else
      rm -f "$PLIST_DST"
    fi
  else
    if [ -n "$BIN_BACKUP" ]; then
      rm -f "$BIN_BACKUP"
    fi
    if [ -n "$PLIST_BACKUP" ]; then
      rm -f "$PLIST_BACKUP"
    fi
  fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

BIN_STAGE=$(mktemp "$BIN_DIR/.abbey-bot.new.XXXXXX")
PLIST_STAGE=$(mktemp "$HOME/Library/LaunchAgents/.$LABEL.new.XXXXXX")

/bin/cp target/release/abbey-bot "$BIN_STAGE"
chmod 700 "$BIN_STAGE"
/bin/cp "$PLIST_SRC" "$PLIST_STAGE"
plutil -replace WorkingDirectory -string "$HOME/.local/share/abbey-bot" "$PLIST_STAGE"
plutil -replace StandardOutPath -string "$LOG_DIR/abbey-bot.log" "$PLIST_STAGE"
plutil -replace StandardErrorPath -string "$LOG_DIR/abbey-bot.log" "$PLIST_STAGE"
plutil -lint "$PLIST_STAGE"

if [ -f "$BIN_DST" ]; then
  BIN_BACKUP=$(mktemp "$BIN_DIR/.abbey-bot.previous.XXXXXX")
  /bin/cp -p "$BIN_DST" "$BIN_BACKUP"
  HAD_BIN=1
fi
if [ -f "$PLIST_DST" ]; then
  PLIST_BACKUP=$(mktemp "$HOME/Library/LaunchAgents/.$LABEL.previous.XXXXXX")
  /bin/cp -p "$PLIST_DST" "$PLIST_BACKUP"
  HAD_PLIST=1
fi

echo "== (re)load"
ROLLBACK=1
stop_service
chmod -R go-rwx "$DATA_DIR" "$LOG_DIR"
mv -f "$BIN_STAGE" "$BIN_DST"
mv -f "$PLIST_STAGE" "$PLIST_DST"
launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"
NEW_PID=$(wait_for_service_pid) || {
  echo "launchd replacement did not start; restoring the prior installation" >&2
  exit 1
}
attempts=0
while [ "$attempts" -lt 5 ]; do
  sleep 1
  CURRENT_PID=$(service_pid || true)
  if [ "$CURRENT_PID" != "$NEW_PID" ]; then
    echo "launchd replacement did not keep one stable process; restoring the prior installation" >&2
    exit 1
  fi
  attempts=$((attempts + 1))
done
SERVICE_STATUS=$(launchctl print "gui/$UID_NUM/$LABEL")
printf '%s\n' "$SERVICE_STATUS"
ROLLBACK=0
if [ -n "$BIN_BACKUP" ]; then
  rm -f "$BIN_BACKUP"
fi
if [ -n "$PLIST_BACKUP" ]; then
  rm -f "$PLIST_BACKUP"
fi
echo "log: $LOG_DIR/abbey-bot.log"
