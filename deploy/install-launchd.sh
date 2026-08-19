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
DATA_DIR="$HOME/.local/share/abbey-bot/data"
LOG_DIR="$HOME/Library/Logs/abbey-bot"
ENV_FILE="$HOME/.config/abbey-bot/env"
UID_NUM=$(id -u)

if [ "${1:-}" = "--uninstall" ]; then
  launchctl bootout "gui/$UID_NUM/$LABEL" 2>/dev/null || true
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
  ABBEY_GUILD_ID, ABBEY_MESSAGE_CONTENT, ABBEY_QUIET, RUST_LOG
(ABBEY_DATA_DIR is set by the plist; a line here overrides it.)
MSG
  exit 1
fi

echo "== build"
cargo build --release --locked

echo "== install"
mkdir -p "$BIN_DIR" "$DATA_DIR" "$LOG_DIR" "$HOME/Library/LaunchAgents" "$HOME/.local/share/abbey-bot"
/bin/cp -f target/release/abbey-bot "$BIN_DIR/abbey-bot"
sed "s|__HOME__|$HOME|g" "$PLIST_SRC" > "$PLIST_DST.tmp"
mv -f "$PLIST_DST.tmp" "$PLIST_DST"
plutil -lint "$PLIST_DST"

echo "== (re)load"
launchctl bootout "gui/$UID_NUM/$LABEL" 2>/dev/null || true
launchctl bootstrap "gui/$UID_NUM" "$PLIST_DST"
launchctl kickstart -k "gui/$UID_NUM/$LABEL"
sleep 2
launchctl print "gui/$UID_NUM/$LABEL" | grep -E "state|pid" || true
echo "log: $LOG_DIR/abbey-bot.log"
