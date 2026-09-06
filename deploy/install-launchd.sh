#!/bin/sh
# Shell owns the transaction/traps; the reviewed sibling Python bundle validates
# fixed private paths and executes bounded phases. No owner environment is sourced.
set -eu
set +x
umask 077
cd "$(dirname "$0")/.."
MODE=install
case "$#:${1:-}" in
  0:) ;;
  1:--uninstall) MODE=uninstall ;;
  *) echo 'usage: install-launchd.sh [--uninstall]' >&2; exit 2 ;;
esac
STATE=
LOCKED=0
ROLLBACK=0
HAD_PRIOR=0
RETAINED=0
PHASE_ACTIVE=0
INTERRUPTED=0

on_interrupt() {
  INTERRUPTED=1
  if [ "$PHASE_ACTIVE" -eq 0 ]; then exit 1; fi
}

phase() {
  printf '%s\n' "$STATE" | python3 -I deploy/service_transaction.py "$1"
}
run_phase() {
  if [ "$INTERRUPTED" -eq 1 ]; then return 1; fi
  PHASE_ACTIVE=1
  phase_result=0
  next_state=$(phase "$1") || phase_result=$?
  PHASE_ACTIVE=0
  if [ "$phase_result" -eq 79 ]; then
    RETAINED=1
    echo 'installation: cleanup_incomplete' >&2
  fi
  if [ "$phase_result" -ne 0 ]; then return 1; fi
  STATE=$next_state
  if [ "$INTERRUPTED" -eq 1 ]; then return 1; fi
}
cleanup() {
  result=$?
  trap - EXIT
  trap '' HUP INT TERM
  set +e
  INTERRUPTED=0
  if [ "$RETAINED" -eq 1 ]; then
    echo 'installation: recovery_retained' >&2
    exit 1
  fi
  if [ "$ROLLBACK" -eq 1 ]; then
    if run_phase capture && run_phase stop && run_phase restore; then
      if [ "$HAD_PRIOR" -eq 1 ]; then
        if run_phase start; then
          echo 'installation: rollback_ready' >&2
          ROLLBACK=0
        fi
      else
        ROLLBACK=0
      fi
    fi
    if [ "$ROLLBACK" -eq 1 ]; then
      RETAINED=1
      echo 'installation: recovery_retained' >&2
    fi
  fi
  if [ "$LOCKED" -eq 1 ] && [ "$RETAINED" -eq 0 ]; then
    if ! run_phase release; then result=1; fi
  fi
  exit "$result"
}
trap cleanup EXIT
# The acquisition and transfer of private lock ownership are one signal-masked step.
trap '' HUP INT TERM
STATE=$(python3 -I deploy/service_transaction.py acquire) || exit 1
LOCKED=1
trap on_interrupt HUP INT TERM
if [ "$MODE" = uninstall ]; then
  run_phase uninstall_prepare
  run_phase stop
  run_phase uninstall
  echo 'installation: uninstalled'
  exit 0
fi
# Build output may contain arbitrary local paths; transaction diagnostics are fixed.
if ! cargo build --release --locked >/dev/null 2>&1; then
  echo 'installation: build' >&2
  exit 1
fi
run_phase prepare
if ! sh deploy/check-launchd-env.sh "$HOME/.config/abbey-bot/env" >/dev/null 2>&1; then
  echo 'installation: environment' >&2
  exit 1
fi
# Only a fixed boolean leaves the private phase state for shell flow control.
HAD_PRIOR=$(printf '%s\n' "$STATE" | python3 -I -c 'import json,sys; print(int(json.load(sys.stdin)["had_plist"]))')
run_phase capture
# A failed initial stop has not changed installed bytes and must not start rollback.
run_phase stop
ROLLBACK=1
run_phase publish
run_phase start
ROLLBACK=0
echo 'installation: ready'
