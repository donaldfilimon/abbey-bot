# Self-hosted macOS runner

The `Gate (macOS)` check (job `gate-macos` in `.github/workflows/rust.yml`) and the `Gate (zig)` check (job `gate` in `.github/workflows/zig.yml`, only when `zig/` changes) run on a macOS arm64 runner registered to this repository. GitHub-hosted jobs cannot start while the account's Actions billing is locked (they fail in about two seconds with zero steps), so the workflows have none; self-hosted jobs still run.

## Registration

| Field | Value |
|-------|-------|
| Labels | `self-hosted`, `macOS`, `ARM64`, `abbey-bot` |
| Register at | [Settings → Actions → Runners → New self-hosted runner](https://github.com/donaldfilimon/abbey-bot/settings/actions/runners/new?arch=arm64) (macOS, ARM64) |

A runner is registered to one repository. If the same Mac already runs a runner for another repository (for example `abi`), install a second runner in its own directory, such as `~/actions-runner-abbey-bot`: run `./config.sh` with this repository's URL and token, add the custom label `abbey-bot` when asked, then `./svc.sh install && ./svc.sh start`.

Until a runner with these labels is online, same-repository `Gate (macOS)` and `Gate (zig)` jobs wait in the queue.

## Host requirements

`check.sh` runs the whole gate: `cargo fmt`, the Python deployment and privacy checks, `plutil -lint`, the offline audio-tap Swift package (`scripts/check-audio-tap.sh`), Clippy, tests and the locked release build.

- **rustup** (`~/.cargo/bin`). The pinned Rust 1.98.0 with `rustfmt` and `clippy` comes from `rust-toolchain.toml` on the first Cargo call. The job runs `cargo install cargo-audit --version 0.22.2 --locked`, which is a no-op once that version is in `~/.cargo/bin`.
- **Python 3.11 or newer** as `python3`. Several gate scripts import `tomllib`, so the Xcode Command Line Tools Python (3.9) is not enough; install one with Homebrew (`brew install python`).
- **Xcode** as the selected developer directory (`xcode-select -p`). The audio-tap gate runs `xcrun swift test` and a release build against the ScreenCaptureKit SDK (package minimum macOS 14). `plutil` ships with macOS.
- The runner service reads `PATH` from the `.path` file written by `./config.sh`. Make sure `~/.cargo/bin` and the Homebrew `bin` directory are in that `PATH` before you configure the runner, or edit `.path` and restart the service.

The first step of the job checks these tools and fails with a clear message if one is missing. No step uses `sudo`.

`Gate (zig)` additionally needs `curl`, `tar` and `sed` (all ship with macOS). It downloads the Zig master pinned in `zig/build.zig.zon` into the job's `RUNNER_TEMP` on every run, so nothing is installed on the host.

## Security

This repository is public, so the self-hosted jobs run only for `push` to `main` and for pull requests whose head branch is in this repository (`github.event.pull_request.head.repo.full_name == github.repository`), and only in `donaldfilimon/abbey-bot` itself. Fork pull requests get no job; run `./check.sh` locally on a fork's branch before merging it. The workflow has no `pull_request_target`, `issue_comment` or `workflow_run` trigger. The checkout uses `persist-credentials: false`, and the workflow token stays `contents: read`.

Where you can, run the runner as a dedicated macOS user rather than your daily account, and keep no production secrets (Discord tokens, `.env` files) readable by that user.

## Not covered

The GitHub-hosted jobs were removed from `.github/workflows/rust.yml` on 2026-09-28. Restore them from history if the billing lock is cleared or matching self-hosted runners exist:

- `Gate (Ubuntu)` (`ubuntu-24.04`) proved Linux behaviour, such as `/bin/sh` being dash and the Linux Rustls/WebPKI dependency tree; running it on macOS would report a Linux result it never measured.
- `Gate (Windows)` (`windows-2025`, `check.ps1`) needs a Windows host.
- `Gate (macOS) (GitHub-hosted, fork PRs)` (`macos-15`) built fork pull requests.
