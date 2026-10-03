#!/bin/sh
# run-abbey-bot driver: build and drive the abbey-bot release binary through
# its token-free modes, never through a second Discord gateway.
#
# The launchd service com.donaldfilimon.abbey-bot already owns the live token
# on this machine. Every non-plan mode below runs with an explicit nonsecret
# environment allowlist, blanks persistence and episode-gate paths, and runs in
# the foreground with a timeout. The one exception is `plan`, which passes only
# the Discord credential needed for a read-only REST diff and refuses `--apply`.
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

# Read the owner-only env file as literal data. The loopback endpoints
# (ABBEY_BOT_LLM_ENDPOINT, ABBEY_VOICE_LOCAL_ENDPOINT, ...) resolve, but the
# file is never evaluated as shell code. Callers run this inside a subshell;
# only the plan loader opts into DISCORD_TOKEN, so it never lingers in a
# non-plan driver process.
_env_trim() {
  _trimmed=$1
  while :; do
    case "$_trimmed" in
      [[:space:]]*) _trimmed=${_trimmed#?};;
      *) break;;
    esac
  done
  while :; do
    case "$_trimmed" in
      *[[:space:]]) _trimmed=${_trimmed%?};;
      *) break;;
    esac
  done
  printf '%s' "$_trimmed"
}

_env_error() {
  printf '%s\n' 'smoke.sh: refusing invalid or unsafe environment file' >&2
}

# Keep this list limited to the nonsecret values explicitly selected by
# clean_env; base process variables such as HOME and PATH remain inherited.
_env_key_allowed() {
  case "$1" in
    RUST_LOG|\
    ABBEY_BOT_LLM_ENDPOINT|\
    ABBEY_BOT_LLM_MODEL|\
    ABBEY_BOT_LLM_CONCURRENCY|\
    ABBEY_BOT_LLM_QUEUE_SECS|\
    ABBEY_BOT_LLM_TIMEOUT_SECS|\
    ABBEY_BOT_LLM_TOOLS|\
    ABBEY_VISION_PROVIDER|\
    ABBEY_VISION_ENDPOINT|\
    ABBEY_VISION_MODEL|\
    ABBEY_FM_MODE|\
    ABBEY_FM_ROLE|\
    ABBEY_FM_ENDPOINT|\
    ABBEY_FM_CLI|\
    ABBEY_FM_FALLBACK|\
    ABBEY_FM_PCC_TIMEOUT_SECS|\
    ABBEY_FM_CAPABILITY_MANIFEST|\
    ABBEY_VOICE_LOCAL_ENDPOINT|\
    ABBEY_VOICE_LOCAL_STT_MODEL|\
    ABBEY_VOICE_LOCAL_TTS_MODEL|\
    ABBEY_VOICE_LOCAL_TTS_VOICE|\
    ABBEY_VOICE_LOCAL_LANGUAGE|\
    ABBEY_VOICE_WAKE_WORD_REQUIRED|\
    ABBEY_VOICE_WAKE_WORDS)
      return 0
      ;;
    DISCORD_TOKEN|DISCORD_BOT_TOKEN)
      [ "${2:-0}" -eq 1 ]
      ;;
    *)
      return 1
      ;;
  esac
}

_env_stat() {
  _env_stat_format=$1
  if _env_stat_value=$(stat -c "$_env_stat_format" "$ENV_FILE" 2>/dev/null); then
    printf '%s' "$_env_stat_value"
    return 0
  fi
  _env_stat_value=$(stat -f "$_env_stat_format" "$ENV_FILE" 2>/dev/null) || return 1
  printf '%s' "$_env_stat_value"
}

_read_env() {
  _seen_keys=' '
  while IFS= read -r _line || [ -n "$_line" ]; do
    _line=$(_env_trim "$_line")
    [ -n "$_line" ] || continue
    case "$_line" in
      '#'*) continue;;
    esac
    case "$_line" in
      export[[:space:]]*)
        _line=${_line#export}
        _line=$(_env_trim "$_line")
        ;;
    esac
    case "$_line" in
      *=*) ;;
      *) _env_error; return 1;;
    esac

    _key=${_line%%=*}
    case "$_key" in
      [A-Za-z_]*) ;;
      *) _env_error; return 1;;
    esac
    case "$_key" in
      *[!A-Za-z0-9_]*) _env_error; return 1;;
    esac
    case "$_seen_keys" in
      *" $_key "*) _env_error; return 1;;
    esac
    _seen_keys="${_seen_keys}${_key} "

    _value=$(_env_trim "${_line#*=}")
    case "$_value" in
      \"*)
        [ "${#_value}" -ge 2 ] || { _env_error; return 1; }
        case "$_value" in
          *\") _value=${_value#\"}; _value=${_value%\"};;
          *) _env_error; return 1;;
        esac
        ;;
      \'*)
        [ "${#_value}" -ge 2 ] || { _env_error; return 1; }
        case "$_value" in
          *\') _value=${_value#\'}; _value=${_value%\'};;
          *) _env_error; return 1;;
        esac
        ;;
    esac

    # Validate every assignment, but export only the bounded driver allowlist.
    if _env_key_allowed "$_key" "${_load_discord_token:-0}"; then
      export "$_key=$_value"
    fi
  done < "$ENV_FILE"
}

load_env() {
  # An absent optional env file keeps the existing token-free degraded behavior.
  if [ ! -e "$ENV_FILE" ] && [ ! -L "$ENV_FILE" ]; then
    return 0
  fi

  case $- in *x*) _smoke_trace=1 ;; *) _smoke_trace=0 ;; esac
  set +x
  case "${1:-}" in
    token) _load_discord_token=1 ;;
    *) _load_discord_token=0 ;;
  esac
  _smoke_rc=0
  if [ -L "$ENV_FILE" ] || [ ! -f "$ENV_FILE" ] || [ ! -r "$ENV_FILE" ]; then
    _env_error
    _smoke_rc=1
  else
    _env_mode=$(_env_stat '%Lp') || _env_mode=
    _env_owner=$(_env_stat '%u') || _env_owner=
    case "$_env_mode" in
      400|600) ;;
      *) _env_error; _smoke_rc=1;;
    esac
    if [ "$_smoke_rc" -ne 1 ] && [ "$_env_owner" != "$(id -u)" ]; then
      _env_error
      _smoke_rc=1
    fi
    if [ "$_smoke_rc" -ne 1 ]; then
      _env_size=$(wc -c < "$ENV_FILE") || _env_size=
      _env_nul_size=$(tr -d '\000' < "$ENV_FILE" | wc -c) || _env_nul_size=
      if [ -z "$_env_size" ] || [ "$_env_size" -gt 65536 ] || \
          [ -z "$_env_nul_size" ] || [ "$_env_size" -ne "$_env_nul_size" ]; then
        _env_error
        _smoke_rc=1
      fi
    fi
    if [ "$_smoke_rc" -ne 1 ]; then
      if _read_env; then
        _smoke_rc=0
      else
        _smoke_rc=$?
      fi
    fi
  fi
  if [ "$_smoke_trace" -eq 1 ]; then
    set -x
  fi
  _smoke_result=$_smoke_rc
  unset _smoke_trace _smoke_rc _env_mode _env_owner _env_size _env_nul_size _load_discord_token
  return "$_smoke_result"
}

# Run the binary with a small nonsecret environment. Explicitly selecting the
# configuration keys avoids inheriting vision, provider, connector, and Discord
# credentials from the owner environment.
clean_env() {
  case $- in *x*) _smoke_env_trace=1 ;; *) _smoke_env_trace=0 ;; esac
  set +x
  if env -i \
    PATH="${PATH:-/usr/bin:/bin}" \
    HOME="${HOME:?}" \
    TMPDIR="${TMPDIR:-/tmp}" \
    CARGO_HOME="${CARGO_HOME:-${HOME}/.cargo}" \
    RUSTUP_HOME="${RUSTUP_HOME:-${HOME}/.rustup}" \
    RUST_LOG="${RUST_LOG:-off}" \
    ABBEY_DATA_DIR= \
    ABBEY_EPISODE_GATE_CONFIG= \
    ABBEY_EPISODE_GATE_ACCEPTANCE_CONFIG= \
    ABBEY_BOT_LLM_ENDPOINT="${ABBEY_BOT_LLM_ENDPOINT:-}" \
    ABBEY_BOT_LLM_MODEL="${ABBEY_BOT_LLM_MODEL:-}" \
    ABBEY_BOT_LLM_CONCURRENCY="${ABBEY_BOT_LLM_CONCURRENCY:-}" \
    ABBEY_BOT_LLM_QUEUE_SECS="${ABBEY_BOT_LLM_QUEUE_SECS:-}" \
    ABBEY_BOT_LLM_TIMEOUT_SECS="${ABBEY_BOT_LLM_TIMEOUT_SECS:-}" \
    ABBEY_BOT_LLM_TOOLS="${ABBEY_BOT_LLM_TOOLS:-}" \
    ABBEY_VISION_PROVIDER="${ABBEY_VISION_PROVIDER:-}" \
    ABBEY_VISION_ENDPOINT="${ABBEY_VISION_ENDPOINT:-}" \
    ABBEY_VISION_MODEL="${ABBEY_VISION_MODEL:-}" \
    ABBEY_FM_MODE="${ABBEY_FM_MODE:-}" \
    ABBEY_FM_ROLE="${ABBEY_FM_ROLE:-}" \
    ABBEY_FM_ENDPOINT="${ABBEY_FM_ENDPOINT:-}" \
    ABBEY_FM_CLI="${ABBEY_FM_CLI:-}" \
    ABBEY_FM_FALLBACK="${ABBEY_FM_FALLBACK:-}" \
    ABBEY_FM_PCC_TIMEOUT_SECS="${ABBEY_FM_PCC_TIMEOUT_SECS:-}" \
    ABBEY_FM_CAPABILITY_MANIFEST="${ABBEY_FM_CAPABILITY_MANIFEST:-}" \
    ABBEY_VOICE_LOCAL_ENDPOINT="${ABBEY_VOICE_LOCAL_ENDPOINT:-}" \
    ABBEY_VOICE_LOCAL_STT_MODEL="${ABBEY_VOICE_LOCAL_STT_MODEL:-}" \
    ABBEY_VOICE_LOCAL_TTS_MODEL="${ABBEY_VOICE_LOCAL_TTS_MODEL:-}" \
    ABBEY_VOICE_LOCAL_TTS_VOICE="${ABBEY_VOICE_LOCAL_TTS_VOICE:-}" \
    ABBEY_VOICE_LOCAL_LANGUAGE="${ABBEY_VOICE_LOCAL_LANGUAGE:-}" \
    ABBEY_VOICE_WAKE_WORD_REQUIRED="${ABBEY_VOICE_WAKE_WORD_REQUIRED:-}" \
    ABBEY_VOICE_WAKE_WORDS="${ABBEY_VOICE_WAKE_WORDS:-}" \
    "$@"; then
    _smoke_env_rc=0
  else
    _smoke_env_rc=$?
  fi
  if [ "$_smoke_env_trace" -eq 1 ]; then
    set -x
  fi
  _smoke_env_result=$_smoke_env_rc
  unset _smoke_env_trace _smoke_env_rc
  return "$_smoke_env_result"
}

plan_env() {
  case $- in *x*) _smoke_env_trace=1 ;; *) _smoke_env_trace=0 ;; esac
  set +x
  if [ "${DISCORD_TOKEN+x}" = x ]; then
    set -- "DISCORD_TOKEN=$DISCORD_TOKEN" "$@"
  fi
  if [ "${DISCORD_BOT_TOKEN+x}" = x ]; then
    set -- "DISCORD_BOT_TOKEN=$DISCORD_BOT_TOKEN" "$@"
  fi
  if env -i \
    PATH="${PATH:-/usr/bin:/bin}" \
    HOME="${HOME:?}" \
    TMPDIR="${TMPDIR:-/tmp}" \
    RUST_LOG="${RUST_LOG:-off}" \
    ABBEY_DATA_DIR= \
    ABBEY_EPISODE_GATE_CONFIG= \
    ABBEY_EPISODE_GATE_ACCEPTANCE_CONFIG= \
    "$@"; then
    _smoke_env_rc=0
  else
    _smoke_env_rc=$?
  fi
  if [ "$_smoke_env_trace" -eq 1 ]; then
    set -x
  fi
  _smoke_env_result=$_smoke_env_rc
  unset _smoke_env_trace _smoke_env_rc
  return "$_smoke_env_result"
}

need_bin() {
  [ -x "$BIN" ] || { echo "missing $BIN; run: smoke.sh build" >&2; exit 1; }
}

do_build() {
  say build
  clean_env cargo build --locked --release > "$OUT/build.log" 2>&1 && rc=0 || rc=$?
  clean_env tail -2 "$OUT/build.log"
  [ "$rc" -eq 0 ] || { echo "build failed (exit $rc), see $OUT/build.log" >&2; exit "$rc"; }
  clean_env shasum -a 256 "$BIN"
}

# Prove the binary you built is the one you are driving: bad argv exits 2.
do_args() {
  say args
  need_bin
  for argv in "--bogus" "--provider-self-test primary" "--voice-self-test"; do
    # shellcheck disable=SC2086
    clean_env "$BIN" $argv > /dev/null 2> "$OUT/args.stderr" && rc=0 || rc=$?
    if [ "$rc" -ne 2 ]; then
      echo "expected exit 2 for '$argv', got $rc" >&2; clean_env cat "$OUT/args.stderr" >&2; exit 1
    fi
    printf 'exit 2 ok: abbey-bot %s\n' "$argv"
  done
}

# Qualify the configured reasoning/vision route with synthetic fixtures.
# No Discord, no state. `all` also probes every configured Apple FM mode.
# CLI exits: 0 success, 1 probe failure, 2 configuration failure.
do_provider() {
  target=${1:-primary}
  say "provider self-test ($target)"
  need_bin
  start=$(date +%s)
  ( load_env && clean_env timeout 600 "$BIN" --provider-self-test "$target" --json ) \
    > "$OUT/provider-$target.json" 2> "$OUT/provider-$target.stderr" && rc=0 || rc=$?
  echo "exit $rc in $(( $(date +%s) - start ))s -> $OUT/provider-$target.json"
  [ -s "$OUT/provider-$target.stderr" ] && clean_env cat "$OUT/provider-$target.stderr" >&2
  if clean_env python3 - "$OUT/provider-$target.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
identity = d["primary"].get("identity") or {}
binary_sha = identity.get("abbey_binary_sha256")
print("overall_pass:", d["overall_pass"], "| binary sha256:", binary_sha[:16] if binary_sha else "unavailable")
fm_modes = d.get("fm_cli_modes") or []

def summarize(lane, body):
    caps = " ".join(f"{k}={v['status']}" for k, v in body["capabilities"].items())
    print(f"  {lane:10} configured={body['configured']!s:5} {caps}")

for lane, body in d.items():
    if lane == "fm_cli" and fm_modes:
        continue
    if isinstance(body, dict) and "capabilities" in body:
        summarize(lane, body)
for index, body in enumerate(fm_modes, 1):
    mode = (body.get("identity") or {}).get("mode") or str(index)
    summarize(f"fm_cli[{mode}]", body)
PY
  then
    summary_rc=0
  else
    summary_rc=$?
  fi
  # A failed report must not replace the probe's original nonzero exit.
  [ "$rc" -ne 0 ] || rc=$summary_rc
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
  ( load_env && clean_env timeout 600 env ABBEY_VISION_ENDPOINT=off "$BIN" --voice-self-test "$wav" ) \
    > "$OUT/voice.stdout" 2> "$OUT/voice.stderr" && rc=0 || rc=$?
  echo "exit $rc in $(( $(date +%s) - start ))s"
  clean_env cat "$OUT/voice.stdout"
  [ -s "$OUT/voice.stderr" ] && clean_env cat "$OUT/voice.stderr" >&2
  [ "$rc" -eq 0 ] && clean_env file "$wav"
  return "$rc"
}

# Read-only evidence about the LIVE launchd service. Never restarts it.
do_status() {
  say "deployed service (read-only)"
  clean_env python3 -I deploy/service-status.py
  clean_env sh deploy/check-launchd-env.sh "$ENV_FILE"
  clean_env launchctl list | clean_env grep -E 'abbey' || true
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
  ( load_env token && plan_env timeout 300 "$BIN" \
      --server-plan blueprints/mlai-community.toml --guild "$guild" "$@" ) \
    > "$OUT/plan.txt" 2> "$OUT/plan.stderr" && rc=0 || rc=$?
  echo "exit $rc -> $OUT/plan.txt"
  clean_env cat "$OUT/plan.txt"
  [ -s "$OUT/plan.stderr" ] && clean_env cat "$OUT/plan.stderr" >&2
  return "$rc"
}

# Direct invocation for PR work: tests live in the binary. A filter that
# matches nothing still exits 0 ("running 0 tests"), so that is a failure here.
do_test() {
  filter=${1:-}
  [ -n "$filter" ] || usage
  say "cargo test --locked $filter"
  clean_env cargo test --locked "$filter" > "$OUT/test.log" 2>&1 && rc=0 || rc=$?
  clean_env grep -E '^(running [0-9]+ tests|test result:)' "$OUT/test.log"
  if clean_env grep -q '^running 0 tests' "$OUT/test.log"; then
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
