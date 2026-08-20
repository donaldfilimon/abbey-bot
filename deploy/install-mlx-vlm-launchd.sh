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
LOCK_PID_FILE="$INSTALL_LOCK/pid"
UID_NUM=$(id -u)
MODEL=mlx-community/gemma-4-12B-it-4bit
MODEL_REVISION=73bcf09092aa277861d5a191b989b666f7f32e8f
MODEL_DIR="$CACHE_DIR/hub/models--mlx-community--gemma-4-12B-it-4bit/snapshots/$MODEL_REVISION"
LIVE_PORT=8282
MAX_KV_SIZE=8192

VENV_STAGE=
RUNNER_STAGE=
PLIST_STAGE=
SMOKE_PID=
LOCK_HELD=0
TRANSITION_ARMED=0
ROLLBACK_IN_PROGRESS=0
RECOVERY_RETAINED=0
HAD_VENV=0
HAD_RUNNER=0
HAD_PLIST=0
OLD_SERVICE=0
STAMP=
VENV_BACKUP=
RUNNER_BACKUP=
PLIST_BACKUP=
VENV_FAILED=
RUNNER_FAILED=
PLIST_FAILED=
UNINSTALL_RUNNER_BACKUP=
UNINSTALL_PLIST_BACKUP=

restore_interrupt_traps() {
  trap 'echo "MLX-VLM installer interrupted by SIGHUP" >&2; exit 1' HUP
  trap 'echo "MLX-VLM installer interrupted by SIGINT" >&2; exit 1' INT
  trap 'echo "MLX-VLM installer interrupted by SIGTERM" >&2; exit 1' TERM
}

release_install_lock() {
  if [ "$LOCK_HELD" -ne 1 ]; then
    return 0
  fi
  lock_owner=$(sed -n '1p' "$LOCK_PID_FILE" 2>/dev/null || true)
  if [ "$lock_owner" != "$$" ]; then
    echo "refusing to release MLX-VLM install lock owned by ${lock_owner:-unknown}" >&2
    return 1
  fi
  if ! rm "$LOCK_PID_FILE"; then
    echo "failed to remove MLX-VLM install-lock PID record: $LOCK_PID_FILE" >&2
    return 1
  fi
  if ! rmdir "$INSTALL_LOCK"; then
    # Keep the owner record if an unexpected entry prevents releasing the lock.
    printf '%s\n' "$$" >"$LOCK_PID_FILE" 2>/dev/null || true
    echo "failed to release MLX-VLM install lock: $INSTALL_LOCK" >&2
    return 1
  fi
  LOCK_HELD=0
}

acquire_install_lock() {
  # Do not allow a signal to land after mkdir succeeds but before ownership is
  # durably published in both the PID file and shell state.
  trap '' HUP INT TERM
  if ! mkdir "$INSTALL_LOCK" 2>/dev/null; then
    lock_owner=$(sed -n '1p' "$LOCK_PID_FILE" 2>/dev/null || true)
    restore_interrupt_traps
    echo "another MLX-VLM installation is already running:" >&2
    echo "  $INSTALL_LOCK (pid ${lock_owner:-unknown})" >&2
    return 1
  fi
  if ! printf '%s\n' "$$" >"$LOCK_PID_FILE"; then
    rm -f "$LOCK_PID_FILE" 2>/dev/null || true
    rmdir "$INSTALL_LOCK" 2>/dev/null || true
    restore_interrupt_traps
    echo "failed to record the MLX-VLM install-lock owner" >&2
    return 1
  fi
  LOCK_HELD=1
  restore_interrupt_traps
}

cleanup() {
  exit_status=$?
  trap - EXIT
  trap '' HUP INT TERM
  if [ "$TRANSITION_ARMED" -eq 1 ]; then
    if [ "$ROLLBACK_IN_PROGRESS" -eq 0 ] && [ "$RECOVERY_RETAINED" -eq 0 ]; then
      if ! rollback; then
        RECOVERY_RETAINED=1
        exit_status=1
      fi
    fi
  fi
  if [ -n "$SMOKE_PID" ]; then
    kill "$SMOKE_PID" 2>/dev/null || true
    wait "$SMOKE_PID" 2>/dev/null || true
    SMOKE_PID=
  fi
  if [ "$RECOVERY_RETAINED" -eq 1 ]; then
    echo "MLX-VLM recovery is incomplete; the install lock and all recovery" >&2
    echo "artifacts were retained" >&2
    echo "manual recovery lock: $INSTALL_LOCK (pid $$)" >&2
    [ -z "$VENV_BACKUP" ] || echo "environment backup: $VENV_BACKUP" >&2
    [ -z "$RUNNER_BACKUP" ] || echo "runner backup: $RUNNER_BACKUP" >&2
    [ -z "$PLIST_BACKUP" ] || echo "plist backup: $PLIST_BACKUP" >&2
    [ -z "$VENV_FAILED" ] || echo "candidate environment: $VENV_FAILED" >&2
    [ -z "$RUNNER_FAILED" ] || echo "candidate runner: $RUNNER_FAILED" >&2
    [ -z "$PLIST_FAILED" ] || echo "candidate plist: $PLIST_FAILED" >&2
    exit 1
  fi
  if [ -n "$RUNNER_STAGE" ]; then
    if ! rm -f "$RUNNER_STAGE"; then
      exit_status=1
    fi
  fi
  if [ -n "$PLIST_STAGE" ]; then
    if ! rm -f "$PLIST_STAGE"; then
      exit_status=1
    fi
  fi
  if [ -n "$VENV_STAGE" ] && [ -d "$VENV_STAGE" ]; then
    echo "staged environment retained for inspection: $VENV_STAGE" >&2
  fi
  if ! release_install_lock; then
    exit_status=1
  fi
  exit "$exit_status"
}
trap cleanup EXIT
restore_interrupt_traps

service_pid() {
  SERVICE_OUTPUT=$(launchctl print "gui/$UID_NUM/$LABEL" 2>/dev/null) || return 1
  SERVICE_PID=$(printf '%s\n' "$SERVICE_OUTPUT" | /usr/bin/sed -n \
    's/^[[:space:]]*pid = \([0-9][0-9]*\)$/\1/p')
  case "$SERVICE_PID" in
    ''|*[!0-9]*) return 1 ;;
    *) printf '%s\n' "$SERVICE_PID" ;;
  esac
}

wait_for_service_pid() {
  pid_attempts=0
  while [ "$pid_attempts" -lt 30 ]; do
    observed_service_pid=$(service_pid || true)
    if [ -n "$observed_service_pid" ]; then
      printf '%s\n' "$observed_service_pid"
      return 0
    fi
    pid_attempts=$((pid_attempts + 1))
    sleep 1
  done
  echo "MLX-VLM launchd service did not acquire a PID within 30 seconds" >&2
  return 1
}

require_pid_owns_loopback_listener() {
  listener_pid=$1
  listener_port=$2
  listener_description=$3
  case "$listener_pid" in
    ''|*[!0-9]*)
      echo "$listener_description has an invalid pid or port" >&2
      return 1
      ;;
  esac
  case "$listener_port" in
    ''|*[!0-9]*)
      echo "$listener_description has an invalid pid or port" >&2
      return 1
      ;;
  esac
  if [ "$listener_pid" -le 0 ] || [ "$listener_port" -le 0 ] || \
    [ "$listener_port" -gt 65535 ]; then
    echo "$listener_description has an invalid pid or port" >&2
    return 1
  fi
  if ! kill -0 "$listener_pid" 2>/dev/null; then
    echo "$listener_description pid $listener_pid is not alive" >&2
    return 1
  fi
  if ! listener_output=$(/usr/sbin/lsof -nP -a -p "$listener_pid" \
    -iTCP@127.0.0.1:"$listener_port" -sTCP:LISTEN -Fpn 2>/dev/null); then
    echo "$listener_description pid $listener_pid does not own the loopback" >&2
    echo "LISTEN port $listener_port" >&2
    return 1
  fi
  listener_pid_seen=0
  listener_name_seen=0
  while IFS= read -r listener_field || [ -n "$listener_field" ]; do
    case "$listener_field" in
      "p$listener_pid") listener_pid_seen=1 ;;
      "n127.0.0.1:$listener_port") listener_name_seen=1 ;;
    esac
  done <<EOF
$listener_output
EOF
  if [ "$listener_pid_seen" -ne 1 ] || [ "$listener_name_seen" -ne 1 ]; then
    echo "$listener_description did not produce an exact loopback listener" >&2
    echo "record for pid $listener_pid" >&2
    return 1
  fi
  return 0
}

wait_for_same_live_listener() {
  expected_pid=$1
  listener_port=$2
  listener_attempts=$3
  listener_description=$4
  if [ ! -x /usr/sbin/lsof ]; then
    echo "/usr/sbin/lsof is required to verify $listener_description" >&2
    return 1
  fi
  attempt=0
  while [ "$attempt" -lt "$listener_attempts" ]; do
    observed_pid=$(service_pid || true)
    if [ "$observed_pid" != "$expected_pid" ]; then
      echo "$listener_description changed launchd pid from $expected_pid" >&2
      echo "to ${observed_pid:-none} before listener acceptance" >&2
      return 1
    fi
    if ! kill -0 "$expected_pid" 2>/dev/null; then
      echo "$listener_description pid $expected_pid exited before listener acceptance" >&2
      return 1
    fi
    if require_pid_owns_loopback_listener "$expected_pid" "$listener_port" \
      "$listener_description" 2>/dev/null; then
      return 0
    fi
    attempt=$((attempt + 1))
    sleep 1
  done
  require_pid_owns_loopback_listener "$expected_pid" "$listener_port" \
    "$listener_description"
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

report_smoke_status() {
  checkpoint=$1
  log_file=$2
  if [ -n "$SMOKE_PID" ] && kill -0 "$SMOKE_PID" 2>/dev/null; then
    echo "staged MLX-VLM pid $SMOKE_PID was still running at $checkpoint" >&2
  elif [ -n "$SMOKE_PID" ]; then
    if wait "$SMOKE_PID"; then
      smoke_status=0
    else
      smoke_status=$?
    fi
    echo "staged MLX-VLM pid $SMOKE_PID exited with status $smoke_status at $checkpoint" >&2
    SMOKE_PID=
  else
    echo "staged MLX-VLM had no recorded pid at $checkpoint" >&2
  fi
  tail -n 120 "$log_file" >&2 || true
}

require_smoke_alive() {
  checkpoint=$1
  log_file=$2
  if [ -n "$SMOKE_PID" ] && kill -0 "$SMOKE_PID" 2>/dev/null; then
    return 0
  fi
  report_smoke_status "$checkpoint" "$log_file"
  return 1
}

wait_for_health() {
  base_url=$1
  log_file=$2
  deadline=$(($(date +%s) + 900))
  until curl --noproxy '*' --fail --silent --show-error --max-time 2 \
    "$base_url/health" >/dev/null 2>&1; do
    if [ -n "$SMOKE_PID" ] && ! kill -0 "$SMOKE_PID" 2>/dev/null; then
      report_smoke_status "while waiting for health" "$log_file"
      return 1
    fi
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
    --max-kv-size "$MAX_KV_SIZE" \
    --max-num-seqs 1 \
    --vision-cache-size 4 \
    --log-level INFO >"$log_file" 2>&1 &
  SMOKE_PID=$!
}

rollback_move() {
  rollback_source=$1
  rollback_destination=$2
  rollback_description=$3
  if ! mv "$rollback_source" "$rollback_destination"; then
    echo "failed to move $rollback_description: $rollback_source -> $rollback_destination" >&2
    RECOVERY_RETAINED=1
    return 1
  fi
}

# A forward transaction move must leave recovery eligible on failure. The
# EXIT handler decides whether anything already moved needs restoration;
# marking recovery retained here would skip that automatic rollback entirely.
transition_move() {
  transition_source=$1
  transition_destination=$2
  transition_description=$3
  if ! mv "$transition_source" "$transition_destination"; then
    echo "failed to move $transition_description:" >&2
    echo "  $transition_source -> $transition_destination" >&2
    return 1
  fi
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
  return 0
}

rollback_restore() {
  restore_source=$1
  restore_destination=$2
  restore_description=$3
  if ! require_path_absent "$restore_destination" "$restore_description destination"; then
    RECOVERY_RETAINED=1
    return 1
  fi
  rollback_move "$restore_source" "$restore_destination" "$restore_description" || return 1
}

# Uninstall runs before the install rollback definition below is evaluated, so
# this definition restores a partially moved launch-file pair on interruption.
rollback() {
  ROLLBACK_IN_PROGRESS=1
  trap '' HUP INT TERM
  if ! stop_service; then
    echo "could not stop MLX-VLM while restoring an interrupted uninstall" >&2
    RECOVERY_RETAINED=1
    return 1
  fi
  if [ -n "$UNINSTALL_RUNNER_BACKUP" ]; then
    if path_is_present "$UNINSTALL_RUNNER_BACKUP"; then
      rollback_restore "$UNINSTALL_RUNNER_BACKUP" "$RUNNER_DST" \
        "uninstalled runner" || return 1
    elif ! path_is_regular_file "$RUNNER_DST"; then
      echo "interrupted uninstall lost both the live runner and its backup" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    UNINSTALL_RUNNER_BACKUP=
  fi
  if [ -n "$UNINSTALL_PLIST_BACKUP" ]; then
    if path_is_present "$UNINSTALL_PLIST_BACKUP"; then
      rollback_restore "$UNINSTALL_PLIST_BACKUP" "$PLIST_DST" \
        "uninstalled plist" || return 1
    elif ! path_is_regular_file "$PLIST_DST"; then
      echo "interrupted uninstall lost both the live plist and its backup" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    UNINSTALL_PLIST_BACKUP=
  fi
  if [ "$OLD_SERVICE" -eq 1 ]; then
    if ! launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"; then
      echo "interrupted uninstall restored files, but not the prior healthy service" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! uninstall_restore_pid=$(wait_for_service_pid); then
      echo "interrupted uninstall restored files, but launchd did not start the prior service" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! wait_for_same_live_listener "$uninstall_restore_pid" "$LIVE_PORT" 900 \
      "prior MLX-VLM before restored uninstall health"; then
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! wait_for_health "http://127.0.0.1:$LIVE_PORT" "$LOG_DIR/mlx-vlm.log"; then
      echo "interrupted uninstall restored files, but the prior service was unhealthy" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    uninstall_restored_pid=$(service_pid || true)
    if [ "$uninstall_restored_pid" != "$uninstall_restore_pid" ] || \
      ! require_pid_owns_loopback_listener "$uninstall_restore_pid" "$LIVE_PORT" \
        "prior MLX-VLM restored after interrupted uninstall"; then
      echo "interrupted uninstall health was not served by one stable prior launchd process" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
  fi
  ROLLBACK_IN_PROGRESS=0
  TRANSITION_ARMED=0
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

if [ "$MODE" = install ]; then
  if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
    echo "Abbey's MLX-VLM sidecar requires Apple Silicon macOS" >&2
    exit 1
  fi
  UV_BIN=$(command -v uv || true)
  if [ -z "$UV_BIN" ]; then
    echo "uv is required to install the pinned Python environment" >&2
    exit 1
  fi
  if [ ! -x /usr/sbin/lsof ]; then
    echo "/usr/sbin/lsof is required to bind acceptance to one server process" >&2
    exit 1
  fi
  for required in "$PLIST_SRC" "$RUNNER_SRC" "$SMOKE_SRC" "$REQUIREMENTS"; do
    if [ ! -f "$required" ]; then
      echo "missing required deployment file: $required" >&2
      exit 1
    fi
  done
fi

echo "== create owner-only layout and take install lock =="
mkdir -p "$STATE_DIR"
chmod 700 "$STATE_DIR"
if [ "$MODE" = install ]; then
  mkdir -p "$INSTALL_DIR" "$CACHE_DIR" "$LOG_DIR" "$HOME/Library/LaunchAgents"
  chmod 700 "$INSTALL_DIR" "$CACHE_DIR" "$LOG_DIR"
fi
acquire_install_lock

if [ "$MODE" = uninstall ]; then
  if launchctl print "gui/$UID_NUM/$LABEL" >/dev/null 2>&1; then
    OLD_SERVICE=1
  fi
  if path_is_present "$RUNNER_DST" && ! path_is_regular_file "$RUNNER_DST"; then
    echo "MLX-VLM runner has an unexpected file type: $RUNNER_DST" >&2
    exit 1
  fi
  if path_is_present "$PLIST_DST" && ! path_is_regular_file "$PLIST_DST"; then
    echo "MLX-VLM plist has an unexpected file type: $PLIST_DST" >&2
    exit 1
  fi
  uninstall_stamp=$(date -u +%Y%m%dT%H%M%SZ).$$
  uninstall_runner_path=
  if path_is_regular_file "$RUNNER_DST"; then
    uninstall_runner_path="$RUNNER_DST.uninstalled.$uninstall_stamp"
    require_path_absent "$uninstall_runner_path" "uninstalled runner path"
  fi
  uninstall_plist_path=
  if path_is_regular_file "$PLIST_DST"; then
    uninstall_plist_path="$PLIST_DST.uninstalled.$uninstall_stamp"
    require_path_absent "$uninstall_plist_path" "uninstalled plist path"
  fi
  TRANSITION_ARMED=1
  if ! stop_service; then
    echo "MLX-VLM service could not be stopped; launch files were left untouched" >&2
    RECOVERY_RETAINED=1
    exit 1
  fi
  if [ -n "$uninstall_runner_path" ]; then
    UNINSTALL_RUNNER_BACKUP=$uninstall_runner_path
    if ! transition_move "$RUNNER_DST" "$UNINSTALL_RUNNER_BACKUP" \
      "runner for uninstall"; then
      exit 1
    fi
  fi
  if [ -n "$uninstall_plist_path" ]; then
    UNINSTALL_PLIST_BACKUP=$uninstall_plist_path
    if ! transition_move "$PLIST_DST" "$UNINSTALL_PLIST_BACKUP" \
      "plist for uninstall"; then
      exit 1
    fi
  fi
  TRANSITION_ARMED=0
  echo "unloaded and removed MLX-VLM launch files; model cache and venv were retained"
  [ -z "$UNINSTALL_RUNNER_BACKUP" ] || \
    echo "previous runner retained: $UNINSTALL_RUNNER_BACKUP"
  [ -z "$UNINSTALL_PLIST_BACKUP" ] || \
    echo "previous plist retained: $UNINSTALL_PLIST_BACKUP"
  exit 0
fi

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
if ! wait_for_health "http://127.0.0.1:$SMOKE_PORT" "$SMOKE_LOG"; then
  exit 1
fi
if ! require_smoke_alive "immediately after health" "$SMOKE_LOG"; then
  exit 1
fi
STAGED_ACCEPT_PID=$SMOKE_PID
if ! require_pid_owns_loopback_listener "$STAGED_ACCEPT_PID" "$SMOKE_PORT" \
  "staged MLX-VLM before acceptance"; then
  report_smoke_status "before acceptance listener check" "$SMOKE_LOG"
  exit 1
fi
if ! "$VENV_STAGE/bin/python" "$SMOKE_SRC" \
  --base-url "http://127.0.0.1:$SMOKE_PORT" \
  --model "$MODEL_DIR" \
  --expected-kv-size "$MAX_KV_SIZE" \
  --timeout 600; then
  echo "staged MLX-VLM acceptance failed" >&2
  report_smoke_status "after failed acceptance" "$SMOKE_LOG"
  exit 1
fi
if ! require_smoke_alive "immediately after acceptance" "$SMOKE_LOG"; then
  exit 1
fi
if [ "$SMOKE_PID" != "$STAGED_ACCEPT_PID" ]; then
  echo "staged MLX-VLM process identity changed during acceptance" >&2
  report_smoke_status "after acceptance identity check" "$SMOKE_LOG"
  exit 1
fi
if ! require_pid_owns_loopback_listener "$STAGED_ACCEPT_PID" "$SMOKE_PORT" \
  "staged MLX-VLM after acceptance"; then
  report_smoke_status "after acceptance listener check" "$SMOKE_LOG"
  exit 1
fi
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
if launchctl print "gui/$UID_NUM/$LABEL" >/dev/null 2>&1; then
  OLD_SERVICE=1
fi

rollback() {
  ROLLBACK_IN_PROGRESS=1
  # Once recovery begins, finish it or retain every artifact and the lock for
  # manual repair. A second signal must not interrupt a filesystem transition.
  trap '' HUP INT TERM
  echo "MLX-VLM replacement failed; restoring the previous installation" >&2
  if ! stop_service; then
    echo "replacement could not be unloaded; no recovery files were mutated" >&2
    RECOVERY_RETAINED=1
    return 1
  fi

  if [ -n "$VENV_BACKUP" ] && path_is_directory "$VENV_BACKUP"; then
    if path_is_directory "$VENV_DIR"; then
      next_failed="$VENV_DIR.failed.$STAMP.$$"
      if ! require_path_absent "$next_failed" "candidate-environment recovery path"; then
        RECOVERY_RETAINED=1
        return 1
      fi
      rollback_move "$VENV_DIR" "$next_failed" "candidate environment" || return 1
      VENV_FAILED=$next_failed
    elif path_is_present "$VENV_DIR"; then
      echo "candidate environment path has an unexpected file type: $VENV_DIR" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    rollback_restore "$VENV_BACKUP" "$VENV_DIR" "environment backup" || return 1
  elif [ "$HAD_VENV" -eq 0 ] && path_is_directory "$VENV_DIR"; then
    next_failed="$VENV_DIR.failed.$STAMP.$$"
    if ! require_path_absent "$next_failed" "candidate-environment recovery path"; then
      RECOVERY_RETAINED=1
      return 1
    fi
    rollback_move "$VENV_DIR" "$next_failed" "candidate environment" || return 1
    VENV_FAILED=$next_failed
  elif [ "$HAD_VENV" -eq 0 ] && path_is_present "$VENV_DIR"; then
    echo "candidate environment path has an unexpected file type: $VENV_DIR" >&2
    RECOVERY_RETAINED=1
    return 1
  elif [ "$HAD_VENV" -eq 1 ] && ! path_is_directory "$VENV_DIR"; then
    echo "previous environment is missing and no restorable backup exists" >&2
    RECOVERY_RETAINED=1
    return 1
  fi

  if [ -n "$RUNNER_BACKUP" ] && path_is_regular_file "$RUNNER_BACKUP"; then
    if path_is_regular_file "$RUNNER_DST"; then
      next_failed="$RUNNER_DST.failed.$STAMP.$$"
      if ! require_path_absent "$next_failed" "candidate-runner recovery path"; then
        RECOVERY_RETAINED=1
        return 1
      fi
      rollback_move "$RUNNER_DST" "$next_failed" "candidate runner" || return 1
      RUNNER_FAILED=$next_failed
    elif path_is_present "$RUNNER_DST"; then
      echo "candidate runner path has an unexpected file type: $RUNNER_DST" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    rollback_restore "$RUNNER_BACKUP" "$RUNNER_DST" "runner backup" || return 1
  elif [ "$HAD_RUNNER" -eq 0 ] && path_is_regular_file "$RUNNER_DST"; then
    next_failed="$RUNNER_DST.failed.$STAMP.$$"
    if ! require_path_absent "$next_failed" "candidate-runner recovery path"; then
      RECOVERY_RETAINED=1
      return 1
    fi
    rollback_move "$RUNNER_DST" "$next_failed" "candidate runner" || return 1
    RUNNER_FAILED=$next_failed
  elif [ "$HAD_RUNNER" -eq 0 ] && path_is_present "$RUNNER_DST"; then
    echo "candidate runner path has an unexpected file type: $RUNNER_DST" >&2
    RECOVERY_RETAINED=1
    return 1
  elif [ "$HAD_RUNNER" -eq 1 ] && ! path_is_regular_file "$RUNNER_DST"; then
    echo "previous runner is missing and no restorable backup exists" >&2
    RECOVERY_RETAINED=1
    return 1
  fi

  if [ -n "$PLIST_BACKUP" ] && path_is_regular_file "$PLIST_BACKUP"; then
    if path_is_regular_file "$PLIST_DST"; then
      next_failed="$PLIST_DST.failed.$STAMP.$$"
      if ! require_path_absent "$next_failed" "candidate-plist recovery path"; then
        RECOVERY_RETAINED=1
        return 1
      fi
      rollback_move "$PLIST_DST" "$next_failed" "candidate plist" || return 1
      PLIST_FAILED=$next_failed
    elif path_is_present "$PLIST_DST"; then
      echo "candidate plist path has an unexpected file type: $PLIST_DST" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    rollback_restore "$PLIST_BACKUP" "$PLIST_DST" "plist backup" || return 1
  elif [ "$HAD_PLIST" -eq 0 ] && path_is_regular_file "$PLIST_DST"; then
    next_failed="$PLIST_DST.failed.$STAMP.$$"
    if ! require_path_absent "$next_failed" "candidate-plist recovery path"; then
      RECOVERY_RETAINED=1
      return 1
    fi
    rollback_move "$PLIST_DST" "$next_failed" "candidate plist" || return 1
    PLIST_FAILED=$next_failed
  elif [ "$HAD_PLIST" -eq 0 ] && path_is_present "$PLIST_DST"; then
    echo "candidate plist path has an unexpected file type: $PLIST_DST" >&2
    RECOVERY_RETAINED=1
    return 1
  elif [ "$HAD_PLIST" -eq 1 ] && ! path_is_regular_file "$PLIST_DST"; then
    echo "previous plist is missing and no restorable backup exists" >&2
    RECOVERY_RETAINED=1
    return 1
  fi

  if [ "$OLD_SERVICE" -eq 1 ]; then
    if ! path_is_regular_file "$PLIST_DST"; then
      echo "previous service was loaded, but its restored plist is missing" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"; then
      echo "previous files were restored, but launchd bootstrap failed" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! rollback_restore_pid=$(wait_for_service_pid); then
      echo "previous files were restored, but launchd did not start the prior service" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! wait_for_same_live_listener "$rollback_restore_pid" "$LIVE_PORT" 900 \
      "prior MLX-VLM before replacement-rollback health"; then
      RECOVERY_RETAINED=1
      return 1
    fi
    if ! wait_for_health "http://127.0.0.1:$LIVE_PORT" "$LOG_DIR/mlx-vlm.log"; then
      echo "previous files were restored, but the restored service is unhealthy" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
    rollback_restored_pid=$(service_pid || true)
    if [ "$rollback_restored_pid" != "$rollback_restore_pid" ] || \
      ! require_pid_owns_loopback_listener "$rollback_restore_pid" "$LIVE_PORT" \
        "prior MLX-VLM after replacement rollback"; then
      echo "restored health was not served by one stable prior launchd process" >&2
      RECOVERY_RETAINED=1
      return 1
    fi
  fi
  ROLLBACK_IN_PROGRESS=0
  TRANSITION_ARMED=0
  echo "previous MLX-VLM installation restored successfully" >&2
}

echo "== publish staged environment with rollback =="
if path_is_present "$VENV_DIR" && ! path_is_directory "$VENV_DIR"; then
  echo "live MLX-VLM environment path has an unexpected file type: $VENV_DIR" >&2
  exit 1
fi
if path_is_present "$RUNNER_DST" && ! path_is_regular_file "$RUNNER_DST"; then
  echo "live MLX-VLM runner path has an unexpected file type: $RUNNER_DST" >&2
  exit 1
fi
if path_is_present "$PLIST_DST" && ! path_is_regular_file "$PLIST_DST"; then
  echo "live MLX-VLM plist path has an unexpected file type: $PLIST_DST" >&2
  exit 1
fi
if path_is_directory "$VENV_DIR"; then
  HAD_VENV=1
fi
if path_is_regular_file "$RUNNER_DST"; then
  HAD_RUNNER=1
fi
if path_is_regular_file "$PLIST_DST"; then
  HAD_PLIST=1
fi
TRANSITION_ARMED=1
if ! stop_service; then
  echo "MLX-VLM service could not be stopped; publish files were left untouched" >&2
  RECOVERY_RETAINED=1
  exit 1
fi
if path_is_directory "$VENV_DIR"; then
  next_backup="$VENV_DIR.backup.$STAMP.$$"
  if ! require_path_absent "$next_backup" "environment backup path"; then
    exit 1
  fi
  VENV_BACKUP=$next_backup
  if ! mv "$VENV_DIR" "$VENV_BACKUP"; then
    echo "failed to back up the live MLX-VLM environment" >&2
    exit 1
  fi
fi
if path_is_regular_file "$RUNNER_DST"; then
  next_backup="$RUNNER_DST.backup.$STAMP.$$"
  if ! require_path_absent "$next_backup" "runner backup path"; then
    exit 1
  fi
  RUNNER_BACKUP=$next_backup
  if ! mv "$RUNNER_DST" "$RUNNER_BACKUP"; then
    echo "failed to back up the live MLX-VLM runner" >&2
    exit 1
  fi
fi
if path_is_regular_file "$PLIST_DST"; then
  next_backup="$PLIST_DST.backup.$STAMP.$$"
  if ! require_path_absent "$next_backup" "plist backup path"; then
    exit 1
  fi
  PLIST_BACKUP=$next_backup
  if ! mv "$PLIST_DST" "$PLIST_BACKUP"; then
    echo "failed to back up the live MLX-VLM plist" >&2
    exit 1
  fi
fi
if ! require_path_absent "$VENV_DIR" "environment publish path"; then
  exit 1
fi
if ! mv "$VENV_STAGE" "$VENV_DIR"; then
  echo "failed to publish the staged MLX-VLM environment" >&2
  exit 1
fi
VENV_STAGE=
if ! require_path_absent "$RUNNER_DST" "runner publish path"; then
  exit 1
fi
if ! mv "$RUNNER_STAGE" "$RUNNER_DST"; then
  echo "failed to publish the staged MLX-VLM runner" >&2
  exit 1
fi
RUNNER_STAGE=
if ! require_path_absent "$PLIST_DST" "plist publish path"; then
  exit 1
fi
if ! mv "$PLIST_STAGE" "$PLIST_DST"; then
  echo "failed to publish the staged MLX-VLM plist" >&2
  exit 1
fi
PLIST_STAGE=

if ! launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"; then
  echo "failed to bootstrap the replacement MLX-VLM service" >&2
  exit 1
fi
if ! wait_for_health "http://127.0.0.1:$LIVE_PORT" "$LOG_DIR/mlx-vlm.log"; then
  exit 1
fi
if ! LIVE_ACCEPT_PID=$(service_pid); then
  echo "replacement MLX-VLM service has no exact launchd pid before acceptance" >&2
  exit 1
fi
if ! require_pid_owns_loopback_listener "$LIVE_ACCEPT_PID" "$LIVE_PORT" \
  "replacement MLX-VLM before acceptance"; then
  exit 1
fi
if ! "$VENV_DIR/bin/python" "$SMOKE_SRC" \
  --base-url "http://127.0.0.1:$LIVE_PORT" \
  --model "$MODEL_DIR" \
  --expected-kv-size "$MAX_KV_SIZE" \
  --timeout 600; then
  echo "replacement MLX-VLM acceptance failed" >&2
  exit 1
fi
if ! LIVE_ACCEPTED_PID=$(service_pid); then
  echo "replacement MLX-VLM service has no exact launchd pid after acceptance" >&2
  exit 1
fi
if [ "$LIVE_ACCEPTED_PID" != "$LIVE_ACCEPT_PID" ]; then
  echo "replacement MLX-VLM restarted during acceptance" >&2
  echo "  $LIVE_ACCEPT_PID -> $LIVE_ACCEPTED_PID" >&2
  exit 1
fi
if ! require_pid_owns_loopback_listener "$LIVE_ACCEPT_PID" "$LIVE_PORT" \
  "replacement MLX-VLM after acceptance"; then
  exit 1
fi
PID=$LIVE_ACCEPT_PID
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
