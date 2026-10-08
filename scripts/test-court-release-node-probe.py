#!/usr/bin/env python3
"""Direct probe lifecycle tests using owned synthetic POSIX executables.

These are single directly spawned children, not process-group/descendant proof.
No Node, provider, service, production socket or user state is used.
"""
import contextlib
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    'court_release_probe_subject', Path(__file__).with_name('prepare-court-release.py'))
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


@unittest.skipUnless(os.name == 'posix', 'direct pipe/shebang lifecycle requires POSIX')
class NodeVersionProbeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='abbey-court-probe-')
        # macOS temp roots often enter through /var -> /private/var. Resolve
        # this owned fixture root before testing explicit symlink rejection.
        self.root = Path(self.temp.name).resolve()
        self.addCleanup(self.temp.cleanup)
        self.python = str(Path(sys.executable).resolve())
        if any(char in self.python for char in (' ', '\n', '\r')):
            self.skipTest('fixture interpreter path cannot be represented by a POSIX shebang')
        self.counter = 0

    def invoke(self, body, expected_error=None, parent_environment=None):
        self.counter += 1
        executable = self.root / f'node-probe-{self.counter}'
        marker = self.root / f'probe-pid-{self.counter}'
        program = (
            f'#!{self.python}\n'
            'import os, sys, time\n'
            'if sys.argv[1:] != ["--version"]:\n'
            '    os.write(1,b"unexpected argv\\n")\n'
            '    raise SystemExit(29)\n'
            f'with open({str(marker)!r},"w") as stream:\n'
            '    stream.write(str(os.getpid()))\n'
            + body + '\n'
        )
        executable.write_text(program)
        executable.chmod(0o700)
        original = executable.read_bytes()
        real_popen = subprocess.Popen
        children = []

        def spawn(*args, **kwargs):
            # This wraps the real OS spawn rather than replacing its outcome.
            self.assertEqual(args[0], [str(executable), '--version'])
            self.assertEqual(kwargs['stdin'], subprocess.DEVNULL)
            self.assertEqual(kwargs['stdout'], subprocess.PIPE)
            self.assertEqual(kwargs['stderr'], subprocess.DEVNULL)
            self.assertEqual(kwargs['env'], {'PATH': str(self.root)})
            child = real_popen(*args, **kwargs)
            children.append(child)
            return child

        stdout = io.StringIO()
        stderr = io.StringIO()
        result = None
        error = None
        observations = []
        started = time.monotonic()
        try:
            with patch.dict(os.environ, parent_environment or {}, clear=False):
                with patch.object(package.subprocess, 'Popen', side_effect=spawn):
                    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                        try:
                            result = package.inspect_node(executable)
                        except package.PackageError as caught:
                            error = caught
        finally:
            # Capture the subject's observed join before fixture fallback
            # cleanup. The fallback cannot turn an unjoined result into a pass.
            for child in children:
                already_reaped = False
                returned_status = child.returncode
                closed_output = child.stdout.closed
                if returned_status is not None:
                    try:
                        os.waitpid(child.pid, os.WNOHANG)
                    except ChildProcessError:
                        already_reaped = True
                observations.append((returned_status, closed_output, already_reaped))
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=3)
                child.stdout.close()
        elapsed = time.monotonic() - started
        self.assertEqual(len(children), 1, 'the actual direct probe child was spawned')
        self.assertTrue(marker.is_file(), 'the synthetic executable actually ran')
        self.assertEqual(int(marker.read_text()), children[0].pid)
        self.assertIsNotNone(observations[0][0], 'subject observed terminal child status before return')
        self.assertTrue(observations[0][1], 'subject closed its owned stdout pipe')
        self.assertTrue(observations[0][2], 'subject reaped its child before fixture cleanup')
        self.assertEqual(stdout.getvalue(), '', 'no dynamic probe output rendered by helper')
        self.assertEqual(stderr.getvalue(), '', 'no dynamic probe diagnostics rendered by helper')
        if expected_error is None:
            self.assertIsNone(error)
            self.assertEqual(result['path'], str(executable))
            self.assertEqual(result['version'], 'v26.10.0')
            self.assertEqual(result['bytes'], len(original))
            self.assertEqual(result['sha256'], hashlib.sha256(original).hexdigest())
        else:
            self.assertIsNone(result)
            self.assertIsInstance(error, package.PackageError)
            self.assertEqual(str(error), expected_error)
        return {
            'elapsed': elapsed, 'status': observations[0][0],
            'original': original, 'after': executable.read_bytes(),
        }

    def test_timeout_refusal_observes_killed_and_reaped_direct_child(self):
        observed = self.invoke('time.sleep(60)', 'node-version-unavailable')
        self.assertGreaterEqual(observed['elapsed'], 2.5, 'the three-second probe deadline was reached')
        self.assertLess(observed['elapsed'], 8, 'cleanup remained bounded after deadline')
        self.assertLess(observed['status'], 0, 'still-running fixture child was terminated')

    def test_oversized_stdout_refuses_before_timeout_and_reaps_child(self):
        observed = self.invoke(
            'os.write(1,b"x"*129)\ntime.sleep(60)', 'node-version-unavailable')
        self.assertLess(observed['elapsed'], 2.5, 'output bound fired before the separate three-second timeout')
        self.assertLess(observed['status'], 0, 'the output-producing fixture did not outlive refusal')

    def test_nonzero_exit_refuses_even_with_valid_version_and_suppresses_diagnostics(self):
        observed = self.invoke(
            'os.write(1,b"v26.10.0\\n")\n'
            'os.write(2,b"SYNTHETIC_DIAGNOSTIC_CANARY\\n")\n'
            'raise SystemExit(17)', 'node-version-unavailable')
        self.assertEqual(observed['status'], 17)
        self.assertEqual(observed['after'], observed['original'])

    def test_malformed_and_non_ascii_versions_refuse_after_observed_successful_exit(self):
        for body, reason in [
            ('os.write(1,b"node-version-v26.10.0\\n")', 'unstable-node'),
            ('os.write(1,b"\\xff\\n")', 'node-version-unavailable'),
            ('os.write(1,b"v26.10.0\\nextra\\n")', 'unstable-node'),
        ]:
            with self.subTest(reason=reason, body=body):
                observed = self.invoke(body, reason)
                self.assertEqual(observed['status'], 0)
                self.assertEqual(observed['after'], observed['original'])

    def test_executable_byte_drift_during_successful_probe_refuses_identity(self):
        observed = self.invoke(
            'with open(__file__,"ab") as stream:\n'
            '    stream.write(b"\\n# SYNTHETIC_EXECUTABLE_DRIFT\\n")\n'
            'os.write(1,b"v26.10.0\\n")', 'unstable-node')
        self.assertEqual(observed['status'], 0)
        self.assertNotEqual(observed['after'], observed['original'])
        self.assertTrue(observed['after'].endswith(b'# SYNTHETIC_EXECUTABLE_DRIFT\n'))

    def test_valid_probe_receives_eof_stdin_and_does_not_inherit_parent_environment(self):
        parent = {
            'ABBEY_COURT_PROBE_CANARY': 'SYNTHETIC_ENV_CANARY',
            'HTTP_PROXY': 'http://synthetic-proxy.invalid',
            'ABBEY_BACKEND': 'synthetic-disallowed-backend',
        }
        body = (
            'if any(key in os.environ for key in '
            '["ABBEY_COURT_PROBE_CANARY","HTTP_PROXY","ABBEY_BACKEND"]):\n'
            '    os.write(1,b"inherited parent environment\\n")\n'
            '    raise SystemExit(31)\n'
            'if os.read(0,1) != b"":\n'
            '    os.write(1,b"inherited stdin\\n")\n'
            '    raise SystemExit(32)\n'
            'os.write(1,b"v26.10.0\\n")'
        )
        observed = self.invoke(body, parent_environment=parent)
        self.assertEqual(observed['status'], 0)
        self.assertEqual(observed['after'], observed['original'])


if __name__ == '__main__':
    unittest.main()
