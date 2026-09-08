#!/bin/sh
# run-abbey-bot driver: build and drive the abbey-bot release binary through
# its token-free modes, never through a second Discord gateway.
#
# The launchd service com.donaldfilimon.abbey-bot already owns the live token
# on this machine. Every mode here scrubs DISCORD_TOKEN (and the cloud keys)
# from the child environment, sets ABBEY_DATA_DIR and ABBEY_EPISODE_GATE_CONFIG
# to blank so nothing persists or proposes to the WDBX ledger, and runs in the
# foreground with a timeout. The one exception is `plan`, which needs the
# token for a read-only REST diff and refuses `--apply`.
#
# Exit codes are captured directly. Nothing here is piped through `tail`.
#
#   .claude/skills/run-abbey-bot/smoke.sh all            # build args provider voice status
#   .claude/skills/run-abbey-bot/smoke.sh provider all   # includes the Apple FM lanes
#   .claude/skills/run-abbey-bot/smoke.sh test voice_ux::
#   .claude/skills/run-abbey-bot/smoke.sh plan 1275617641620443146 [--stage reveal]
set -eu

REPO=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$REPO/target/release/abbey-bot"
ENV_FILE="$HOME/.config/abbey-bot/env"
STAMP=$(date +%Y%m%d-%H%M%S)
TMP_ROOT=${TMPDIR:-/tmp}
OUT="${RUN_ABBEY_BOT_OUT:-${TMP_ROOT%/}/run-abbey-bot-$STAMP}"
mkdir -p "$OUT"
cd "$REPO"

usage() {
  cat >&2 <<EOF
usage: smoke.sh build | args | provider [primary|all] | voice | status | plan GUILD_ID [--stage S] [--category C] | test FILTER | all
outputs land in $OUT
EOF
  exit 2
}

say() { printf '== %s ==\n' "$*"; }

# Source the owner-only env file without ever printing it, so the loopback
# endpoints (ABBEY_BOT_LLM_ENDPOINT, ABBEY_VOICE_LOCAL_ENDPOINT, ...) resolve.
# Tracing is switched off around the source so `sh -x smoke.sh ...` cannot
# echo DISCORD_TOKEN into a transcript. Callers run this inside a subshell
# so the token never lingers in the driver process itself.
load_env() {
  if [ -r "$ENV_FILE" ]; then
    case $- in *x*) _trace=1 ;; *) _trace=0 ;; esac
    set +x
    set -a
    # shellcheck disable=SC1090
    . "$ENV_FILE"
    set +a
    [ "$_trace" -eq 1 ] && set -x
    unset _trace
  fi
}

# Run the binary with every credential and every persistence path removed.
scrubbed() {
  env -u DISCORD_TOKEN -u DISCORD_BOT_TOKEN -u ANTHROPIC_API_KEY -u OPENAI_API_KEY \
    ABBEY_DATA_DIR= ABBEY_EPISODE_GATE_CONFIG= "$@"
}

need_bin() {
  [ -x "$BIN" ] || { echo "missing $BIN; run: smoke.sh build" >&2; exit 1; }
}

do_build() {
  say build
  cargo build --locked --release > "$OUT/build.log" 2>&1 && rc=0 || rc=$?
  tail -2 "$OUT/build.log"
  [ "$rc" -eq 0 ] || { echo "build failed (exit $rc), see $OUT/build.log" >&2; exit "$rc"; }
  shasum -a 256 "$BIN"
}

# Prove the binary you built is the one you are driving: bad argv exits 2.
do_args() {
  say args
  need_bin
  for argv in "--bogus" "--provider-self-test primary" "--voice-self-test"; do
    # shellcheck disable=SC2086
    "$BIN" $argv > /dev/null 2> "$OUT/args.stderr" && rc=0 || rc=$?
    if [ "$rc" -ne 2 ]; then
      echo "expected exit 2 for '$argv', got $rc" >&2; cat "$OUT/args.stderr" >&2; exit 1
    fi
    printf 'exit 2 ok: abbey-bot %s\n' "$argv"
  done
}

# Qualify the configured reasoning/vision route with synthetic fixtures.
# No Discord, no state. `all` also probes the Apple FM lanes and exits 2 on a
# machine where ABBEY_FM_MODE is off, because fm_cli.text fails there.
do_provider() {
  target=${1:-primary}
  say "provider self-test ($target)"
  need_bin
  start=$(date +%s)
  ( load_env; scrubbed timeout 600 "$BIN" --provider-self-test "$target" --json ) \
    > "$OUT/provider-$target.json" 2> "$OUT/provider-$target.stderr" && rc=0 || rc=$?
  echo "exit $rc in $(( $(date +%s) - start ))s -> $OUT/provider-$target.json"
  [ -s "$OUT/provider-$target.stderr" ] && cat "$OUT/provider-$target.stderr" >&2
  python3 - "$OUT/provider-$target.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
print("overall_pass:", d["overall_pass"], "| binary sha256:", d["primary"]["identity"]["abbey_binary_sha256"][:16])
for lane, body in d.items():
    if isinstance(body, dict) and "capabilities" in body:
        caps = " ".join(f"{k}={v['status']}" for k, v in body["capabilities"].items())
        print(f"  {lane:10} configured={body['configured']!s:5} {caps}")
PY
  return "$rc"
}

# Local TTS -> STT -> Abbey generation -> TTS. Writes a fresh owner-only WAV;
# the mode exits 1 ("refuses to overwrite") on an existing path, so the path
# is timestamped.
do_voice() {
  say "voice self-test"
  need_bin
  wav="$OUT/audition-$STAMP.wav"
  start=$(date +%s)
  ( load_env; scrubbed timeout 600 env ABBEY_VISION_ENDPOINT=off "$BIN" --voice-self-test "$wav" ) \
    > "$OUT/voice.stdout" 2> "$OUT/voice.stderr" && rc=0 || rc=$?
  echo "exit $rc in $(( $(date +%s) - start ))s"
  cat "$OUT/voice.stdout"
  [ -s "$OUT/voice.stderr" ] && cat "$OUT/voice.stderr" >&2
  [ "$rc" -eq 0 ] && file "$wav"
  return "$rc"
}

# Read-only evidence about the LIVE launchd service. Never restarts it.
do_status() {
  say "deployed service (read-only)"
  python3 -I deploy/service-status.py
  sh deploy/check-launchd-env.sh "$ENV_FILE"
  launchctl list | grep -E 'abbey' || true
}

# Dry-run diff of the shipped plan against a live guild over REST. Needs the
# token, so it is the only mode that keeps it. `--apply` is refused here.
do_plan() {
  guild=${1:-}
  [ -n "$guild" ] || usage
  shift
  for a in "$@"; do
    [ "$a" = "--apply" ] && { echo "refusing --apply from the driver; run it by hand per README" >&2; exit 2; }
  done
  say "server plan dry run (guild $guild)"
  need_bin
  ( load_env; env ABBEY_DATA_DIR= ABBEY_EPISODE_GATE_CONFIG= timeout 300 "$BIN" \
      --server-plan blueprints/mlai-community.toml --guild "$guild" "$@" ) \
    > "$OUT/plan.txt" 2> "$OUT/plan.stderr" && rc=0 || rc=$?
  echo "exit $rc -> $OUT/plan.txt"
  cat "$OUT/plan.txt"
  [ -s "$OUT/plan.stderr" ] && cat "$OUT/plan.stderr" >&2
  return "$rc"
}

# Direct invocation for PR work: tests live in the binary. A filter that
# matches nothing still exits 0 ("running 0 tests"), so that is a failure here.
do_test() {
  filter=${1:-}
  [ -n "$filter" ] || usage
  say "cargo test --locked $filter"
  cargo test --locked "$filter" > "$OUT/test.log" 2>&1 && rc=0 || rc=$?
  grep -E '^(running [0-9]+ tests|test result:)' "$OUT/test.log"
  if grep -q '^running 0 tests' "$OUT/test.log"; then
    echo "filter '$filter' selected no tests (see $OUT/test.log)" >&2
    exit 1
  fi
  [ "$rc" -eq 0 ] || { echo "tests failed (exit $rc), see $OUT/test.log" >&2; exit "$rc"; }
}

cmd=${1:-}
[ -n "$cmd" ] || usage
shift
case "$cmd" in
  build) do_build ;;
  args) do_args ;;
  provider) do_provider "$@" ;;
  voice) do_voice ;;
  status) do_status ;;
  plan) do_plan "$@" ;;
  test) do_test "$@" ;;
  all) do_build; do_args; do_provider primary; do_voice; do_status ;;
  *) usage ;;
esac
echo "outputs: $OUT"
