#!/usr/bin/env python3
"""Offline readiness acceptance. No real launchd, HOME reads, or PID probes."""
import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

import service_readiness as r
from service_protocol import Failure, ProtocolError

DIGEST = "a" * 64
NONCE = "b" * 64
OLD = "c" * 64


class Clock:
    def __init__(self):
        self.ns = 0
        self.sleeps = []

    def monotonic(self):
        return self.ns

    def sleep(self, seconds):
        self.sleeps.append(seconds)
        self.ns += round(seconds * 1e9)

    def wall(self):
        return 100000 + self.ns // 1000000


def document(clock, **changes):
    value = {"schema_version": 1, "pid": 4242, "run_nonce": NONCE,
             "executable_sha256": DIGEST, "phase": "ready",
             "published_at_unix_ms": clock.wall(), "discord": "ready",
             "scheduler": "running", "telegram": "disabled", "slack": "degraded",
             "last_persistence": "complete"}
    value.update(changes)
    return value


class Tests(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.context = r.TransactionContext(100000, r.BUDGET_NS, (OLD,))
        self.pid = Mock(return_value=4242)
        self.alive = Mock(return_value=True)

    def run_wait(self, reader=None, **kwargs):
        return r.wait_ready(self.context, 4242, DIGEST, entry_ns=0,
                            monotonic=self.clock.monotonic, wall_ms=self.clock.wall,
                            sleep=self.clock.sleep, current_pid=self.pid,
                            alive=self.alive, read_document=reader or (lambda: document(self.clock)),
                            **kwargs)

    def fails(self, call, code=None):
        with self.assertRaises(r.ReadinessError) as raised:
            call()
        if code:
            self.assertEqual(raised.exception.code, code)
        self.assertNotIn(NONCE, repr(raised.exception))

    def test_context_roundtrip_and_private_repr(self):
        raw = r.encode_context(self.context)
        self.assertEqual(r.parse_context(raw, 100000), self.context)
        self.assertLessEqual(len(raw), 512)
        self.assertNotIn(OLD, repr(self.context))
        self.assertEqual(r.make_context((OLD,), wall_ms=self.clock.wall,
                                        monotonic=self.clock.monotonic), self.context)

    def test_context_strict_matrix(self):
        valid = json.loads(r.encode_context(self.context))
        for key in valid:
            missing = dict(valid); del missing[key]
            self.fails(lambda: r.parse_context((json.dumps(missing) + '\n').encode(), 100000))
        invalid = [b'', b'{}\n', b'null\n', b'\xff\n', b' ' * 513,
                   r.encode_context(self.context).rstrip(),
                   r.encode_context(self.context) + b'\n',
                   r.encode_context(self.context).replace(b'"schema_version":1',
                       b'"schema_version":1,"schema_version":1')]
        for key in ("schema_version", "transaction_start_ms", "deadline_monotonic_ns"):
            for value in (None, True, 1.0, "1", -1, 2**63):
                changed = dict(valid); changed[key] = value
                invalid.append((json.dumps(changed, separators=(',', ':')) + '\n').encode())
        for value in (None, {}, [OLD, OLD], [OLD, NONCE, DIGEST], [OLD.upper()], [True]):
            changed = dict(valid); changed['excluded_nonces'] = value
            invalid.append((json.dumps(changed, separators=(',', ':')) + '\n').encode())
        for raw in invalid:
            with self.subTest(raw_length=len(raw)):
                self.fails(lambda: r.parse_context(raw, 100000), r.FailureCode.CONTEXT)

    def test_arguments_exact_no_abbreviations_or_duplicates(self):
        valid = ['--transaction-start-ms', '100000', '--expected-sha256', DIGEST,
                 '--launchd-pid', '4242']
        self.assertEqual(r.parse_arguments(valid), (100000, DIGEST, 4242))
        invalid = [[], valid + ['--help'], valid[:-1], valid[:2] * 3,
                   ['--transaction-start', *valid[1:]]]
        for position, values in ((1, ['+1', '-1', '01', '1.0', str(2**63)]),
                                 (3, ['A'*64, 'x']), (5, ['0', '01', str(2**31)])):
            for value in values:
                changed = list(valid); changed[position] = value; invalid.append(changed)
        for args in invalid:
            self.fails(lambda: r.parse_arguments(args), r.FailureCode.USAGE)

    def test_context_input_requires_eof_and_deadline(self):
        raw = r.encode_context(self.context)
        chunks = iter([raw[:10], raw[10:], b''])
        context = r.read_context(99, 100000, 0, monotonic=self.clock.monotonic,
                                 wait=lambda *args: ([99], [], []), read=lambda *args: next(chunks))
        self.assertEqual(context, self.context)
        def wait(*args):
            self.clock.sleep(args[-1]); return ([], [], [])
        self.fails(lambda: r.read_context(99, 100000, 0, monotonic=self.clock.monotonic,
                                          wait=wait, read=lambda *args: b''), r.FailureCode.TIMEOUT)
        self.assertEqual(self.clock.ns, r.BUDGET_NS)

    def test_context_input_honors_passed_deadline_without_eof(self):
        raw = r.encode_context(r.TransactionContext(100000, 1_000_000_000, ()))
        calls = 0
        def wait(*args):
            nonlocal calls
            calls += 1
            if calls == 1:
                return ([99], [], [])
            self.clock.sleep(args[-1]); return ([], [], [])
        self.fails(lambda: r.read_context(99, 100000, 0, monotonic=self.clock.monotonic,
                                          wait=wait, read=lambda *args: raw), r.FailureCode.TIMEOUT)
        self.assertEqual(self.clock.ns, 1_000_000_000)

    def test_context_input_oversize_and_empty(self):
        for raw in (b'', b'a'*513):
            self.fails(lambda: r.read_context(99, 100000, 0, monotonic=lambda: 0,
                      wait=lambda *args: ([99], [], []), read=lambda *args: raw), r.FailureCode.CONTEXT)

    def test_baseline_verified_absence_and_unreadable(self):
        self.assertEqual(r.capture_prior_nonces(Path('/fake'), reader=lambda _: None), ())
        self.assertEqual(r.capture_prior_nonces(Path('/fake'),
                         reader=lambda _: document(self.clock, phase='draining')), (NONCE,))
        def unsafe(_):
            raise ProtocolError(Failure.UNSAFE_FILE)
        with self.assertRaises(ProtocolError):
            r.capture_prior_nonces(Path('/fake'), reader=unsafe)

    def test_pid_parser_requires_exact_root_and_single_top_level_pid(self):
        root = b'gui/501/com.donaldfilimon.abbey-bot = {\n'
        self.assertEqual(r.parse_pid_record(root + b'\tpid = 4242\n}\n', 501), 4242)
        self.assertEqual(r.parse_pid_record(root + b'\tnested = {\n\t\tpid = 7\n\t}\n\tpid = 4242\n}\n', 501), 4242)
        for body in (b'pid = 0\n}', b'pid = 01\n}', b'pid = 2147483648\n}',
                     b'pid = 2\npid = 3\n}', b'pid = 2', b'nested = {\npid = 2\n}\n}',
                     b'pid = 2\n}\nextra', b'pid = +2\n}', b'pid = 2\npid=3\n}'):
            body = b'\n'.join(b'\t' + line if line != b'}' else line for line in body.split(b'\n'))
            self.fails(lambda: r.parse_pid_record(root + body, 501), r.FailureCode.LAUNCHD)
        self.fails(lambda: r.parse_pid_record(root + b'pid = 4242\n}', 501))
        self.fails(lambda: r.parse_pid_record(root + b'\tpid = 2\n}', 502))
        self.fails(lambda: r.parse_pid_record(b'x' * (r.STATUS_CAP + 1), 501))

    def test_success_five_seconds_two_pid_samples_each_and_optional_updates(self):
        def reader():
            return document(self.clock, telegram='degraded', last_persistence='partial')
        self.run_wait(reader)
        self.assertEqual(self.clock.ns, r.STABILITY_NS)
        self.assertEqual(self.pid.call_count, 42)
        self.assertEqual(self.alive.call_count, 42)
        self.assertTrue(all(s <= .25 for s in self.clock.sleeps))

    def test_pre_ready_wait_consumes_same_budget(self):
        def reader():
            return None if self.clock.ns < 26_000_000_000 else document(self.clock)
        self.fails(lambda: self.run_wait(reader), r.FailureCode.TIMEOUT)
        self.assertEqual(self.clock.ns, r.BUDGET_NS)

    def test_bootstrap_elapsed_and_supplied_long_deadline_do_not_reset(self):
        self.clock.ns = 26_000_000_000
        self.context = r.TransactionContext(100000, 100_000_000_000, ())
        self.fails(self.run_wait, r.FailureCode.TIMEOUT)
        self.assertEqual(self.clock.ns, r.BUDGET_NS)

    def test_final_sample_exactly_at_deadline_is_allowed(self):
        self.run_wait(lambda: None if self.clock.ns < 25_000_000_000 else document(self.clock))
        self.assertEqual(self.clock.ns, r.BUDGET_NS)

    def test_liveness_and_two_identity_samples_also_cover_starting(self):
        self.run_wait(lambda: None if self.clock.ns < 1_000_000_000 else document(self.clock))
        self.assertEqual(self.pid.call_count, self.alive.call_count)
        self.assertEqual(self.clock.ns, 6_000_000_000)

    def test_expired_context_never_reads(self):
        self.clock.ns = r.BUDGET_NS
        self.fails(self.run_wait, r.FailureCode.TIMEOUT)
        self.pid.assert_not_called()

    def test_required_changes_and_missing_fail_during_stability(self):
        for changes in ({'phase': 'starting'}, {'scheduler': 'stopped'}, {'discord': 'connecting'},
                        {'pid': 4243}, {'run_nonce': DIGEST}, {'executable_sha256': OLD},
                        {'published_at_unix_ms': 99999}, {'published_at_unix_ms': 9999999}):
            self.clock = Clock()
            self.fails(lambda: self.run_wait(lambda: document(self.clock, **changes)
                                            if self.clock.ns else document(self.clock)))
            self.assertEqual(self.clock.ns, r.POLL_NS)
        self.clock = Clock()
        self.fails(lambda: self.run_wait(lambda: None if self.clock.ns else document(self.clock)))

    def test_reused_nonce_dead_pid_and_launchd_replacement(self):
        self.fails(lambda: self.run_wait(lambda: document(self.clock, run_nonce=OLD)), r.FailureCode.IDENTITY)
        self.alive.return_value = False
        self.fails(self.run_wait, r.FailureCode.DOCUMENT)
        self.alive.return_value = True
        self.pid.side_effect = [4242, 4243]
        self.fails(self.run_wait, r.FailureCode.IDENTITY)

    def test_trailing_identity_query_cannot_age_final_sample_into_false_success(self):
        self.context = r.TransactionContext(0, r.BUDGET_NS, ())
        calls = 0
        def current_pid(_deadline):
            nonlocal calls
            calls += 1
            if calls == 42:
                self.clock.ns += 2_000_000
            return 4242
        self.pid = current_pid
        def reader():
            changes = {'published_at_unix_ms': self.clock.wall() - 29999} if self.clock.ns >= r.STABILITY_NS else {}
            return document(self.clock, **changes)
        self.fails(lambda: self.run_wait(reader), r.FailureCode.DOCUMENT)
        self.assertEqual(self.clock.ns, r.STABILITY_NS + 2_000_000)

    def test_slow_validation_cannot_pass_after_deadline(self):
        def reader():
            self.clock.ns += 31_000_000_000
            return document(self.clock)
        self.fails(lambda: self.run_wait(reader), r.FailureCode.TIMEOUT)

    def test_unsafe_or_malformed_never_retried(self):
        for category in (Failure.UNSAFE_FILE, Failure.INVALID_DOCUMENT, Failure.UNAVAILABLE):
            def reader():
                raise ProtocolError(category)
            self.fails(lambda: self.run_wait(reader), r.FailureCode.DOCUMENT)
            self.assertEqual(self.clock.ns, 0)

    def test_launchctl_bounded_capture_and_owned_timeout_cleanup(self):
        child = Mock()
        child.stdout.fileno.return_value = 90
        child.poll.return_value = 0
        child.wait.return_value = 0
        raw = b'gui/501/com.donaldfilimon.abbey-bot = {\n\tpid = 4242\n}\n'
        with patch.object(r.subprocess, 'Popen', return_value=child) as spawn, \
             patch.object(r.select, 'select', return_value=([child.stdout], [], [])), \
             patch.object(r.os, 'read', side_effect=[raw, b'']):
            self.assertEqual(r.launchd_pid(r.BUDGET_NS, monotonic=lambda: 0, uid=501), 4242)
            self.assertEqual(spawn.call_args.args[0], ['/bin/launchctl', 'print',
                             'gui/501/com.donaldfilimon.abbey-bot'])
            child.stdout.close.assert_called_once()
        child.reset_mock()
        child.poll.return_value = None
        with patch.object(r.subprocess, 'Popen', return_value=child), \
             patch.object(r.select, 'select', return_value=([], [], [])):
            self.fails(lambda: r.launchd_pid(r.BUDGET_NS, monotonic=lambda: 0, uid=501), r.FailureCode.TIMEOUT)
            child.kill.assert_called_once()
            child.wait.assert_called_once()
            self.assertLessEqual(child.wait.call_args.kwargs["timeout"], 2)

    def test_failed_child_cleanup_retains_owner_and_never_reports_success(self):
        child = Mock()
        child.poll.return_value = None
        child.wait.side_effect = subprocess.TimeoutExpired('private-canary', .1)
        before = len(r._OUTSTANDING_CHILDREN)
        with patch.object(r.subprocess, 'Popen', return_value=child), \
             patch.object(r.select, 'select', return_value=([], [], [])):
            self.fails(lambda: r.launchd_pid(r.BUDGET_NS, monotonic=lambda: 0, uid=501), r.FailureCode.CLEANUP)
        self.assertIs(r._OUTSTANDING_CHILDREN[-1], child)
        child.stdout.close.assert_called_once()
        # The fixture is an inert fake; retire its simulated ownership explicitly.
        del r._OUTSTANDING_CHILDREN[before:]

    def test_cli_fixed_diagnostics_and_relocated_complete_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ('check-service-readiness.py', 'service_readiness.py',
                         'service_protocol.py', 'service-protocol-v1.json'):
                shutil.copyfile(Path(__file__).with_name(name), root / name)
            result = subprocess.run([sys.executable, str(root / 'check-service-readiness.py'), NONCE],
                                    cwd='/', env={'PATH': os.defpath}, capture_output=True, check=False)
            self.assertEqual((result.returncode, result.stdout, result.stderr), (2, b'readiness: usage\n', b''))
            (root / 'service-protocol-v1.json').unlink()
            result = subprocess.run([sys.executable, str(root / 'check-service-readiness.py')],
                                    cwd='/', env={'PATH': os.defpath}, capture_output=True, check=False)
            self.assertEqual((result.returncode, result.stdout, result.stderr), (1, b'readiness: bundle\n', b''))

    def test_main_never_prints_injected_raw_error(self):
        output = io.StringIO()
        with patch.object(r, 'parse_arguments', side_effect=OSError(NONCE)), contextlib.redirect_stdout(output):
            self.assertEqual(r.main([]), 1)
        self.assertEqual(output.getvalue(), 'readiness: internal\n')


if __name__ == '__main__':
    unittest.main()
