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
sh -n deploy/install-launchd.sh
sh -n deploy/check-launchd-env.sh
sh -n deploy/install-mlx-audio-launchd.sh
sh -n deploy/install-mlx-vlm-launchd.sh
sh -n deploy/run-mlx-audio.sh
sh -n deploy/run-mlx-vlm.sh
python3 scripts/check-python-syntax.py
python3 deploy/check-python-locks.py \
  deploy/mlx-vlm-requirements.txt \
  deploy/mlx-audio-requirements.txt \
  deploy/mlx-audio-build-constraints.txt
python3 deploy/test-configure-mlx-primary.py
python3 deploy/test-publish-provider-qualification.py
python3 deploy/test-check-launchd-env.py
python3 deploy/test-smoke-mlx-vlm-tool-deltas.py
python3 deploy/test-patch-mlx-vlm-tool-encoding.py
python3 scripts/check-privacy.py
python3 scripts/test-check-abbey-contracts.py
python3 scripts/check-abbey-contracts.py
python3 scripts/test-check-linux-tls-tree.py
python3 scripts/check-linux-tls-tree.py
python3 scripts/test-check-rustsec-debt.py
python3 scripts/check-rustsec-debt.py
python3 scripts/test-check-wdbx-conformance.py
python3 scripts/check-wdbx-conformance.py
if command -v plutil >/dev/null 2>&1; then
  plutil -lint deploy/com.donaldfilimon.abbey-bot.plist
  plutil -lint deploy/com.donaldfilimon.abbey-mlx-audio.plist
  plutil -lint deploy/com.donaldfilimon.abbey-mlx-vlm.plist
fi

echo "== clippy =="
cargo clippy --all-targets --locked -- -D warnings

echo "== test =="
# --locked here proves what the Dockerfile's --locked build depends on:
# a Cargo.toml bump without a regenerated lock fails THIS gate, not the deploy.
cargo test --locked

echo "== release build =="
cargo build --release --locked

echo "== ok =="
