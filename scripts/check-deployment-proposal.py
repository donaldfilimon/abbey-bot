#!/usr/bin/env python3
"""Validate the dormant JSON-subset YAML proposal without a YAML dependency."""
import json
from pathlib import Path
import sys
ROOT = Path(__file__).resolve().parent.parent
TEMPLATE = 'deploy/ci/rust-auto-deploy.yml'
GATE = "github.repository == 'donaldfilimon/abbey-bot' && ((github.ref == 'refs/heads/main' && (github.event_name == 'push' || github.event_name == 'workflow_dispatch')) || (github.event_name == 'pull_request' && github.event.pull_request.head.repo.full_name == github.repository))"
DEPLOY = "github.repository == 'donaldfilimon/abbey-bot' && github.ref == 'refs/heads/main' && (github.event_name == 'push' || github.event_name == 'workflow_dispatch') && needs.gate-macos.result == 'success'"

# Pin the reviewed commands, including their failure-propagating shell bodies.
HOST_PREREQUISITES = {'name': 'Check host prerequisites', 'run': 'set -eu\nfor tool in rustup cargo python3 plutil xcrun; do\n  command -v "$tool" >/dev/null 2>&1 || exit 1\ndone\npython3 -c \'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)\'\nenv -u TOOLCHAINS xcrun swift --version'}
AUDIT_INSTALL = {'name': 'Install pinned cargo-audit report tool', 'run': 'cargo install cargo-audit --version 0.22.2 --locked'}
SOURCE_GATE = {'name': 'Strict source gate', 'run': './check.sh'}
QUALIFICATION = {'name': 'Qualify the exact gated candidate with synthetic FM probes', 'run': 'set -eu\numask 077\nmkdir -p "$CARGO_TARGET_DIR/qualification"\nchmod 700 "$CARGO_TARGET_DIR/qualification"\nenv -i HOME="$HOME" PATH="$PATH" ABBEY_FM_MODE=pcc,system ABBEY_FM_ROLE=primary ABBEY_FM_CLI=/usr/bin/fm python3 -I deploy/publish-provider-qualification.py --binary "$CARGO_TARGET_DIR/release/abbey-bot" --target fm --output "$CARGO_TARGET_DIR/qualification/fm.json"'}
QUALIFIED_INSTALL = {'name': 'Publish binary and qualification in one verified install transaction', 'run': './deploy/install-launchd.sh --qualified-candidate "$CARGO_TARGET_DIR/release/abbey-bot" "$CARGO_TARGET_DIR/qualification/fm.json" "$GITHUB_SHA"'}

def required_step(steps, expected, job_name, issues):
    matches = [i for i, step in enumerate(steps) if step.get('name') == expected['name']]
    commands = [i for i, step in enumerate(steps) if step.get('run') == expected['run']]
    if len(matches) != 1 or commands != matches:
        issues.append('critical step must occur exactly once with pinned command: '
                      + job_name + ': ' + expected['name'])
        return None
    index = matches[0]
    # Exact metadata prevents conditional execution, ignored errors, and shell,
    # environment or directory overrides that could bypass normal propagation.
    if steps[index] != expected:
        issues.append('critical step execution metadata drift: '
                      + job_name + ': ' + expected['name'])
    return index

def errors(root):
    issues = []
    try:
        value = json.loads((root/TEMPLATE).read_text())
        if (root/'.github/workflows/rust-auto-deploy.yml').exists(): issues.append('deployment proposal must remain dormant')
        if value['on'] != {'push':{'branches':['main']},'pull_request':{'branches':['main']},'workflow_dispatch':{}}: issues.append('trigger admission drift')
        if value['permissions'] != {'contents':'read'}: issues.append('token permissions drift')
        if value['concurrency'] != {'group':'rust-deployment-${{ github.workflow }}-${{ github.ref }}','cancel-in-progress':False}: issues.append('workflow cancellation drift')
        jobs = value['jobs']
        if set(jobs) != {'gate-macos','deploy-macos'}: issues.append('job inventory drift')
        for name, condition in [('gate-macos',GATE),('deploy-macos',DEPLOY)]:
            job = jobs[name]
            if job['if'] != condition: issues.append('complete trust expression drift: '+name)
            if job['runs-on'] != ['self-hosted','macOS','ARM64','abbey-bot']: issues.append('runner label drift')
            if job.get('continue-on-error',False): issues.append('gate failure bypass')
            if job['env'] != {'ABBEY_REQUIRE_WDBX_CONFORMANCE':'1','ABBEY_WDBX_REPO':'${{ github.workspace }}/wdbx'}: issues.append('strict conformance drift')
            if job['defaults'] != {'run':{'working-directory':'abbey-bot','shell':'sh'}}: issues.append('shell/directory drift')
            steps = job['steps']
            prerequisite = required_step(steps, HOST_PREREQUISITES, name, issues)
            audit = required_step(steps, AUDIT_INSTALL, name, issues)
            gate = required_step(steps, SOURCE_GATE, name, issues)
            for index in (prerequisite, audit):
                if index is not None and gate is not None and index >= gate:
                    issues.append('host prerequisites must precede source gate: ' + name)
            if name == 'deploy-macos':
                qualification = required_step(steps, QUALIFICATION, name, issues)
                publish = required_step(steps, QUALIFIED_INSTALL, name, issues)
                if gate is not None and qualification is not None and gate >= qualification:
                    issues.append('qualification must follow source gate')
                if qualification is not None and publish is not None and qualification >= publish:
                    issues.append('qualified installation must follow qualification')
            for step in steps:
                if step.get('run') == './check.sh' or 'uses' in step or '--qualified-candidate' in step.get('run', '') or '--target fm' in step.get('run', ''):
                    if 'if' in step or step.get('continue-on-error', False):
                        issues.append('critical step may not skip or ignore failure')
            checkouts = [s for s in steps if 'uses' in s]
            if len(checkouts) != 2 or any(s['uses'] != 'actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1' or s['with'].get('persist-credentials') is not False for s in checkouts): issues.append('checkout provenance drift')
            if checkouts[0]['with'].get('ref') != '${{ github.sha }}' or checkouts[1]['with'].get('ref') != '9fee98ff5ccb92fa86a2ed44f93abd65e7e181ae': issues.append('exact checkout revision drift')
        if jobs['gate-macos']['name'] != 'Gate (macOS)': issues.append('required check name drift')
        deploy = jobs['deploy-macos']
        if deploy['needs'] != 'gate-macos': issues.append('prerequisite drift')
        if deploy['concurrency'] != {'group':'abbey-bot-production-deployment','cancel-in-progress':False}: issues.append('deployment serialization drift')
    except (KeyError, ValueError, TypeError, IndexError, OSError):
        issues.append('malformed deployment proposal')
    return issues

def main():
    issues=errors(ROOT)
    if issues:
        print('\n'.join(issues),file=sys.stderr);return 1
    print('Dormant deployment proposal: trust, prerequisites, serialization, provenance verified');return 0
if __name__ == '__main__': raise SystemExit(main())
