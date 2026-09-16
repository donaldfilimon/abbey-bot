#!/bin/sh
# Install the interim Office Hours auto-listen kickstart watcher LaunchAgent.
# Usage: /bin/sh deploy/install-oh-autolisten-launchd.sh [--uninstall]
set -eu
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
LABEL=com.donaldfilimon.abbey-oh-autolisten
UID_NUM=$(/usr/bin/id -u)
HOME_DIR=${HOME:?}
LIBEXEC="$HOME_DIR/.local/libexec/abbey-bot"
AGENTS="$HOME_DIR/Library/LaunchAgents"
LOG_DIR="$HOME_DIR/Library/Logs/abbey-bot"
PLIST_SRC="$ROOT/deploy/${LABEL}.plist"
PLIST_DST="$AGENTS/${LABEL}.plist"
SCRIPT_SRC="$ROOT/deploy/watch-office-hours-auto-listen.sh"
SCRIPT_DST="$LIBEXEC/watch-office-hours-auto-listen.sh"

uninstall() {
  /bin/launchctl bootout "gui/${UID_NUM}/${LABEL}" 2>/dev/null || true
  /bin/rm -f "$PLIST_DST"
  echo "unloaded ${LABEL}"
}

case "${1:-}" in
  --uninstall) uninstall; exit 0 ;;
  '' ) ;;
  *) echo "usage: $0 [--uninstall]" >&2; exit 2 ;;
esac

mkdir -p "$LIBEXEC" "$AGENTS" "$LOG_DIR"
/bin/cp "$SCRIPT_SRC" "$SCRIPT_DST"
/bin/chmod 755 "$SCRIPT_DST"
/usr/bin/sed "s|__HOME__|${HOME_DIR}|g" "$PLIST_SRC" >"$PLIST_DST"
/bin/chmod 644 "$PLIST_DST"
/bin/launchctl bootout "gui/${UID_NUM}/${LABEL}" 2>/dev/null || true
/bin/launchctl bootstrap "gui/${UID_NUM}" "$PLIST_DST"
/bin/launchctl enable "gui/${UID_NUM}/${LABEL}" 2>/dev/null || true
/bin/launchctl kickstart -k "gui/${UID_NUM}/${LABEL}" 2>/dev/null || true
echo "installed ${LABEL}"
echo "script: ${SCRIPT_DST}"
echo "plist:  ${PLIST_DST}"
echo "log:    ${LOG_DIR}/oh-autolisten.log"
