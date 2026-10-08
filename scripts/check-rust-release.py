#!/usr/bin/env python3
"""Fail closed on drift in the Rust source-release CI and retirement contract."""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
WDBX_REVISION = "7ddeb3d1389ec0e9fab5ac5397acf0da91a01174"

# Deliberately validate the complete small expression, not isolated substrings.
# A changed expression requires review and matching truth-table tests.
TRUST_CONDITION = """github.repository == 'donaldfilimon/abbey-bot' &&
(github.event_name == 'push' ||
(github.event_name == 'pull_request' &&
github.event.pull_request.head.repo.full_name == github.repository))"""

def trust_condition(code: str) -> str:
    match = re.search(r"^    if: >\n((?:      .*\n)+)", code, re.M)
    return " ".join(match.group(1).split()) if match else ""

def prerequisite_script(workflow: str) -> str:
    match = re.search(
        r"^      - name: Check host prerequisites\n        run: \|\n"
        r"((?:          .*\n)+)", workflow, re.M)
    return "".join(line[10:] + "\n" for line in match.group(1).splitlines()) if match else ""

def errors(root: Path) -> list[str]:
    problems = []
    workflow = (root / ".github/workflows/rust.yml").read_text()
    code = "\n".join(line for line in workflow.splitlines() if not line.lstrip().startswith("#"))
    if len(re.findall(r'^    if:', code, re.M)) != 1:
        problems.append('self-hosted job must have exactly one trust condition')
    if trust_condition(code) != " ".join(TRUST_CONDITION.split()):
        problems.append('self-hosted trust condition must match the reviewed complete expression')
    if not re.search(r'^jobs:\n  gate-macos:\n    name: Gate \(macOS\)\n', code, re.M):
        problems.append('preserve gate-macos job id and Gate (macOS) check name')
    if re.findall(r'^  ([\w-]+):$', code.split('jobs:', 1)[-1], re.M) != ['gate-macos']:
        problems.append('unexpected CI job; review its trust boundary')
    triggers = code.split('on:', 1)[-1].split('permissions:', 1)[0].strip()
    if triggers != 'push:\n    branches: ["main"]\n  pull_request:\n    branches: ["main"]':
        problems.append('unexpected workflow trigger; review event admission')
    if not re.search(r'^permissions:\n  contents: read\n', code, re.M):
        problems.append('workflow token must remain contents: read')
    label_path = root / '.github/actionlint.yaml'
    labels = label_path.read_text() if label_path.is_file() else ''
    labels = "\n".join(line for line in labels.splitlines() if not line.lstrip().startswith('#'))
    if labels.strip() != 'self-hosted-runner:\n  labels:\n    - abbey-bot':
        problems.append('actionlint must recognize exactly the abbey-bot custom runner label')
    preflight = prerequisite_script(workflow)
    if not preflight.startswith('set -eu\n') or 'for tool in rustup cargo python3 plutil xcrun; do' not in preflight:
        problems.append('host prerequisite step must fail fast and check every required tool')
    install = workflow.find('- name: Install pinned cargo-audit')
    gate = workflow.find('run: ./check.sh')
    if not preflight or install < 0 or gate < install or workflow.find('- name: Check host prerequisites') > install:
        problems.append('host prerequisites must precede installation and the gate')
    required = [
        'ABBEY_REQUIRE_WDBX_CONFORMANCE: "1"',
        'ABBEY_WDBX_REPO: ${{ github.workspace }}/wdbx',
        'repository: donaldfilimon/wdbx',
        f'ref: {WDBX_REVISION}',
        'path: wdbx', 'path: abbey-bot', 'working-directory: abbey-bot',
        'runs-on: [self-hosted, macOS, ARM64, abbey-bot]',
        "github.repository == 'donaldfilimon/abbey-bot' &&",
        'github.event.pull_request.head.repo.full_name == github.repository',
        'echo "CARGO_TARGET_DIR=$RUNNER_TEMP/abbey-rust-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}" >> "$GITHUB_ENV"',
        'run: ./check.sh',
    ]
    for value in required:
        if value not in code:
            problems.append(f"missing required CI contract: {value}")
    if re.search(r'CARGO_TARGET_DIR:\s*\$\{\{\s*runner\.', code):
        problems.append('runner context is unavailable in job-level env; set the target at runtime')
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
