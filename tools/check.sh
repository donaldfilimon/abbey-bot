#!/bin/sh
# abbey-bot-zig gate. One command; the exit code is the verdict.
#
# No stage pipes a command into another: `cmd | tail` reports tail's status,
# which is how a red suite reads as green. Every tool writes to a log file and
# its own exit status is checked directly.
set -eu
cd "$(dirname "$0")/.."
LOG_DIR="${TMPDIR:-/tmp}/abbey-bot-zig-gate"
mkdir -p "$LOG_DIR"
stage() { printf '== %s ==\n' "$1"; }
run_logged() {
  name=$1; shift
  if "$@" > "$LOG_DIR/$name.log" 2>&1; then
    return 0
  else
    status=$?
    cat "$LOG_DIR/$name.log"
    printf 'FAIL: %s (exit %s; log %s)\n' "$name" "$status" "$LOG_DIR/$name.log"
    exit 1
  fi
}

stage "toolchain"
want=$(sed -n 's/.*\.minimum_zig_version = "\(.*\)",/\1/p' build.zig.zon)
have=$(zig version)
echo "zig $have (minimum $want)"
[ "$have" = "$want" ] || { echo "FAIL: zig $have does not equal the pinned $want"; exit 1; }

stage "fmt"
run_logged fmt zig fmt --check build.zig build.zig.zon src
echo "zig fmt --check: clean"

stage "build"
run_logged build zig build
echo "zig build: ok"

stage "test (leak-checked)"
run_logged test zig build test --summary all
sed -n 's/^Build Summary: .*; \([0-9]*\/[0-9]* tests passed\).*/\1/p' "$LOG_DIR/test.log"
grep -q 'tests passed' "$LOG_DIR/test.log" || { echo "FAIL: no test count in log"; exit 1; }
if grep -q 'leaked' "$LOG_DIR/test.log"; then echo "FAIL: leak reported"; exit 1; fi

stage "catalog parity (CLI)"
run_logged catalog ./zig-out/bin/abbey-bot-zig catalog-json
run_logged catalog-cmp cmp contracts/catalog/command-payload.json "$LOG_DIR/catalog.log"
echo "abbey-bot-zig catalog-json is byte-identical to contracts/catalog/command-payload.json"

stage "contract corpus"
run_logged contracts python3 scripts/check-abbey-contracts.py
cat "$LOG_DIR/contracts.log"
run_logged contracts-selftest python3 scripts/test-check-abbey-contracts.py
echo "check-abbey-contracts self-test: ok"

stage "wdbx fixture parity"
if [ -z "${ABBEY_WDBX_REPO:-}" ] && [ -d ../wdbx ]; then ABBEY_WDBX_REPO=../wdbx; export ABBEY_WDBX_REPO; fi
# With a sibling WDBX checkout present the comparison is required, not skippable.
if [ -n "${ABBEY_WDBX_REPO:-}" ]; then ABBEY_REQUIRE_WDBX_CONFORMANCE=1; export ABBEY_REQUIRE_WDBX_CONFORMANCE; fi
run_logged wdbx python3 scripts/check-wdbx-conformance.py
cat "$LOG_DIR/wdbx.log"
run_logged wdbx-selftest python3 scripts/test-check-wdbx-conformance.py
echo "check-wdbx-conformance self-test: ok"

stage "generated tables"
run_logged unicode python3 tools/gen_unicode_tables.py --check
cat "$LOG_DIR/unicode.log"

stage "claims"
run_logged claims python3 tools/claims.py --check
cat "$LOG_DIR/claims.log"

stage "size guard"
over=0
for f in $(find src tools scripts -type f \( -name '*.zig' -o -name '*.py' -o -name '*.sh' \)); do
  n=$(wc -l < "$f")
  if [ "$n" -gt 1000 ]; then echo "over 1000 lines: $f ($n)"; over=1; fi
done
[ "$over" -eq 0 ] || { echo "FAIL: size guard"; exit 1; }
echo "every source file is at most 1000 lines"

echo "== verdict: abbey-bot-zig gate PASSED =="
