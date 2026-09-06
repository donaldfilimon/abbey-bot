#!/bin/sh
# Install or update the WDBX episode gateway as a launchd user agent on this
# Mac. It is the ledger the bot's episode gate (`ABBEY_EPISODE_GATE_CONFIG`)
# proposes to, so it must be up before the bot is restarted with the gate on.
#
#   ./deploy/install-wdbx-gateway-launchd.sh ABI_RELEASE_DIR   # install/update + (re)load
#   ./deploy/install-wdbx-gateway-launchd.sh --uninstall
#
# ABI_RELEASE_DIR is the sibling abi workspace's `target/release` after
# `cargo build --release -p abi-cli -p abi-wdbx-gateway` there; abbey-bot
# never builds abi itself (it takes no dependency on it, see AGENTS.md).
#
# Layout (all under $HOME, nothing needs sudo):
#   ~/.local/libexec/abbey-bot/{abi,abi-wdbx-gateway,libabi_*.dylib}
#   ~/.local/share/abbey-bot/wdbx-gateway/        the store; ledger under episodes/
#   ~/.config/abbey-bot/episode-gateway-token     bearer token (chmod 600; you write it)
#   ~/.config/abbey-bot/episode-policy.json       StorePolicy JSON (you write it)
#   ~/Library/Logs/abbey-bot/wdbx-gateway.log
#   ~/Library/LaunchAgents/com.donaldfilimon.abbey-wdbx-gateway.plist
#
# Never prints the token. No pipe decides an exit status below.
set -eu
cd "$(dirname "$0")/.."

LABEL=com.donaldfilimon.abbey-wdbx-gateway
PLIST_SRC=deploy/$LABEL.plist
PLIST_DST="$HOME/Library/LaunchAgents/$LABEL.plist"
BIN_DIR="$HOME/.local/libexec/abbey-bot"
STORE_DIR="$HOME/.local/share/abbey-bot/wdbx-gateway"
LOG_DIR="$HOME/Library/Logs/abbey-bot"
BEARER_PATH="$HOME/.config/abbey-bot/episode-gateway-token"
POLICY_FILE="$HOME/.config/abbey-bot/episode-policy.json"
ENDPOINT=http://127.0.0.1:50051
UID_NUM=$(id -u)

usage() {
  echo "usage: $0 ABI_RELEASE_DIR | --uninstall" >&2
  exit 2
}

service_loaded() {
  launchctl print "gui/$UID_NUM/$LABEL" >/dev/null 2>&1
}

unload_service() {
  if service_loaded; then
    launchctl bootout "gui/$UID_NUM/$LABEL"
    tries=0
    while service_loaded; do
      tries=$((tries + 1))
      if [ "$tries" -gt 50 ]; then
        echo "gateway service did not unload within 10 seconds" >&2
        return 1
      fi
      sleep 0.2
    done
  fi
}

if [ "$#" -ne 1 ]; then
  usage
fi

if [ "$1" = "--uninstall" ]; then
  unload_service
  rm -f "$PLIST_DST"
  echo "uninstalled $LABEL; binaries, store, token, and policy left in place"
  exit 0
fi

RELEASE_DIR=$1
for name in abi abi-wdbx-gateway libabi_fm_shim.dylib libabi_metal_dot.dylib; do
  if [ ! -f "$RELEASE_DIR/$name" ]; then
    echo "missing $RELEASE_DIR/$name (build abi-cli and abi-wdbx-gateway --release in the abi workspace)" >&2
    exit 1
  fi
done
for required in "$BEARER_PATH" "$POLICY_FILE"; do
  if [ ! -f "$required" ]; then
    echo "missing $required" >&2
    exit 1
  fi
done
token_mode=$(stat -f '%Lp' "$BEARER_PATH")
if [ "$token_mode" != "600" ]; then
  echo "$BEARER_PATH must be mode 600 (is $token_mode)" >&2
  exit 1
fi

umask 077
mkdir -p "$BIN_DIR" "$STORE_DIR" "$LOG_DIR" "$HOME/Library/LaunchAgents"
chmod 700 "$STORE_DIR"

# Stop the running gateway before replacing its binary; a stale process
# holding the old inode would keep serving the old build.
unload_service

for name in abi abi-wdbx-gateway libabi_fm_shim.dylib libabi_metal_dot.dylib; do
  /bin/cp -f "$RELEASE_DIR/$name" "$BIN_DIR/$name.new"
  chmod 755 "$BIN_DIR/$name.new"
  mv -f "$BIN_DIR/$name.new" "$BIN_DIR/$name"
done

# `abi` loads its two shims from @loader_path; a missing one aborts at dyld
# time, which `help` (exit 0) would not survive.
if ! "$BIN_DIR/abi" help >/dev/null 2>&1; then
  echo "$BIN_DIR/abi does not start (missing libabi_*.dylib beside it?)" >&2
  exit 1
fi

/usr/bin/sed "s|__HOME__|$HOME|g" "$PLIST_SRC" >"$PLIST_DST.new"
chmod 644 "$PLIST_DST.new"
mv -f "$PLIST_DST.new" "$PLIST_DST"
plutil -lint "$PLIST_DST" >/dev/null

launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"

# Readiness: a verify of the all-zero digest answers found=false only when the
# gateway is up, the token matches, and the policy loaded (an unknown guild or
# a policy failure answers differently). Reads nothing and appends nothing.
guild_ref=$(python3 -c 'import json,sys; print(sorted(json.load(open(sys.argv[1]))["guilds"])[0])' "$POLICY_FILE")
zero=0000000000000000000000000000000000000000000000000000000000000000
tries=0
until out=$("$BIN_DIR/abi" wdbx episode verify "$guild_ref" "$zero" \
  --endpoint "$ENDPOINT" --token-file "$BEARER_PATH" --json 2>/dev/null) \
  || [ "$out" = '{"found":"false"}' ]; do
  tries=$((tries + 1))
  if [ "$tries" -gt 50 ]; then
    echo "gateway did not answer a verify on $ENDPOINT within 10 seconds; see $LOG_DIR/wdbx-gateway.log" >&2
    exit 1
  fi
  sleep 0.2
done
if [ "$out" != '{"found":"false"}' ]; then
  echo "unexpected verify answer from the gateway: $out" >&2
  exit 1
fi
pid=$(launchctl print "gui/$UID_NUM/$LABEL" | /usr/bin/awk '/^[[:space:]]*pid = /{print $3; exit}')
echo "WDBX gateway ready: pid ${pid:-unknown}, $ENDPOINT, store $STORE_DIR"
echo "log: $LOG_DIR/wdbx-gateway.log"
echo "next: add ABBEY_EPISODE_GATE_CONFIG=\$HOME/.config/abbey-bot/episode-gate.json to ~/.config/abbey-bot/env and restart the bot (launchctl kickstart -k gui/$UID_NUM/com.donaldfilimon.abbey-bot; SIGTERM persists first)"
