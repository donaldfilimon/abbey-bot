# Self-hosted macOS runner

The `Gate (macOS)` check (job `gate-macos` in `.github/workflows/rust.yml`) runs on the repository's macOS arm64 runner. It checks out Abbey and WDBX separately, pins WDBX to `7ddeb3d1389ec0e9fab5ac5397acf0da91a01174`, and requires cross-repository conformance. Each run uses a unique disposable Cargo target directory. GitHub-hosted lanes were removed while Actions billing was locked; their absence is not Linux or Windows acceptance.

## Registration

| Field | Value |
|-------|-------|
| Labels | `self-hosted`, `macOS`, `ARM64`, `abbey-bot` |
| Register at | [Settings → Actions → Runners → New self-hosted runner](https://github.com/donaldfilimon/abbey-bot/settings/actions/runners/new?arch=arm64) (macOS, ARM64) |

A runner is registered to one repository. If the same Mac already runs a runner for another repository (for example `abi`), install a second runner in its own directory, such as `~/actions-runner-abbey-bot`: run `./config.sh` with this repository's URL and token, add the custom label `abbey-bot` when asked, then `./svc.sh install && ./svc.sh start`.

Until a runner with these labels is online, same-repository `Gate (macOS)` jobs wait in the queue.

## Host requirements

`check.sh` runs the whole gate: `cargo fmt`, the Python deployment and privacy checks, `plutil -lint`, the offline audio-tap Swift package (`scripts/check-audio-tap.sh`), Clippy, tests and the locked release build.

- **rustup** (`~/.cargo/bin`). The pinned Rust 1.98.0 with `rustfmt` and `clippy` comes from `rust-toolchain.toml` on the first Cargo call. The job runs `cargo install cargo-audit --version 0.22.2 --locked`, which is a no-op once that version is in `~/.cargo/bin`.
- **Python 3.11 or newer** as `python3`. Several gate scripts import `tomllib`, so the Xcode Command Line Tools Python (3.9) is not enough; install one with Homebrew (`brew install python`).
- **Xcode** as the selected developer directory (`xcode-select -p`). The audio-tap gate runs `xcrun swift test` and a release build against the ScreenCaptureKit SDK (package minimum macOS 14). `plutil` ships with macOS.
- The runner service reads `PATH` from the `.path` file written by `./config.sh`. Make sure `~/.cargo/bin` and the Homebrew `bin` directory are in that `PATH` before you configure the runner, or edit `.path` and restart the service.

The prerequisite step checks these tools before installing cargo-audit or running the gate. It explicitly enables shell failure propagation, rejects Python below 3.11, and probes Rust and Swift with `TOOLCHAINS` removed from the Swift environment. No step uses `sudo`.


## Security

This repository is public, so the self-hosted jobs run only for `push` to `main` and for pull requests whose head branch is in this repository (`github.event.pull_request.head.repo.full_name == github.repository`), and only in `donaldfilimon/abbey-bot` itself. Fork pull requests get no job; run `./check.sh` locally on a fork's branch before merging it. The workflow has no `pull_request_target`, `issue_comment` or `workflow_run` trigger. The checkout uses `persist-credentials: false`, and the workflow token stays `contents: read`.

Where you can, run the runner as a dedicated macOS user rather than your daily account, and keep no production secrets (Discord tokens, `.env` files) readable by that user.

## Repository verification

`python3 scripts/test-check-rust-release.py` and
`python3 scripts/check-rust-release.py` run in both local gates. They protect the
complete reviewed job trust expression, main-only triggers, read-only token,
`gate-macos` job ID, `Gate (macOS)` check name, runner label tuple, and actionlint's
`abbey-bot` custom label. The trust truth table covers canonical and foreign
repositories, same-repository and fork heads, absent heads, and unsupported events.
Prerequisite tests execute the workflow's actual shell block against disposable
stub tools, including missing tools, Python 3.10, and failed Rust/Swift probes.
They require failure before a sentinel gate command, without contacting a runner.
The shell execution cases explicitly skip on hosts without `/bin/sh`; the
workflow contract and trust truth table still run there.
Run `actionlint .github/workflows/rust.yml` separately when actionlint is installed.

These source checks do not prove runner registration, host isolation, branch
protection settings, or that a particular SHA completed CI. A skipped fork job
is not gate acceptance. A PR can change workflow source, so this condition is
not an isolation boundary against arbitrary workflow edits; review those edits
and retain the host precautions in [GitHub's self-hosted runner guidance](https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/add-runners).

## Not covered

The GitHub-hosted jobs were removed from `.github/workflows/rust.yml` on 2026-09-28. Restore them from history if the billing lock is cleared or matching self-hosted runners exist:

- `Gate (Ubuntu)` (`ubuntu-24.04`) proved Linux behaviour, such as `/bin/sh` being dash and the Linux Rustls/WebPKI dependency tree; running it on macOS would report a Linux result it never measured.
- `Gate (Windows)` (`windows-2025`, `check.ps1`) needs a Windows host.
- `Gate (macOS) (GitHub-hosted, fork PRs)` (`macos-15`) built fork pull requests.
