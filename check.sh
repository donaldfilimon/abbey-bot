#!/bin/sh
# Abbey Bot gate — fmt + clippy + tests + the deployed release artifact.
#
# Note the deliberate absence of a pipe anywhere below: `cmd | tail` reports
# tail's exit status, not cmd's, which is how a red suite reads as green.
set -eu
cd "$(dirname "$0")"

echo "== fmt =="
cargo fmt --all -- --check

echo "== deployment and privacy validation =="
# Shell syntax is ENUMERATED, not listed. The hand-maintained list this replaced
# had gone stale: install-oh-autolisten-launchd.sh and the 5.8 KB watcher it
# installs, watch-office-hours-auto-listen.sh, were never parsed by any gate.
# A glob covers the next service the day it lands instead of the day someone
# notices. Every script under these two directories is #!/bin/sh (verified
# 2026-09-16), so `sh -n` is the correct parser for all of them; add a shebang
# dispatch here if a bash script ever lands, because dash on the Linux lane will
# not forgive bashisms that macOS's bash-as-sh accepts. Root-level launch.sh and
# run_bot.sh are deliberately out of scope: both are untracked local-only
# wrappers and do not exist on a CI runner.
shell_checked=0
for deploy_script in deploy/*.sh scripts/*.sh; do
  sh -n "$deploy_script"
  shell_checked=$((shell_checked + 1))
done
# A selector that quietly matches nothing reads exactly like a pass. An unmatched
# glob stays literal under POSIX sh so `sh -n 'deploy/*.sh'` already fails loudly,
# but assert the count too: that covers the day someone narrows the pattern.
[ "$shell_checked" -gt 0 ]
echo "shell syntax: ${shell_checked} script(s)"
python3 scripts/check-python-syntax.py
python3 deploy/check-python-locks.py \
  deploy/mlx-vlm-requirements.txt \
  deploy/mlx-audio-requirements.txt \
  deploy/mlx-audio-build-constraints.txt
python3 deploy/test-configure-mlx-primary.py
python3 deploy/test-publish-provider-qualification.py
python3 deploy/test-check-launchd-env.py
python3 deploy/test-check-activity-url-map.py
python3 deploy/check-activity-url-map.py
python3 deploy/test-service-protocol.py
python3 deploy/test-service-environment.py
python3 deploy/test-service-installation.py
python3 deploy/test-service-readiness.py
python3 deploy/test-service-status.py
python3 deploy/test-install-launchd.py
python3 deploy/test-install-audio-tap-launchd.py
python3 deploy/test-install-wdbx-gateway-launchd.py
python3 deploy/test-smoke-mlx-vlm-tool-deltas.py
python3 deploy/test-patch-mlx-vlm-tool-encoding.py
python3 scripts/check-privacy.py
python3 scripts/test-check-rust-module-size.py
python3 scripts/check-rust-module-size.py
python3 scripts/test-check-pages-liquid.py
python3 scripts/check-pages-liquid.py
python3 scripts/test-check-abbey-contracts.py
python3 scripts/check-abbey-contracts.py
python3 scripts/test-check-linux-tls-tree.py
python3 scripts/check-linux-tls-tree.py
python3 scripts/test-check-rustsec-debt.py
python3 scripts/check-rustsec-debt.py
python3 scripts/test-check-wdbx-conformance.py
python3 scripts/check-wdbx-conformance.py
python3 scripts/test-check-systemd-unit.py
python3 scripts/check-systemd-unit.py
if command -v plutil >/dev/null 2>&1; then
  # Enumerated for the same reason as the shell loop above: the hand-maintained
  # list omitted abbey-oh-autolisten.plist and abbey-wdbx-gateway.plist, both of
  # which belong to services that are actually loaded. A plist holding an install
  # placeholder (abbey-oh-autolisten's __HOME__) is still valid plist and lints
  # clean, so templating is not a reason to leave one out.
  plist_checked=0
  for deploy_plist in deploy/*.plist; do
    plutil -lint "$deploy_plist"
    plist_checked=$((plist_checked + 1))
  done
  [ "$plist_checked" -gt 0 ]
  echo "plist lint: ${plist_checked} plist(s)"
fi

echo "== offline macOS audio tap =="
sh scripts/check-audio-tap.sh

echo "== clippy =="
cargo clippy --all-targets --locked -- -D warnings

echo "== test =="
# --locked here proves what the Dockerfile's --locked build depends on:
# a Cargo.toml bump without a regenerated lock fails THIS gate, not the deploy.
cargo test --locked

echo "== release build =="
cargo build --release --locked

echo "== ok =="
