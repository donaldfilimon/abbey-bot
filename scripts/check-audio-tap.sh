#!/bin/sh
# Offline sidecar gate. Tests use synthetic PCM; never start real capture.
set -eu
cd "$(dirname "$0")/.."

if [ "$(uname -s)" != Darwin ]; then
    echo "audio-tap Swift gate: skipped (requires macOS ScreenCaptureKit SDK)"
    exit 0
fi

scratch=$(mktemp -d "${TMPDIR:-/tmp}/abbey-audio-tap-check.XXXXXXXX")
trap 'rm -rf "$scratch"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

env -u TOOLCHAINS xcrun swift test \
    --package-path tools/abbey-audio-tap --scratch-path "$scratch"
env -u TOOLCHAINS xcrun swift build -c release \
    --package-path tools/abbey-audio-tap --scratch-path "$scratch"
echo "audio-tap Swift gate: offline tests and release build passed"
