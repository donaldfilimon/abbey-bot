#!/usr/bin/env python3
"""Fail closed on drift in the Rust source-release CI and retirement contract."""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
WDBX_REVISION = "9fee98ff5ccb92fa86a2ed44f93abd65e7e181ae"

def errors(root: Path) -> list[str]:
    problems = []
    workflow = (root / ".github/workflows/rust.yml").read_text()
    code = "\n".join(line for line in workflow.splitlines() if not line.lstrip().startswith("#"))
    required = [
        'ABBEY_REQUIRE_WDBX_CONFORMANCE: "1"',
        'ABBEY_WDBX_REPO: ${{ github.workspace }}/wdbx',
        'repository: donaldfilimon/wdbx',
        f'ref: {WDBX_REVISION}',
        'path: wdbx', 'path: abbey-bot', 'working-directory: abbey-bot',
        'runs-on: [self-hosted, macOS, ARM64, abbey-bot]',
        "github.repository == 'donaldfilimon/abbey-bot' &&",
        'github.event.pull_request.head.repo.full_name == github.repository',
        'CARGO_TARGET_DIR: ${{ runner.temp }}/abbey-rust-${{ github.run_id }}-${{ github.run_attempt }}',
        'run: ./check.sh',
    ]
    for value in required:
        if value not in code:
            problems.append(f"missing required CI contract: {value}")
    if code.count('persist-credentials: false') != 2 or re.search(r'persist-credentials:\s*true', code):
        problems.append('both checkouts must disable persisted credentials')
    if re.search(r'continue-on-error:\s*true|pull_request_target:|workflow_run:', code):
        problems.append('release gate may not ignore failure or admit indirect untrusted events')
    for relative in ('zig', '.github/workflows/zig.yml'):
        if (root / relative).exists():
            problems.append(f'retired source/build lane returned: {relative}')
    for relative in ('check.sh', 'check.ps1', 'Cargo.toml', 'Dockerfile', '.dockerignore'):
        if re.search(r'\bzig\b', (root / relative).read_text(), re.I):
            problems.append(f'active build references retired toolchain: {relative}')
    return problems

def main() -> int:
    problems = errors(ROOT)
    if problems:
        print('\n'.join(problems), file=sys.stderr)
        return 1
    print(f'Rust-only build and strict pinned WDBX CI contract: {WDBX_REVISION}')
    return 0

if __name__ == '__main__':
    raise SystemExit(main())
