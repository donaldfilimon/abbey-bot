#!/usr/bin/env python3
"""Mutations must not turn missing dependencies or retired lanes into green CI."""
import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys
import shlex
import tempfile
import unittest
ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('release', ROOT/'scripts/check-rust-release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
class ReleaseTests(unittest.TestCase):
    def fixture(self, root):
        for relative in ['.github/actionlint.yaml','.github/workflows/rust.yml','check.sh','check.ps1','Cargo.toml','Dockerfile','.dockerignore']:
            path=root/relative;path.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/relative,path)
    def test_current_tree(self):
        self.assertEqual(release.errors(ROOT),[])
    def test_missing_moving_optional_or_untrusted_contract_fails(self):
        mutations=[
            ('ABBEY_REQUIRE_WDBX_CONFORMANCE: "1"','ABBEY_REQUIRE_WDBX_CONFORMANCE: "0"'),
            (release.WDBX_REVISION,'main'),
            ('path: wdbx','path: unrelated'),
            ('working-directory: abbey-bot','working-directory: .'),
            ('github.event.pull_request.head.repo.full_name == github.repository','true'),
            ('run: ./check.sh','run: echo skipped'),
            ('ABBEY_WDBX_REPO:', 'CARGO_TARGET_DIR: ${{ runner.temp }}/invalid\n      ABBEY_WDBX_REPO:'),
            ('persist-credentials: false','persist-credentials: true'),
            ('Gate (macOS)', 'Gate (Mac)'),
            ('gate-macos:', 'gate-renamed:'),
            ('ARM64, abbey-bot', 'ARM64, unrelated'),
            ("github.repository == 'donaldfilimon/abbey-bot' &&", "github.repository == 'donaldfilimon/abbey-bot' ||"),
            ("github.event_name == 'push' ||", "true ||"),
            ('contents: read', 'contents: write'),
            ('set -eu', 'set +e'),
            ('    runs-on:', '    if: true\n    runs-on:'),
            ('permissions:\n', '  workflow_dispatch:\n\npermissions:\n'),
            ('jobs:\n', 'jobs:\n  unsafe:\n    runs-on: self-hosted\n'),
            ('branches: ["main"]', 'branches: ["other"]'),
            ('timeout-minutes: 60','timeout-minutes: 60\n    continue-on-error: true'),
        ]
        for old,new in mutations:
            with self.subTest(old=old),tempfile.TemporaryDirectory() as temp:
                root=Path(temp);self.fixture(root);path=root/'.github/workflows/rust.yml'
                path.write_text(path.read_text().replace(old,new))
                self.assertTrue(release.errors(root))
    def test_trust_truth_table(self):
        condition = release.trust_condition((ROOT/'.github/workflows/rust.yml').read_text())
        self.assertEqual(condition, " ".join(release.TRUST_CONDITION.split()))
        for repo in ['donaldfilimon/abbey-bot', 'outsider/abbey-bot']:
            for event in ['push', 'pull_request', 'pull_request_target', 'workflow_run', 'workflow_dispatch', 'issue_comment']:
                for head in ['donaldfilimon/abbey-bot', 'outsider/abbey-bot', '']:
                    with self.subTest(repo=repo, event=event, head=head):
                        # Only evaluate the validated fixed expression, with literal data.
                        expression = condition.replace('github.event.pull_request.head.repo.full_name', repr(head))
                        expression = expression.replace('github.repository', repr(repo)).replace('github.event_name', repr(event))
                        expression = expression.replace('&&', ' and ').replace('||', ' or ')
                        expected = repo == 'donaldfilimon/abbey-bot' and (event == 'push' or (event == 'pull_request' and head == repo))
                        self.assertEqual(eval(expression, {'__builtins__': {}}), expected)

    def test_custom_label_drift(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); self.fixture(root)
            (root/'.github/actionlint.yaml').write_text('self-hosted-runner:\n  labels:\n    - other\n')
            self.assertTrue(release.errors(root))
            (root/'.github/actionlint.yaml').unlink()
            self.assertTrue(release.errors(root))

    @unittest.skipUnless(Path("/bin/sh").is_file(), "POSIX workflow shell requires /bin/sh")
    def test_prerequisites_without_a_runner(self):
        script = release.prerequisite_script((ROOT/'.github/workflows/rust.yml').read_text())
        tools = ['rustup', 'cargo', 'python3', 'plutil', 'xcrun', 'env']
        cases = [(None, None), (None, 'old-python')] + [(tool, None) for tool in tools[:-1]] + [(None, tool) for tool in ['python3', 'rustup', 'xcrun']]
        for missing, failing in cases:
            with self.subTest(missing=missing, failing=failing), tempfile.TemporaryDirectory() as temp:
                root = Path(temp); log = root/'calls'
                for tool in tools:
                    if tool == missing:
                        continue
                    stub = root/tool
                    body = '#!/bin/sh\necho "' + tool + ':$*" >> "$CALL_LOG"\n'
                    if tool == 'env':
                        body += 'shift 2\nexec "$@"\n'
                    elif tool == 'python3' and failing == 'old-python':
                        body += "exec " + shlex.quote(sys.executable) + " -c " + shlex.quote("import sys; sys.version_info = (3, 10); exec(sys.argv[1])") + ' "$2"\n'
                    elif tool == failing:
                        body += 'echo "synthetic prerequisite failure" >&2\nexit 1\n'
                    stub.write_text(body); stub.chmod(0o755)
                result = subprocess.run(['/bin/sh', '-c', script + '\necho GATE_REACHED\n'], env={'PATH': temp, 'CALL_LOG': str(log), 'TOOLCHAINS': 'poison'}, text=True, capture_output=True)
                success = missing is None and failing is None
                self.assertEqual(result.returncode == 0, success, result.stderr)
                self.assertEqual('GATE_REACHED' in result.stdout, success)
                if missing:
                    self.assertIn('missing ' + missing, result.stdout)
                if failing == 'old-python':
                    self.assertIn('python3 must be 3.11 or newer', result.stderr)
                if success:
                    self.assertIn('env:-u TOOLCHAINS xcrun swift --version', log.read_text())
                    self.assertIn('sys.version_info >= (3, 11)', log.read_text())

    def test_retired_lane_or_build_reference_fails(self):
        for relative in ['zig/build.zig','.github/workflows/zig.yml','check.sh','Dockerfile']:
            with self.subTest(path=relative),tempfile.TemporaryDirectory() as temp:
                root=Path(temp);self.fixture(root);path=root/relative;path.parent.mkdir(parents=True,exist_ok=True)
                path.write_text('zig build\n')
                self.assertTrue(release.errors(root))
if __name__=='__main__':unittest.main()
