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
REQUIREMENTS=deploy/mlx-audio-requirements.txt
BUILD_CONSTRAINTS=deploy/mlx-audio-build-constraints.txt
INSTALL_DIR="$HOME/.local/libexec/abbey-bot"
RUNNER_DST="$INSTALL_DIR/run-mlx-audio"
STATE_DIR="$HOME/.local/share/abbey-bot/mlx-audio"
CACHE_DIR="$STATE_DIR/huggingface"
INSTALL_LOCK="$STATE_DIR/install.lock"
CURRENT_VENV_FILE="$STATE_DIR/current-venv"
LOG_DIR="$HOME/Library/Logs/abbey-bot"
UID_NUM=$(id -u)
STT_MODEL=mlx-community/whisper-large-v3-turbo-asr-fp16
STT_REVISION=624c19c9af5603fa73b83bce14d4aeea96156d18
TTS_MODEL=mlx-community/Kokoro-82M-bf16
TTS_REVISION=a71e4d38b236d968966a2002c4c895dbd12b1c3c
VOICE_MODEL=prince-canuma/Kokoro-82M
VOICE_REVISION=e02c9eada7ce7416798af36b190a8a2dd2ecd566
LIVE_PORT=8181
LOCK_HELD=0
MODE=install

case "$#:${1:-}" in
  0:) ;;
  1:--uninstall) MODE=uninstall ;;
  *)
    echo "usage: $0 [--uninstall]" >&2
    exit 2
    ;;
esac

STAGE_VENV=
STAGE_CACHE=
BACKUP_ROOT=
RUNNER_STAGE=
PLIST_STAGE=
SMOKE_DIR=
CURRENT_VENV_STAGE=
STAGED_PID=
LIVE_PID=
SWITCH_STARTED=0
PREVIOUS_LOADED=0
HAD_CACHE=0
HAD_RUNNER=0
HAD_PLIST=0
HAD_VENV_POINTER=0
NEW_CACHE_INSTALLED=0
NEW_RUNNER_INSTALLED=0
NEW_PLIST_INSTALLED=0
NEW_VENV_POINTER_INSTALLED=0
KEEP_STAGE_VENV=0
RECOVERY_RETAINED=0
PREVIOUS_VENV=
PREVIOUS_VENV_VALID=0
UNINSTALL_BACKUP_ROOT=
UNINSTALL_RUNNER_BACKUP=
UNINSTALL_PLIST_BACKUP=
UNINSTALL_RUNNER_MOVED=0
UNINSTALL_PLIST_MOVED=0

arm_interrupt_traps() {
  trap 'exit 1' HUP INT TERM
}

acquire_install_lock() {
  # A signal between mkdir, PID publication, and LOCK_HELD would otherwise
  # leave an ownerless lock or make cleanup overlook a lock owned by us.
  trap '' HUP INT TERM
  if ! mkdir "$INSTALL_LOCK" 2>/dev/null; then
    lock_owner=unknown
    if [ -r "$INSTALL_LOCK/pid" ]; then
      lock_owner=$(/usr/bin/sed -n '1p' "$INSTALL_LOCK/pid")
    fi
    echo "another MLX-Audio install transaction holds $INSTALL_LOCK (pid $lock_owner)" >&2
    echo "remove the lock only after verifying that transaction is no longer running" >&2
    arm_interrupt_traps
    return 1
  fi
  if ! printf '%s\n' "$$" >"$INSTALL_LOCK/pid"; then
    rm -f "$INSTALL_LOCK/pid" 2>/dev/null || true
    rmdir "$INSTALL_LOCK" 2>/dev/null || true
    echo "could not publish the MLX-Audio install-lock owner" >&2
    arm_interrupt_traps
    return 1
  fi
  LOCK_HELD=1
  arm_interrupt_traps
}

release_install_lock() {
  if [ "$LOCK_HELD" -ne 1 ]; then
    return 0
  fi
  lock_owner=$(/usr/bin/sed -n '1p' "$INSTALL_LOCK/pid" 2>/dev/null || true)
  if [ "$lock_owner" != "$$" ]; then
    echo "refusing to release MLX-Audio install lock owned by ${lock_owner:-unknown}" >&2
    return 1
  fi
  if ! rm "$INSTALL_LOCK/pid"; then
    echo "failed to remove MLX-Audio install-lock PID record" >&2
    return 1
  fi
  if ! rmdir "$INSTALL_LOCK"; then
    printf '%s\n' "$$" >"$INSTALL_LOCK/pid" 2>/dev/null || true
    echo "failed to release MLX-Audio install lock: $INSTALL_LOCK" >&2
    return 1
  fi
  LOCK_HELD=0
}

path_is_present() {
  [ -e "$1" ] || [ -L "$1" ]
}

path_is_directory() {
  [ -d "$1" ] && [ ! -L "$1" ]
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
}

move_without_nesting() {
  move_source=$1
  move_destination=$2
  move_description=$3
  require_path_absent "$move_destination" "$move_description destination" || return 1
  if ! mv "$move_source" "$move_destination"; then
    echo "could not move $move_description: $move_source -> $move_destination" >&2
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

pid_owns_loopback_listener() {
  listener_pid=$1
  listener_port=$2
  case "$listener_pid" in
    ''|*[!0-9]*) return 1 ;;
  esac
  case "$listener_port" in
    ''|*[!0-9]*) return 1 ;;
  esac
  if ! kill -0 "$listener_pid" 2>/dev/null; then
    return 1
  fi
  if [ ! -x /usr/sbin/lsof ]; then
    echo "cannot verify listener ownership because /usr/sbin/lsof is unavailable" >&2
    return 1
  fi
  listener_records=$(/usr/sbin/lsof -nP -a -p "$listener_pid" \
    "-iTCP@127.0.0.1:$listener_port" -sTCP:LISTEN -Fpn 2>/dev/null) || return 1
  listener_saw_pid=0
  listener_saw_address=0
  while IFS= read -r listener_record; do
    case "$listener_record" in
      "p$listener_pid") listener_saw_pid=1 ;;
      "n127.0.0.1:$listener_port") listener_saw_address=1 ;;
    esac
  done <<EOF
$listener_records
EOF
  [ "$listener_saw_pid" -eq 1 ] && [ "$listener_saw_address" -eq 1 ]
}

require_pid_owns_loopback_listener() {
  listener_pid=$1
  listener_port=$2
  listener_name=$3
  if ! pid_owns_loopback_listener "$listener_pid" "$listener_port"; then
    echo "$listener_name pid $listener_pid does not own 127.0.0.1:$listener_port" >&2
    return 1
  fi
}

require_same_live_pid() {
  expected_pid=$1
  checkpoint=$2
  if ! observed_pid=$(service_pid); then
    echo "MLX-Audio has no exact launchd pid $checkpoint" >&2
    return 1
  fi
  if [ "$observed_pid" != "$expected_pid" ]; then
    echo "MLX-Audio launchd pid changed from $expected_pid to $observed_pid $checkpoint" >&2
    return 1
  fi
  require_pid_owns_loopback_listener "$expected_pid" "$LIVE_PORT" \
    "MLX-Audio $checkpoint"
}

wait_for_same_live_listener() {
  expected_pid=$1
  listener_attempts=$2
  checkpoint=$3
  attempt=0
  while [ "$attempt" -lt "$listener_attempts" ]; do
    observed_pid=$(service_pid || true)
    if [ "$observed_pid" != "$expected_pid" ]; then
      echo "MLX-Audio launchd pid changed from $expected_pid to ${observed_pid:-none}" >&2
      echo "while waiting for its loopback listener $checkpoint" >&2
      return 1
    fi
    if pid_owns_loopback_listener "$expected_pid" "$LIVE_PORT"; then
      return 0
    fi
    attempt=$((attempt + 1))
    sleep 1
  done
  echo "MLX-Audio pid $expected_pid did not own 127.0.0.1:$LIVE_PORT $checkpoint" >&2
  return 1
}

wait_for_service_pid() {
  pid_attempts=0
  while [ "$pid_attempts" -lt 20 ]; do
    if candidate_pid=$(service_pid) && kill -0 "$candidate_pid" 2>/dev/null; then
      printf '%s\n' "$candidate_pid"
      return 0
    fi
    pid_attempts=$((pid_attempts + 1))
    sleep 1
  done
  echo "MLX-Audio launchd service did not publish a live exact pid within 20 seconds" >&2
  return 1
}

restart_previous_service() {
  if ! path_is_regular_file "$PLIST_DST"; then
    echo "restored MLX-Audio plist is missing or has an unexpected type" >&2
    return 1
  fi
  if ! launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"; then
    echo "could not bootstrap the restored MLX-Audio service" >&2
    return 1
  fi
  if ! expected_restored_pid=$(wait_for_service_pid); then
    return 1
  fi
  if ! wait_for_same_live_listener "$expected_restored_pid" 60 \
    "before restored health"; then
    return 1
  fi
  if ! wait_for_health "$LIVE_PORT" 60 "restored MLX-Audio"; then
    return 1
  fi
  require_same_live_pid "$expected_restored_pid" "after restored health"
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

if [ "$MODE" = install ]; then
  if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
    echo "Abbey's MLX-Audio sidecar requires Apple Silicon macOS" >&2
    exit 1
  fi
  UV_BIN=$(command -v uv || true)
  if [ -z "$UV_BIN" ]; then
    echo "uv is required to install the pinned Python environment" >&2
    exit 1
  fi
  if [ ! -s "$REQUIREMENTS" ]; then
    echo "missing hash-locked MLX-Audio requirements: $REQUIREMENTS" >&2
    exit 1
  fi
  if [ ! -s "$BUILD_CONSTRAINTS" ]; then
    echo "missing hash-locked MLX-Audio build constraints: $BUILD_CONSTRAINTS" >&2
    exit 1
  fi
fi

stop_staged_service() {
  if [ -n "$STAGED_PID" ]; then
    kill "$STAGED_PID" 2>/dev/null || true
    wait "$STAGED_PID" 2>/dev/null || true
    STAGED_PID=
  fi
}

restore_previous_install() {
  trap '' HUP INT TERM
  echo "== candidate failed; restore previous MLX-Audio install ==" >&2
  if ! stop_service; then
    RECOVERY_RETAINED=1
    echo "rollback stopped before mutating files because the candidate service is still loaded" >&2
    echo "candidate install retained at $STAGE_VENV; backups retained at $BACKUP_ROOT" >&2
    return 1
  fi
  if [ "$NEW_CACHE_INSTALLED" -eq 1 ]; then
    if ! path_is_directory "$CACHE_DIR"; then
      echo "candidate cache is missing or has an unexpected file type: $CACHE_DIR" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! move_without_nesting "$CACHE_DIR" "$BACKUP_ROOT/failed-cache" \
      "failed candidate cache"; then
      echo "could not remove the candidate cache from its live path" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
  fi
  if [ "$NEW_RUNNER_INSTALLED" -eq 1 ]; then
    if ! path_is_regular_file "$RUNNER_DST"; then
      echo "candidate runner is missing or has an unexpected file type: $RUNNER_DST" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! move_without_nesting "$RUNNER_DST" "$BACKUP_ROOT/failed-runner" \
      "failed candidate runner"; then
      echo "could not remove the candidate runner from its live path" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
  fi
  if [ "$NEW_PLIST_INSTALLED" -eq 1 ]; then
    if ! path_is_regular_file "$PLIST_DST"; then
      echo "candidate plist is missing or has an unexpected file type: $PLIST_DST" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! move_without_nesting "$PLIST_DST" "$BACKUP_ROOT/failed-plist" \
      "failed candidate plist"; then
      echo "could not remove the candidate plist from its live path" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
  fi
  if [ "$NEW_VENV_POINTER_INSTALLED" -eq 1 ]; then
    if ! path_is_regular_file "$CURRENT_VENV_FILE"; then
      echo "candidate venv pointer is missing or has an unexpected file type: $CURRENT_VENV_FILE" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! move_without_nesting "$CURRENT_VENV_FILE" \
      "$BACKUP_ROOT/failed-current-venv" "failed candidate venv pointer"; then
      echo "could not remove the candidate venv pointer from its live path" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
  fi
  if [ "$HAD_CACHE" -eq 1 ]; then
    if ! path_is_directory "$BACKUP_ROOT/cache"; then
      echo "previous cache backup is missing or has an unexpected type" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! move_without_nesting "$BACKUP_ROOT/cache" "$CACHE_DIR" "previous cache"; then
      echo "could not restore the previous cache" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
  fi
  if [ "$HAD_RUNNER" -eq 1 ]; then
    if ! path_is_regular_file "$BACKUP_ROOT/runner"; then
      echo "previous runner backup is missing or has an unexpected type" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! move_without_nesting "$BACKUP_ROOT/runner" "$RUNNER_DST" "previous runner"; then
      echo "could not restore the previous runner" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
  fi
  if [ "$HAD_PLIST" -eq 1 ]; then
    if ! path_is_regular_file "$BACKUP_ROOT/plist"; then
      echo "previous plist backup is missing or has an unexpected type" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! move_without_nesting "$BACKUP_ROOT/plist" "$PLIST_DST" "previous plist"; then
      echo "could not restore the previous plist" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
  fi
  if [ "$HAD_VENV_POINTER" -eq 1 ]; then
    if ! path_is_regular_file "$BACKUP_ROOT/current-venv"; then
      echo "previous venv pointer backup is missing or has an unexpected type" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! move_without_nesting "$BACKUP_ROOT/current-venv" "$CURRENT_VENV_FILE" \
      "previous venv pointer"; then
      echo "could not restore the previous venv pointer" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
  fi
  if [ "$PREVIOUS_LOADED" -eq 1 ]; then
    if ! restart_previous_service; then
      RECOVERY_RETAINED=1
      echo "previous files were restored but the service did not recover; recovery files retained" >&2
      return 1
    fi
    echo "previous MLX-Audio service restored" >&2
  fi
  SWITCH_STARTED=0
}

restore_interrupted_uninstall() {
  trap '' HUP INT TERM
  echo "== uninstall interrupted; restore MLX-Audio launch files ==" >&2
  if ! stop_service; then
    RECOVERY_RETAINED=1
    echo "could not stop MLX-Audio while restoring the interrupted uninstall" >&2
    return 1
  fi
  if [ "$UNINSTALL_RUNNER_MOVED" -eq 1 ]; then
    if ! path_is_regular_file "$UNINSTALL_RUNNER_BACKUP" || \
      ! move_without_nesting "$UNINSTALL_RUNNER_BACKUP" "$RUNNER_DST" \
        "uninstalled runner"; then
      RECOVERY_RETAINED=1
      echo "could not restore the runner after interrupted uninstall" >&2
      return 1
    fi
    UNINSTALL_RUNNER_MOVED=0
  fi
  if [ "$UNINSTALL_PLIST_MOVED" -eq 1 ]; then
    if ! path_is_regular_file "$UNINSTALL_PLIST_BACKUP" || \
      ! move_without_nesting "$UNINSTALL_PLIST_BACKUP" "$PLIST_DST" \
        "uninstalled plist"; then
      RECOVERY_RETAINED=1
      echo "could not restore the plist after interrupted uninstall" >&2
      return 1
    fi
    UNINSTALL_PLIST_MOVED=0
  fi
  if [ "$PREVIOUS_LOADED" -eq 1 ]; then
    if ! restart_previous_service; then
      RECOVERY_RETAINED=1
      echo "uninstall rollback restored files but not the prior service" >&2
      return 1
    fi
  fi
  SWITCH_STARTED=0
}

cleanup() {
  cleanup_status=0
  stop_staged_service
  if [ "$RECOVERY_RETAINED" -eq 1 ]; then
    echo "recovery artifacts were retained without further cleanup" >&2
    echo "install lock retained at $INSTALL_LOCK until manual recovery is complete" >&2
    return 1
  fi
  if [ "$KEEP_STAGE_VENV" -eq 0 ]; then
    if [ -n "$STAGE_VENV" ]; then
      rm -rf "$STAGE_VENV" || cleanup_status=1
    fi
  fi
  if [ -n "$STAGE_CACHE" ]; then
    rm -rf "$STAGE_CACHE" || cleanup_status=1
  fi
  if [ -n "$SMOKE_DIR" ]; then
    rm -rf "$SMOKE_DIR" || cleanup_status=1
  fi
  if [ -n "$BACKUP_ROOT" ]; then
    if [ "$KEEP_STAGE_VENV" -eq 1 ] && \
      { [ "$HAD_CACHE" -eq 1 ] || [ "$HAD_RUNNER" -eq 1 ] || \
        [ "$HAD_PLIST" -eq 1 ] || [ "$HAD_VENV_POINTER" -eq 1 ]; }; then
      : # Preserve the last healthy release until live voice acceptance is recorded.
    else
      rm -rf "$BACKUP_ROOT" || cleanup_status=1
    fi
  fi
  if [ -n "$RUNNER_STAGE" ]; then
    rm -f "$RUNNER_STAGE" || cleanup_status=1
  fi
  if [ -n "$PLIST_STAGE" ]; then
    rm -f "$PLIST_STAGE" || cleanup_status=1
  fi
  if [ -n "$CURRENT_VENV_STAGE" ]; then
    rm -f "$CURRENT_VENV_STAGE" || cleanup_status=1
  fi
  if [ "$MODE" = uninstall ] && [ "$SWITCH_STARTED" -eq 0 ] && \
    [ "$UNINSTALL_RUNNER_MOVED" -eq 0 ] && [ "$UNINSTALL_PLIST_MOVED" -eq 0 ] && \
    [ -n "$UNINSTALL_BACKUP_ROOT" ]; then
    rmdir "$UNINSTALL_BACKUP_ROOT" 2>/dev/null || true
  fi
  release_install_lock || cleanup_status=1
  return "$cleanup_status"
}

on_exit() {
  status=$1
  trap - EXIT
  trap '' HUP INT TERM
  set +e
  if [ "$status" -ne 0 ] && [ "$SWITCH_STARTED" -eq 1 ]; then
    if [ "$MODE" = uninstall ]; then
      restore_interrupted_uninstall || status=1
    else
      restore_previous_install || status=1
    fi
  fi
  cleanup || status=1
  exit "$status"
}
trap 'on_exit $?' EXIT
arm_interrupt_traps

echo "== create owner-only transaction layout =="
mkdir -p "$STATE_DIR"
chmod 700 "$STATE_DIR"
if [ "$MODE" = install ]; then
  mkdir -p "$INSTALL_DIR" "$LOG_DIR" "$HOME/Library/LaunchAgents"
  chmod 700 "$INSTALL_DIR" "$LOG_DIR"
fi
acquire_install_lock

if [ "$MODE" = uninstall ]; then
  if path_is_present "$RUNNER_DST" && ! path_is_regular_file "$RUNNER_DST"; then
    echo "MLX-Audio runner has an unexpected file type: $RUNNER_DST" >&2
    exit 1
  fi
  if path_is_present "$PLIST_DST" && ! path_is_regular_file "$PLIST_DST"; then
    echo "MLX-Audio plist has an unexpected file type: $PLIST_DST" >&2
    exit 1
  fi
  if launchctl print "gui/$UID_NUM/$LABEL" >/dev/null 2>&1; then
    PREVIOUS_LOADED=1
    if ! path_is_regular_file "$PLIST_DST"; then
      echo "loaded MLX-Audio service has no restorable regular plist" >&2
      exit 1
    fi
  fi
  mkdir -p "$STATE_DIR/uninstall"
  chmod 700 "$STATE_DIR/uninstall"
  UNINSTALL_BACKUP_ROOT=$(mktemp -d \
    "$STATE_DIR/uninstall/$(date -u +%Y%m%dT%H%M%SZ).$$.XXXXXX")
  UNINSTALL_RUNNER_BACKUP="$UNINSTALL_BACKUP_ROOT/runner"
  UNINSTALL_PLIST_BACKUP="$UNINSTALL_BACKUP_ROOT/plist"
  SWITCH_STARTED=1
  if ! stop_service; then
    echo "MLX-Audio service could not be stopped; uninstall rollback will retry" >&2
    exit 1
  fi
  if path_is_regular_file "$RUNNER_DST"; then
    trap '' HUP INT TERM
    if ! move_without_nesting "$RUNNER_DST" "$UNINSTALL_RUNNER_BACKUP" \
      "runner for uninstall"; then
      arm_interrupt_traps
      exit 1
    fi
    UNINSTALL_RUNNER_MOVED=1
    arm_interrupt_traps
  fi
  if path_is_regular_file "$PLIST_DST"; then
    trap '' HUP INT TERM
    if ! move_without_nesting "$PLIST_DST" "$UNINSTALL_PLIST_BACKUP" \
      "plist for uninstall"; then
      arm_interrupt_traps
      exit 1
    fi
    UNINSTALL_PLIST_MOVED=1
    arm_interrupt_traps
  fi
  SWITCH_STARTED=0
  echo "unloaded and removed MLX-Audio launch files; model cache and venv were retained"
  if [ "$UNINSTALL_RUNNER_MOVED" -eq 1 ]; then
    echo "previous runner retained: $UNINSTALL_RUNNER_BACKUP"
  fi
  if [ "$UNINSTALL_PLIST_MOVED" -eq 1 ]; then
    echo "previous plist retained: $UNINSTALL_PLIST_BACKUP"
  fi
  exit 0
fi

STAGE_VENV=$(mktemp -d "$INSTALL_DIR/.mlx-audio-venv.release.XXXXXX")
STAGE_CACHE=$(mktemp -d "$STATE_DIR/.huggingface-stage.XXXXXX")
mkdir -p "$STATE_DIR/rollback"
chmod 700 "$STATE_DIR/rollback"
BACKUP_ROOT=$(mktemp -d "$STATE_DIR/rollback/$(date -u +%Y%m%dT%H%M%SZ).XXXXXX")
RUNNER_STAGE=$(mktemp "$INSTALL_DIR/.run-mlx-audio.new.XXXXXX")
PLIST_STAGE=$(mktemp "$HOME/Library/LaunchAgents/.$LABEL.new.XXXXXX")
SMOKE_DIR=$(mktemp -d "${TMPDIR:-/tmp}/abbey-mlx-smoke.XXXXXX")

if path_is_regular_file "$CURRENT_VENV_FILE" && [ -r "$CURRENT_VENV_FILE" ]; then
  IFS= read -r PREVIOUS_VENV <"$CURRENT_VENV_FILE" || PREVIOUS_VENV=
  if [ "$(dirname "$PREVIOUS_VENV")" = "$INSTALL_DIR" ] && \
    [ ! -L "$PREVIOUS_VENV" ]; then
    case "$(basename "$PREVIOUS_VENV")" in
      .mlx-audio-venv.release.*) PREVIOUS_VENV_VALID=1 ;;
    esac
  fi
fi

echo "== install pinned MLX-Audio environment in sibling venv =="
"$UV_BIN" venv --clear --python 3.11 "$STAGE_VENV"
"$UV_BIN" pip install --python "$STAGE_VENV/bin/python" \
  --only-binary=:all: \
  --no-binary=webrtcvad \
  --require-hashes \
  --build-constraint "$BUILD_CONSTRAINTS" \
  --requirement "$REQUIREMENTS"

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
if ! require_pid_owns_loopback_listener "$STAGED_PID" "$SMOKE_PORT" \
  "staged MLX-Audio before acceptance"; then
  echo "staged MLX-Audio did not retain its exact loopback listener before acceptance" >&2
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
if ! require_pid_owns_loopback_listener "$STAGED_PID" "$SMOKE_PORT" \
  "staged MLX-Audio after acceptance"; then
  echo "staged MLX-Audio did not retain its exact loopback listener through acceptance" >&2
  tail -n 80 "$SMOKE_DIR/server.log" >&2 || true
  exit 1
fi
stop_staged_service

echo "== atomically switch the validated candidate into place =="
if path_is_present "$CACHE_DIR" && ! path_is_directory "$CACHE_DIR"; then
  echo "live MLX-Audio cache has an unexpected file type: $CACHE_DIR" >&2
  exit 1
fi
if path_is_present "$RUNNER_DST" && ! path_is_regular_file "$RUNNER_DST"; then
  echo "live MLX-Audio runner has an unexpected file type: $RUNNER_DST" >&2
  exit 1
fi
if path_is_present "$PLIST_DST" && ! path_is_regular_file "$PLIST_DST"; then
  echo "live MLX-Audio plist has an unexpected file type: $PLIST_DST" >&2
  exit 1
fi
if path_is_present "$CURRENT_VENV_FILE" && \
  ! path_is_regular_file "$CURRENT_VENV_FILE"; then
  echo "live MLX-Audio venv pointer has an unexpected file type: $CURRENT_VENV_FILE" >&2
  exit 1
fi
if launchctl print "gui/$UID_NUM/$LABEL" >/dev/null 2>&1; then
  PREVIOUS_LOADED=1
  if ! path_is_regular_file "$PLIST_DST"; then
    echo "loaded MLX-Audio service has no restorable regular plist" >&2
    exit 1
  fi
fi
SWITCH_STARTED=1
if ! stop_service; then
  echo "MLX-Audio service could not be stopped; publish files were left untouched" >&2
  exit 1
fi
if path_is_directory "$CACHE_DIR"; then
  trap '' HUP INT TERM
  HAD_CACHE=1
  if ! move_without_nesting "$CACHE_DIR" "$BACKUP_ROOT/cache" "live cache backup"; then
    arm_interrupt_traps
    exit 1
  fi
  arm_interrupt_traps
fi
if path_is_regular_file "$RUNNER_DST"; then
  trap '' HUP INT TERM
  HAD_RUNNER=1
  if ! move_without_nesting "$RUNNER_DST" "$BACKUP_ROOT/runner" \
    "live runner backup"; then
    arm_interrupt_traps
    exit 1
  fi
  arm_interrupt_traps
fi
if path_is_regular_file "$PLIST_DST"; then
  trap '' HUP INT TERM
  HAD_PLIST=1
  if ! move_without_nesting "$PLIST_DST" "$BACKUP_ROOT/plist" \
    "live plist backup"; then
    arm_interrupt_traps
    exit 1
  fi
  arm_interrupt_traps
fi
if path_is_regular_file "$CURRENT_VENV_FILE"; then
  trap '' HUP INT TERM
  HAD_VENV_POINTER=1
  if ! move_without_nesting "$CURRENT_VENV_FILE" "$BACKUP_ROOT/current-venv" \
    "live venv-pointer backup"; then
    arm_interrupt_traps
    exit 1
  fi
  arm_interrupt_traps
fi
trap '' HUP INT TERM
if ! move_without_nesting "$STAGE_CACHE" "$CACHE_DIR" "candidate cache"; then
  arm_interrupt_traps
  exit 1
fi
NEW_CACHE_INSTALLED=1
STAGE_CACHE=
arm_interrupt_traps
trap '' HUP INT TERM
if ! move_without_nesting "$RUNNER_STAGE" "$RUNNER_DST" "candidate runner"; then
  arm_interrupt_traps
  exit 1
fi
NEW_RUNNER_INSTALLED=1
RUNNER_STAGE=
arm_interrupt_traps
trap '' HUP INT TERM
if ! move_without_nesting "$PLIST_STAGE" "$PLIST_DST" "candidate plist"; then
  arm_interrupt_traps
  exit 1
fi
NEW_PLIST_INSTALLED=1
PLIST_STAGE=
arm_interrupt_traps

echo "== start and verify the switched loopback-only offline service =="
if ! launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"; then
  echo "failed to bootstrap the replacement MLX-Audio service" >&2
  exit 1
fi
if ! wait_for_health "$LIVE_PORT" 60 "MLX-Audio"; then
  tail -n 80 "$LOG_DIR/mlx-audio.log" >&2 || true
  exit 1
fi
if ! LIVE_PID=$(service_pid); then
  echo "replacement MLX-Audio service has no exact launchd pid before acceptance" >&2
  exit 1
fi
if ! require_same_live_pid "$LIVE_PID" "before acceptance"; then
  exit 1
fi
curl --noproxy '*' --fail --silent --show-error --max-time 600 --request POST --get \
  --data-urlencode "model_name=$STT_MODEL" \
  "http://127.0.0.1:$LIVE_PORT/v1/models" >/dev/null
curl --noproxy '*' --fail --silent --show-error --max-time 600 --request POST --get \
  --data-urlencode "model_name=$TTS_MODEL" \
  "http://127.0.0.1:$LIVE_PORT/v1/models" >/dev/null

if ! require_same_live_pid "$LIVE_PID" "after acceptance"; then
  exit 1
fi

CURRENT_VENV_STAGE=$(mktemp "$STATE_DIR/.current-venv.new.XXXXXX")
printf '%s\n' "$STAGE_VENV" >"$CURRENT_VENV_STAGE"
chmod 600 "$CURRENT_VENV_STAGE"
trap '' HUP INT TERM
if ! move_without_nesting "$CURRENT_VENV_STAGE" "$CURRENT_VENV_FILE" \
  "candidate venv pointer"; then
  arm_interrupt_traps
  exit 1
fi
NEW_VENV_POINTER_INSTALLED=1
CURRENT_VENV_STAGE=
KEEP_STAGE_VENV=1
SWITCH_STARTED=0
arm_interrupt_traps
echo "MLX-Audio ready: pid $LIVE_PID, http://127.0.0.1:$LIVE_PORT"
echo "log: $LOG_DIR/mlx-audio.log"
if [ "$HAD_CACHE" -eq 1 ] || [ "$HAD_RUNNER" -eq 1 ] || \
  [ "$HAD_PLIST" -eq 1 ] || [ "$HAD_VENV_POINTER" -eq 1 ]; then
  echo "previous healthy installation retained for rollback: $BACKUP_ROOT"
  if [ "$PREVIOUS_VENV_VALID" -eq 1 ]; then
    echo "previous environment retained for rollback: $PREVIOUS_VENV"
  fi
fi
