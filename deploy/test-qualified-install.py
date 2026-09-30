#!/usr/bin/env python3
"""Relocated real shell/phase transaction, synthetic identity and launchd only."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('install_fixture', Path(__file__).with_name('test-install-launchd.py'))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
def setUpModule():
    if os.name == 'posix' and Path('/bin/sh').is_file(): fixture.setUpModule()

def tearDownModule():
    if os.name == 'posix' and Path('/bin/sh').is_file(): fixture.tearDownModule()
HEAD = '1' * 40

@unittest.skipUnless(os.name == 'posix' and Path('/bin/sh').is_file(), 'relocated POSIX installer fixture')
class Tests(unittest.TestCase):
    def harness(self, temp, scenario='success', rollback='success', old=True, configured=True):
        h = fixture.Harness(temp, scenario=scenario, prior=True, rollback=rollback)
        helper = h.repo / 'deploy/service_transaction.py'
        code = helper.read_text()
        code = code.replace('    return module\n', '''    module.candidate_identity = lambda binary, cli: {
        'abbey_binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'provider_binary_sha256': '2'*64, 'os_sha256': '3'*64, 'tool_schema_sha256': '4'*64}
    return module
''')
        code = code.replace("    result = subprocess.run(['git', 'rev-parse', 'HEAD'],", "    if head != '1'*40: raise TransactionError('head')\n    return\n    result = subprocess.run(['git', 'rev-parse', 'HEAD'],")
        helper.write_text(code)
        h.target = '.config/abbey-bot/fm-capability-manifest.json'
        env = (h.home / '.config/abbey-bot/env').read_bytes()
        if configured:
            env += ('ABBEY_FM_CAPABILITY_MANIFEST=' + str(h.home/h.target) + '\n').encode()
        cli = h.root / 'fm-cli'; cli.write_bytes(b'fake cli'); cli.chmod(0o700)
        env += ('ABBEY_FM_CLI=' + str(cli) + '\n').encode()
        h.write('.config/abbey-bot/env', env)
        if old:
            h.write(h.target, b'old manifest')
        h.manifest = h.root / 'candidate-manifest.json'
        caps = {name: name in ('text', 'structured_output', 'tools') for name in ('text', 'streaming', 'structured_output', 'tools', 'vision', 'ocr')}
        identity = {'abbey_binary_sha256': hashlib.sha256(b'candidate binary').hexdigest(),
                    'provider_binary_sha256': '2'*64, 'os_sha256': '3'*64, 'tool_schema_sha256': '4'*64}
        record = {'version':2,'fixture_version':'abbey-provider-fixtures-v1',
                  'provider_id':'foundation-models-pcc','provider_class':'os_managed_local',
                  'identity':identity,'declared_capabilities':caps,
                  'isolation_capabilities':{name:False for name in ('environment_cleared','absolute_no_shell_execution','process_tree_contained','private_runtime_state','loopback_only','sandbox_attested')},
                  'qualification_status':'qualified'}
        h.manifest.write_text(json.dumps([record])); h.manifest.chmod(0o600)
        h.args = ('--qualified-candidate', str(h.repo/'target/release/abbey-bot'), str(h.manifest), HEAD)
        h.env_before = env
        return h

    def assert_environment(self, h):
        self.assertEqual((h.home/'.config/abbey-bot/env').read_bytes(), h.env_before)

    def test_success_receipt_binds_head_binary_manifest_and_real_ready_identity(self):
        for configured in (True, False):
            with self.subTest(configured=configured), tempfile.TemporaryDirectory() as temp:
                h = self.harness(temp, configured=configured)
                result = h.run(*h.args)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_environment(h)
                self.assertEqual((h.home/h.target).read_bytes(), h.manifest.read_bytes())
                journal = json.loads(next((h.home/'.local/share/abbey-bot/rollback/abbey').glob('*/transaction.json')).read_bytes())
                self.assertEqual(journal['phase'], 'committed')
                self.assertEqual(journal['qualification']['head'], HEAD)
                self.assertEqual(journal['qualification']['binary_sha256'], hashlib.sha256(h.binary().read_bytes()).hexdigest())
                self.assertEqual(journal['qualification']['manifest_sha256'], hashlib.sha256(h.manifest.read_bytes()).hexdigest())
                self.assertEqual(journal['readiness'], json.loads((h.home/'.local/share/abbey-bot/readiness.json').read_bytes()))
                self.assertFalse((h.home/'.local/share/abbey-bot/install.lock').exists())

    def update_readiness_at_commit(self, h, changes):
        helper = h.repo / 'deploy/service_transaction.py'
        code = helper.read_text()
        marker = "        elif operation == 'commit':\n"
        self.assertEqual(code.count(marker), 1)
        # Advance the synthetic clock after start's stability wait, then publish
        # a new sample before commit reads it. No real service is involved.
        update = ("            SYSTEM.sleep(1)\n"
                  "            sample = read_optional_private(home)\n"
                  "            sample['published_at_unix_ms'] = SYSTEM.wall()\n"
                  f"            sample.update({changes!r})\n"
                  "            tree.write(STATE + '/readiness.json', (json.dumps(sample) + '\\n').encode())\n")
        helper.write_text(code.replace(marker, marker + update))

    def test_commit_accepts_fresh_heartbeat_and_receipts_latest_ready_sample(self):
        with tempfile.TemporaryDirectory() as temp:
            h = self.harness(temp)
            self.update_readiness_at_commit(h, {'slack': 'connected'})
            result = h.run(*h.args)
            self.assertEqual(result.returncode, 0, result.stderr)
            backup = next((h.home/'.local/share/abbey-bot/rollback/abbey').iterdir())
            verified = json.loads((backup/'verified-ready.json').read_bytes())
            journal = json.loads((backup/'transaction.json').read_bytes())
            ready = json.loads((h.home/'.local/share/abbey-bot/readiness.json').read_bytes())
            self.assertGreater(ready['published_at_unix_ms'], verified['published_at_unix_ms'])
            self.assertEqual(ready['slack'], 'connected')
            self.assertEqual(journal['phase'], 'committed')
            self.assertEqual(journal['readiness'], ready)
            self.assert_environment(h)
            self.assertFalse((h.home/'.local/share/abbey-bot/install.lock').exists())

    def test_commit_refuses_changed_identity_unready_or_stale_heartbeat(self):
        for changes in ({'run_nonce': 'e'*64}, {'pid': 4244},
                        {'executable_sha256': 'e'*64}, {'phase': 'draining'},
                        {'discord': 'connecting'}, {'scheduler': 'stopped'},
                        {'published_at_unix_ms': 0}, {'published_at_unix_ms': 999999}):
            with self.subTest(changes=changes), tempfile.TemporaryDirectory() as temp:
                h = self.harness(temp)
                self.update_readiness_at_commit(h, changes)
                result = h.run(*h.args)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(b'installation: receipt', result.stderr)
                self.assertEqual(h.binary().read_bytes(), b'old binary')
                self.assertEqual((h.home/h.target).read_bytes(), b'old manifest')
                self.assert_environment(h)
                journal = json.loads(next((h.home/'.local/share/abbey-bot/rollback/abbey').glob('*/transaction.json')).read_bytes())
                self.assertEqual(journal['phase'], 'restored')
                self.assertFalse((h.home/'.local/share/abbey-bot/install.lock').exists())

    def test_oversized_combined_arguments_refuse_before_acquiring_lock(self):
        with tempfile.TemporaryDirectory() as temp:
            h = self.harness(temp)
            longpath = '/' + 'x' * 3900
            result = h.run('--qualified-candidate', longpath, longpath, HEAD)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((h.home/'.local/share/abbey-bot/install.lock').exists())
            self.assertEqual(h.state()['commands'], [])
            self.assertEqual(h.binary().read_bytes(), b'old binary')
            self.assertEqual((h.home/h.target).read_bytes(), b'old manifest')
            self.assert_environment(h)

    def test_refusal_never_stops_or_changes_prior_artifacts(self):
        for problem in ('failed', 'wrong_sha', 'wrong_head', 'unsafe_target'):
            with self.subTest(problem=problem), tempfile.TemporaryDirectory() as temp:
                h = self.harness(temp)
                records = json.loads(h.manifest.read_bytes())
                if problem == 'failed': records[0]['qualification_status'] = 'failed'
                if problem == 'wrong_sha': records[0]['identity']['abbey_binary_sha256'] = '0'*64
                h.manifest.write_text(json.dumps(records))
                args = h.args
                if problem == 'wrong_head': args = (*h.args[:-1], '5'*40)
                if problem == 'unsafe_target':
                    h.write('.config/abbey-bot/env', h.env_before + b'ABBEY_FM_CAPABILITY_MANIFEST=/outside/private.json\n')
                    h.env_before = (h.home/'.config/abbey-bot/env').read_bytes()
                result = h.run(*args)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(h.state()['commands'], [])
                self.assertEqual(h.binary().read_bytes(), b'old binary')
                self.assertEqual((h.home/h.target).read_bytes(), b'old manifest')
                self.assert_environment(h)

    def test_start_failure_and_interrupt_restore_binary_manifest_with_environment_unchanged(self):
        for scenario in ('bootstrap_failure', 'wrong_sha', 'signal_publish_SIGTERM', 'signal_SIGINT'):
            for old in (True, False):
                with self.subTest(scenario=scenario, old=old), tempfile.TemporaryDirectory() as temp:
                    h = self.harness(temp, scenario, old=old)
                    result = h.run(*h.args)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(h.binary().read_bytes(), b'old binary')
                    self.assert_environment(h)
                    if old: self.assertEqual((h.home/h.target).read_bytes(), b'old manifest')
                    else: self.assertFalse((h.home/h.target).exists())
                    journal = json.loads(next((h.home/'.local/share/abbey-bot/rollback/abbey').glob('*/transaction.json')).read_bytes())
                    self.assertEqual(journal['phase'], 'restored')

    def test_failed_rollback_retains_lock_and_recovery_journal(self):
        with tempfile.TemporaryDirectory() as temp:
            h = self.harness(temp, 'bootstrap_failure', rollback='bootstrap_failure')
            result = h.run(*h.args)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b'recovery_retained', result.stderr)
            self.assertTrue((h.home/'.local/share/abbey-bot/install.lock/owner').exists())
            self.assertTrue(list((h.home/'.local/share/abbey-bot/rollback/abbey').glob('*/transaction.json')))
            self.assertEqual((h.home/h.target).read_bytes(), b'old manifest')

    def test_competing_owner_and_incomplete_cleanup_keep_recoverable_artifacts(self):
        for scenario in ('locked', 'cleanup_incomplete'):
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as temp:
                h = self.harness(temp, scenario)
                if scenario == 'locked': h.write('.local/share/abbey-bot/install.lock/owner', b'other owner')
                result = h.run(*h.args)
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue((h.home/'.local/share/abbey-bot/install.lock/owner').exists())
                self.assert_environment(h)
                if scenario == 'locked':
                    self.assertEqual(h.binary().read_bytes(), b'old binary')
                    self.assertEqual((h.home/h.target).read_bytes(), b'old manifest')
                else:
                    journal = json.loads(next((h.home/'.local/share/abbey-bot/rollback/abbey').glob('*/transaction.json')).read_bytes())
                    self.assertEqual(journal['phase'], 'publishing')

    def test_publication_failure_and_staged_swap_restore_all_identity_artifacts(self):
        for failure in ('manifest_write', 'manifest_swap'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temp:
                h = self.harness(temp)
                helper = h.repo / 'deploy/service_transaction.py'
                code = helper.read_text()
                if failure == 'manifest_write':
                    code = code.replace("                tree.write(q['target'], manifest)",
                                        "                raise TransactionError('write')\n                tree.write(q['target'], manifest)")
                else:
                    code = code.replace("            candidate = tree.read(backup + '/candidate-binary', 0o700)",
                                        "            tree.write(backup + '/candidate-manifest', b'changed')\n            candidate = tree.read(backup + '/candidate-binary', 0o700)")
                helper.write_text(code)
                result = h.run(*h.args)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(h.binary().read_bytes(), b'old binary')
                self.assertEqual((h.home/h.target).read_bytes(), b'old manifest')
                self.assert_environment(h)

    def test_symlink_or_writable_cli_refuses_without_stopping(self):
        for invalid in ('symlink', 'writable'):
            with self.subTest(invalid=invalid), tempfile.TemporaryDirectory() as temp:
                h = self.harness(temp)
                cli = h.root/'fm-cli'
                if invalid == 'symlink':
                    actual = h.root/'actual-cli'; cli.rename(actual); cli.symlink_to(actual)
                else: cli.chmod(0o777)
                result = h.run(*h.args)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(h.state()['commands'], [])
                self.assertEqual(h.binary().read_bytes(), b'old binary')
                self.assertEqual((h.home/h.target).read_bytes(), b'old manifest')

    def test_real_head_validator_checks_head_tracked_and_untracked_source(self):
        from unittest.mock import patch
        from types import SimpleNamespace
        good = SimpleNamespace(returncode=0, stdout=(HEAD+'\n').encode())
        clean = SimpleNamespace(returncode=0, stdout=b'')
        bad = SimpleNamespace(returncode=1, stdout=b'')
        untracked = SimpleNamespace(returncode=0, stdout=b'src/new.rs\n')
        for responses in ([good,clean,clean], [good,bad], [good,clean,untracked], [bad]):
            with patch.object(fixture.transaction.subprocess,'run',side_effect=responses):
                if len(responses)==3 and responses[-1] is clean:
                    fixture.transaction.verify_head(Path('.'),HEAD)
                else:
                    with self.assertRaises(fixture.transaction.TransactionError):
                        fixture.transaction.verify_head(Path('.'),HEAD)

if __name__ == '__main__': unittest.main()
