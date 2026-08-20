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
STATE_DIR="$HOME/.local/share/abbey-bot"
LOG_DIR="$HOME/Library/Logs/abbey-bot"
ENV_FILE="$HOME/.config/abbey-bot/env"
UID_NUM=$(id -u)
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
ROLLBACK_PARENT="$STATE_DIR/rollback/abbey"
ROLLBACK_ROOT=
INSTALL_LOCK="$STATE_DIR/install.lock"
LOCK_HELD=0
RECOVERY_RETAINED=0

restore_interrupt_traps() {
  trap 'exit 1' HUP INT TERM
}

acquire_install_lock() {
  mkdir -p "$STATE_DIR"
  chmod 700 "$STATE_DIR"
  # Keep mkdir, PID publication, and LOCK_HELD synchronized even if the user
  # interrupts at the exact instant this process takes ownership.
  trap '' HUP INT TERM
  if ! mkdir "$INSTALL_LOCK" 2>/dev/null; then
    lock_owner=unknown
    if [ -r "$INSTALL_LOCK/pid" ]; then
      lock_owner=$(/usr/bin/sed -n '1p' "$INSTALL_LOCK/pid")
    fi
    restore_interrupt_traps
    echo "another Abbey install transaction holds $INSTALL_LOCK (pid $lock_owner)" >&2
    echo "remove the lock only after verifying that transaction is no longer running" >&2
    return 1
  fi
  if ! printf '%s\n' "$$" >"$INSTALL_LOCK/pid"; then
    rm -f "$INSTALL_LOCK/pid" 2>/dev/null || true
    rmdir "$INSTALL_LOCK" 2>/dev/null || true
    restore_interrupt_traps
    echo "could not publish the Abbey install-lock owner" >&2
    return 1
  fi
  LOCK_HELD=1
  restore_interrupt_traps
}

release_install_lock() {
  if [ "$LOCK_HELD" -ne 1 ]; then
    return 0
  fi
  lock_owner=$(/usr/bin/sed -n '1p' "$INSTALL_LOCK/pid" 2>/dev/null || true)
  if [ "$lock_owner" != "$$" ]; then
    echo "refusing to release Abbey install lock owned by ${lock_owner:-unknown}" >&2
    return 1
  fi
  if ! rm "$INSTALL_LOCK/pid"; then
    echo "failed to remove Abbey install-lock PID record" >&2
    return 1
  fi
  if ! rmdir "$INSTALL_LOCK"; then
    printf '%s\n' "$$" >"$INSTALL_LOCK/pid" 2>/dev/null || true
    echo "failed to release Abbey install lock: $INSTALL_LOCK" >&2
    return 1
  fi
  LOCK_HELD=0
}

release_only() {
  exit_status=$?
  trap - EXIT
  trap '' HUP INT TERM
  set +e
  if ! release_install_lock; then
    exit_status=1
  fi
  exit "$exit_status"
}

path_is_present() {
  [ -e "$1" ] || [ -L "$1" ]
}

path_is_regular_file() {
  [ -f "$1" ] && [ ! -L "$1" ]
}

require_path_absent() {
  guarded_path=$1
  guarded_description=$2
  if path_is_present "$guarded_path"; then
    echo "refusing to overwrite existing $guarded_description: $guarded_path" >&2
    return 1
  fi
  return 0
}

remove_regular_file() {
  removable_path=$1
  removable_description=$2
  if path_is_regular_file "$removable_path"; then
    if ! rm "$removable_path"; then
      echo "could not remove $removable_description: $removable_path" >&2
      return 1
    fi
  elif path_is_present "$removable_path"; then
    echo "$removable_description has an unexpected file type: $removable_path" >&2
    return 1
  fi
  return 0
}

restore_regular_backup() {
  restore_source=$1
  restore_destination=$2
  restore_description=$3
  if ! path_is_regular_file "$restore_source"; then
    echo "restorable $restore_description is missing or has an unexpected file type:" >&2
    echo "  $restore_source" >&2
    return 1
  fi
  if ! remove_regular_file "$restore_destination" "candidate $restore_description"; then
    return 1
  fi
  if ! require_path_absent "$restore_destination" "$restore_description restore destination"; then
    return 1
  fi
  if ! /bin/cp -p "$restore_source" "$restore_destination"; then
    echo "could not restore the prior $restore_description" >&2
    return 1
  fi
}

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

wait_for_stable_service_pid() {
  candidate_pid=$(wait_for_service_pid) || return 1
  stable_attempts=0
  while [ "$stable_attempts" -lt 5 ]; do
    sleep 1
    observed_pid=$(service_pid || true)
    if [ "$observed_pid" != "$candidate_pid" ]; then
      echo "launchd service did not keep one stable process" >&2
      return 1
    fi
    stable_attempts=$((stable_attempts + 1))
  done
  printf '%s\n' "$candidate_pid"
}

stop_service() {
  old_pid=$(service_pid || true)
  launchctl bootout "gui/$UID_NUM/$LABEL" 2>/dev/null || true
  if [ -n "$old_pid" ]; then
    wait_for_pid_exit "$old_pid"
  fi
  wait_until_unloaded
}

MODE=install
case "$#:${1:-}" in
  0:) ;;
  1:--uninstall) MODE=uninstall ;;
  *)
    echo "usage: $0 [--uninstall]" >&2
    exit 2
    ;;
esac

if [ "$MODE" = uninstall ]; then
  trap release_only EXIT
  restore_interrupt_traps
  acquire_install_lock
  if path_is_present "$BIN_DST" && ! path_is_regular_file "$BIN_DST"; then
    echo "live Abbey binary has an unexpected file type: $BIN_DST" >&2
    exit 1
  fi
  if path_is_present "$PLIST_DST" && ! path_is_regular_file "$PLIST_DST"; then
    echo "live Abbey launchd plist has an unexpected file type: $PLIST_DST" >&2
    exit 1
  fi
  stop_service
  remove_regular_file "$PLIST_DST" "live Abbey launchd plist"
  echo "unloaded and removed $PLIST_DST (binary, data, and env left in place)"
  exit 0
fi

if [ ! -f "$ENV_FILE" ]; then
  cat >&2 <<MSG
missing $ENV_FILE — create it (chmod 600) from .env.example. At least:
  DISCORD_TOKEN=...
and for a bot that answers (not the honest 'no backend' line):
  ABBEY_BOT_LLM_ENDPOINT=http://127.0.0.1:11434  ABBEY_BOT_LLM_MODEL=gemma4:12b
optional, as you ran it by hand: ABBEY_VISION_ENDPOINT/ABBEY_VISION_MODEL,
  ABBEY_GUILD_ID, ABBEY_MESSAGE_CONTENT, ABBEY_QUIET, RUST_LOG; live voice also
  needs ABBEY_VOICE_GUILD_ID + ABBEY_VOICE_CHANNEL_ID. Local mode is the
  default and uses deploy/install-mlx-audio-launchd.sh; OpenAI audio is selected
  only by ABBEY_VOICE_MODE=openai and then requires OPENAI_API_KEY.
  Apple-silicon Gemma 4 reasoning + tools + vision can use the separately
  pinned deploy/install-mlx-vlm-launchd.sh profile on 127.0.0.1:8282; use the
  exact snapshot path printed by that installer as both model variables.
(launchd always uses $DATA_DIR; ABBEY_DATA_DIR in this file is ignored.)
MSG
  exit 1
fi

trap release_only EXIT
restore_interrupt_traps
acquire_install_lock

echo "== build"
cargo build --release --locked

echo "== install"
mkdir -p "$BIN_DIR" "$DATA_DIR" "$LOG_DIR" "$HOME/Library/LaunchAgents" \
  "$STATE_DIR" "$ROLLBACK_PARENT"
ROLLBACK_ROOT=$(mktemp -d "$ROLLBACK_PARENT/$STAMP.XXXXXX")
chmod 600 "$ENV_FILE"
chmod 700 "$BIN_DIR" "$DATA_DIR" "$LOG_DIR" "$STATE_DIR" \
  "$STATE_DIR/rollback" "$ROLLBACK_PARENT" "$ROLLBACK_ROOT"

BIN_STAGE=
PLIST_STAGE=
BIN_BACKUP=
PLIST_BACKUP=
ENV_BACKUP=
HAD_BIN=0
HAD_PLIST=0
ROLLBACK=0

cleanup() {
  exit_status=$?
  trap - EXIT
  trap '' HUP INT TERM
  set +e
  if [ "$ROLLBACK" -eq 1 ]; then
    rollback_ok=1
    if ! stop_service; then
      echo "automatic rollback is unsafe while the replacement is still running" >&2
      echo "backups retained: $BIN_BACKUP $PLIST_BACKUP" >&2
      rollback_ok=0
    else
      if [ "$HAD_BIN" -eq 1 ]; then
        if ! restore_regular_backup "$BIN_BACKUP" "$BIN_DST" "Abbey binary"; then
          rollback_ok=0
        fi
      elif ! remove_regular_file "$BIN_DST" "candidate Abbey binary"; then
        rollback_ok=0
      fi
      if [ "$HAD_PLIST" -eq 1 ]; then
        if ! restore_regular_backup "$PLIST_BACKUP" "$PLIST_DST" \
          "Abbey launchd plist"; then
          rollback_ok=0
        fi
      elif ! remove_regular_file "$PLIST_DST" "candidate Abbey launchd plist"; then
        rollback_ok=0
      fi
      if [ "$rollback_ok" -eq 1 ] && [ "$HAD_PLIST" -eq 1 ]; then
        if ! launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"; then
          echo "prior installation restored, but launchd could not restart it" >&2
          rollback_ok=0
        else
          restored_pid=$(wait_for_stable_service_pid || true)
          if [ -z "$restored_pid" ]; then
            echo "prior installation restarted without a stable process" >&2
            rollback_ok=0
          fi
        fi
      fi
    fi
    if [ "$rollback_ok" -ne 1 ]; then
      RECOVERY_RETAINED=1
      echo "rollback incomplete; candidate, backups, and install lock retained" >&2
    else
      ROLLBACK=0
    fi
  fi
  if [ "$RECOVERY_RETAINED" -eq 1 ]; then
    echo "manual recovery lock: $INSTALL_LOCK" >&2
    exit 1
  fi
  if [ -n "$BIN_STAGE" ]; then
    rm -f "$BIN_STAGE"
  fi
  if [ -n "$PLIST_STAGE" ]; then
    rm -f "$PLIST_STAGE"
  fi
  if ! release_install_lock; then
    exit_status=1
  fi
  exit "$exit_status"
}
trap cleanup EXIT
restore_interrupt_traps

BIN_STAGE=$(mktemp "$BIN_DIR/.abbey-bot.new.XXXXXX")
PLIST_STAGE=$(mktemp "$HOME/Library/LaunchAgents/.$LABEL.new.XXXXXX")

/bin/cp target/release/abbey-bot "$BIN_STAGE"
chmod 700 "$BIN_STAGE"
RELEASE_SHA_OUTPUT=$(shasum -a 256 target/release/abbey-bot)
RELEASE_SHA=${RELEASE_SHA_OUTPUT%% *}
if [ "${#RELEASE_SHA}" -ne 64 ]; then
  echo "could not compute the release binary SHA-256" >&2
  exit 1
fi
case "$RELEASE_SHA" in
  *[!0-9a-f]*) echo "release SHA-256 contained invalid characters" >&2; exit 1 ;;
esac
/bin/cp "$PLIST_SRC" "$PLIST_STAGE"
plutil -replace WorkingDirectory -string "$HOME/.local/share/abbey-bot" "$PLIST_STAGE"
plutil -replace StandardOutPath -string "$LOG_DIR/abbey-bot.log" "$PLIST_STAGE"
plutil -replace StandardErrorPath -string "$LOG_DIR/abbey-bot.log" "$PLIST_STAGE"
plutil -lint "$PLIST_STAGE"

if path_is_present "$BIN_DST" && ! path_is_regular_file "$BIN_DST"; then
  echo "live Abbey binary has an unexpected file type: $BIN_DST" >&2
  exit 1
fi
if path_is_present "$PLIST_DST" && ! path_is_regular_file "$PLIST_DST"; then
  echo "live Abbey launchd plist has an unexpected file type: $PLIST_DST" >&2
  exit 1
fi
if path_is_regular_file "$BIN_DST"; then
  BIN_BACKUP="$ROLLBACK_ROOT/abbey-bot"
  require_path_absent "$BIN_BACKUP" "Abbey binary backup path"
  /bin/cp -p "$BIN_DST" "$BIN_BACKUP"
  HAD_BIN=1
fi
if path_is_regular_file "$PLIST_DST"; then
  PLIST_BACKUP="$ROLLBACK_ROOT/$LABEL.plist"
  require_path_absent "$PLIST_BACKUP" "Abbey plist backup path"
  /bin/cp -p "$PLIST_DST" "$PLIST_BACKUP"
  HAD_PLIST=1
fi
ENV_BACKUP="$ROLLBACK_ROOT/env"
/bin/cp -p "$ENV_FILE" "$ENV_BACKUP"
chmod 600 "$ENV_BACKUP"

echo "== (re)load"
ROLLBACK=1
stop_service
chmod -R go-rwx "$DATA_DIR" "$LOG_DIR"
if ! remove_regular_file "$BIN_DST" "live Abbey binary before publish"; then
  exit 1
fi
if ! require_path_absent "$BIN_DST" "Abbey binary publish path"; then
  exit 1
fi
if ! mv "$BIN_STAGE" "$BIN_DST"; then
  echo "could not publish the staged Abbey binary" >&2
  exit 1
fi
BIN_STAGE=
if ! remove_regular_file "$PLIST_DST" "live Abbey launchd plist before publish"; then
  exit 1
fi
if ! require_path_absent "$PLIST_DST" "Abbey plist publish path"; then
  exit 1
fi
if ! mv "$PLIST_STAGE" "$PLIST_DST"; then
  echo "could not publish the staged Abbey launchd plist" >&2
  exit 1
fi
PLIST_STAGE=
INSTALLED_SHA_OUTPUT=$(shasum -a 256 "$BIN_DST")
INSTALLED_SHA=${INSTALLED_SHA_OUTPUT%% *}
if [ "$INSTALLED_SHA" != "$RELEASE_SHA" ]; then
  echo "installed binary hash does not match the gated release artifact" >&2
  exit 1
fi
launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"
NEW_PID=$(wait_for_stable_service_pid) || {
  echo "launchd replacement did not start; restoring the prior installation" >&2
  exit 1
}
SERVICE_STATUS=$(launchctl print "gui/$UID_NUM/$LABEL")
printf '%s\n' "$SERVICE_STATUS"
ROLLBACK=0
echo "release SHA-256:   $RELEASE_SHA"
echo "installed SHA-256: $INSTALLED_SHA"
echo "previous installation and environment retained: $ROLLBACK_ROOT"
echo "log: $LOG_DIR/abbey-bot.log"
