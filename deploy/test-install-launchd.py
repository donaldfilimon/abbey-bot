#!/usr/bin/env python3
"""Execute the real shell transaction in a relocated bundle under fake effects."""
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import service_transaction as transaction

LABEL = 'com.donaldfilimon.abbey-bot'
CANARY = 'PRIVATE-CANARY-should-never-be-visible'
FAKE = r'''
import hashlib, json, os, signal, time
from pathlib import Path
from service_readiness import ReadinessError, FailureCode
from service_protocol import encode_document

class Fake:
    def __init__(self):
        self.home = Path(os.environ['HOME'])
        self.file = self.home / 'fake-state.json'
    def load(self):
        return json.loads(self.file.read_text())
    def save(self, state):
        self.file.write_text(json.dumps(state))
    def monotonic(self):
        return self.load()['ns']
    def wall(self):
        return 100000 + self.monotonic() // 1000000
    def sleep(self, seconds):
        state = self.load(); state['ns'] += round(seconds * 1e9); self.save(state)
    def alive(self, pid):
        state = self.load()
        return state['active'] and state['pid'] == pid
    def absent(self, home, deadline):
        return not self.load()['loaded']
    def optional_pid(self, deadline):
        state=self.load()
        if state.get('mode') == 'missing_pid':
            raise ReadinessError(FailureCode.LAUNCHD)
        return state['pid'] if state['active'] else None
    def exit78(self, deadline):
        return self.load().get('mode') == 'exit78'
    def pid(self, deadline):
        state = self.load()
        if not state['active']:
            raise ReadinessError(FailureCode.LAUNCHD)
        if state.get('mode') == 'pid_change' and state['ns'] - state['started'] >= 250000000:
            return state['pid'] + 1
        if state.get('mode') == 'future':
            path = self.home / '.local/share/abbey-bot/readiness.json'
            doc = json.loads(path.read_text()); doc['published_at_unix_ms'] = self.wall() + 2001
            path.write_bytes(encode_document(doc))
        if state.get('mode') == 'missing_pid':
            raise ReadinessError(FailureCode.LAUNCHD)
        if state.get('mode') == 'nonce_change' and state['ns'] - state['started'] >= 250000000:
            path = self.home / '.local/share/abbey-bot/readiness.json'
            doc = json.loads(path.read_text()); doc['run_nonce'] = 'e' * 64
            path.write_bytes(encode_document(doc))
        if state.get('mode') == 'stop_scheduler' and state['ns'] - state['started'] >= 250000000:
            path = self.home / '.local/share/abbey-bot/readiness.json'
            doc = json.loads(path.read_text()); doc['scheduler'] = 'stopped'
            path.write_bytes(encode_document(doc))
        return state['pid']
    def send_signal(self, name):
        state = self.load()
        if state.get('signal_sent'): return
        state['helper_blocked'] = True; state['signal_sent'] = True; self.save(state)
        os.kill(state['shell_pid'], getattr(signal, name))
        time.sleep(.3)
        state = self.load(); state['helper_blocked'] = False; self.save(state)
    def control(self, operation, home, deadline):
        state = self.load()
        state['commands'].append(operation)
        if operation == 'bootout':
            if state.get('helper_blocked'):
                raise AssertionError('rollback raced the old helper')
            if state['scenario'] == 'stop_failure':
                self.save(state); return 1
            state['active'] = False; state['loaded'] = False; self.save(state)
            if state['scenario'] == 'successor_uninstall':
                path = home / '.local/share/abbey-bot/readiness.json'
                doc = json.loads(path.read_text()); doc['pid'] += 10; doc['run_nonce'] = 'f'*64
                path.write_bytes(encode_document(doc))
            return 0
        state['starts'] += 1
        mode = state['scenario'] if state['starts'] == 1 else state.get('rollback_mode', 'success')
        state['mode'] = mode
        state['pid'] += 1
        state['active'] = mode != 'exit78'
        state['loaded'] = True
        state['started'] = state['ns']
        self.save(state)
        if mode == 'slow_bootstrap':
            self.sleep(26)
        if mode == 'lock_replaced':
            (home / '.local/share/abbey-bot/install.lock/owner').write_text('f'*64)
        if mode.startswith('signal_') and not mode.startswith('signal_publish_') and state['starts'] == 1:
            self.send_signal(mode[len('signal_'):])
        if mode == 'cleanup_incomplete':
            raise ReadinessError(FailureCode.CLEANUP)
        if mode == 'bootstrap_failure':
            return 1
        base = home / '.local/share/abbey-bot'
        digest = hashlib.sha256((home / '.local/libexec/abbey-bot/abbey-bot').read_bytes()).hexdigest()
        nonce = ('b' if state['starts'] == 1 else 'd') * 64
        if mode.startswith('bootstrap_') and mode != 'bootstrap_failure':
            doc = {'schema_version': 1, 'pid': state['pid'], 'run_nonce': nonce,
                   'executable_sha256': digest, 'phase': 'failed', 'code': mode[len('bootstrap_'):]}
            path = base / 'bootstrap-status.json'; path.write_bytes(encode_document(doc, 'bootstrap')); path.chmod(0o600)
            return 0
        if mode in ('missing', 'exit78', 'missing_pid'):
            return 0
        doc = {'schema_version': 1, 'pid': state['pid'], 'run_nonce': nonce,
               'executable_sha256': digest, 'phase': 'ready', 'published_at_unix_ms': self.wall(),
               'discord': 'ready', 'scheduler': 'running', 'telegram': 'disabled',
               'slack': 'degraded', 'last_persistence': 'complete'}
        if mode == 'wrong_nonce': doc['run_nonce'] = 'a' * 64
        if mode == 'wrong_sha': doc['executable_sha256'] = 'c' * 64
        if mode == 'wrong_pid': doc['pid'] += 1
        if mode == 'stale': doc['published_at_unix_ms'] = 0
        if mode == 'future': doc['published_at_unix_ms'] += 2001
        if mode == 'starting': doc['phase'] = 'starting'
        if mode == 'discord': doc['discord'] = 'connecting'
        if mode == 'scheduler': doc['scheduler'] = 'stopped'
        path = base / 'readiness.json'
        raw = encode_document(doc)
        if mode == 'malformed': raw = b'{PRIVATE-CANARY-should-never-be-visible'
        if mode == 'duplicate': raw = raw.replace(b'"schema_version":1', b'"schema_version":1,"schema_version":1')
        if mode == 'unknown': raw = raw.replace(b'{', b'{"unknown":true,', 1)
        if mode == 'missing_key': raw = raw.replace(b'"schema_version":1,', b'')
        if path.exists() or path.is_symlink(): path.unlink()
        if mode == 'symlink':
            target = base / 'other'; target.write_bytes(raw); target.chmod(0o600); path.symlink_to(target)
        else:
            path.write_bytes(raw); path.chmod(0o644 if mode == 'wrong_mode' else 0o600)
        return 0
'''


class Harness:
    def __init__(self, temporary, scenario='success', prior=False, rollback='success'):
        self.root = Path(temporary)
        self.home = self.root / 'home'; self.home.mkdir(mode=0o700)
        self.repo = self.root / 'bundle'; (self.repo / 'deploy').mkdir(parents=True)
        source = Path(__file__).resolve().parent
        for name in ('install-launchd.sh', 'service_transaction.py', 'service_installation.py',
                     'service_readiness.py', 'service_protocol.py', 'service-protocol-v1.json',
                     'check-service-readiness.py', 'check-launchd-env.sh', LABEL + '.plist'):
            shutil.copyfile(source / name, self.repo / 'deploy' / name)
        helper = self.repo / 'deploy/service_transaction.py'
        text = helper.read_text().replace("if __name__ == '__main__':", "from fixture_effects import Fake\nSYSTEM = Fake()\n\nif __name__ == '__main__':")
        text = text.replace("        elif operation == 'publish':", "        elif operation == 'publish':\n            scenario = SYSTEM.load()['scenario']\n            if scenario.startswith('signal_publish_'):\n                SYSTEM.send_signal(scenario[len('signal_publish_'):])")
        guards = """
def guard_target(tree, relative):
    target = (tree.home / relative).resolve()
    fixture_home = Path(os.environ['HOME']).resolve()
    if not target.is_relative_to(fixture_home):
        raise AssertionError('mutation outside fixture home')
_original_write = PrivateTree.write
_original_remove = PrivateTree.remove
_original_directory = PrivateTree.directory
def checked_write(tree, relative, *args, **kwargs):
    guard_target(tree, relative)
    return _original_write(tree, relative, *args, **kwargs)
def checked_remove(tree, relative, *args, **kwargs):
    guard_target(tree, relative)
    return _original_remove(tree, relative, *args, **kwargs)
def checked_directory(tree, relative, create=False, exact=False):
    if create:
        guard_target(tree, relative)
    return _original_directory(tree, relative, create=create, exact=exact)
PrivateTree.write = checked_write
PrivateTree.remove = checked_remove
PrivateTree.directory = checked_directory
"""
        text = text.replace("if __name__ == '__main__':", guards + "\nif __name__ == '__main__':")
        helper.write_text(text)
        (self.repo / 'deploy/fixture_effects.py').write_text(FAKE)
        (self.repo / 'target/release').mkdir(parents=True)
        (self.repo / 'target/release/abbey-bot').write_bytes(b'candidate binary')
        self.commands = self.root / 'commands'; self.commands.mkdir()
        (self.commands / 'python3').symlink_to(sys.executable)
        cargo = self.commands / 'cargo'; cargo.write_text('#!/bin/sh\necho private-build-canary >&2\nexit 0\n'); cargo.chmod(0o700)
        self.env = {'HOME': str(self.home), 'PATH': str(self.commands) + ':/usr/bin:/bin', 'LC_ALL': 'C'}
        self.write('.config/abbey-bot/env', ('DISCORD_TOKEN=' + CANARY + '\n').encode())
        self.statefile = self.home / 'fake-state.json'
        self.statefile.write_text(json.dumps({'ns': 0, 'scenario': scenario, 'rollback_mode': rollback,
                                             'starts': 0, 'pid': 4242, 'active': prior, 'loaded': prior, 'commands': []}))
        self.write('Library/Logs/abbey-bot/abbey-bot.log', b'legacy untouched')
        if prior:
            self.write('.local/libexec/abbey-bot/abbey-bot', b'old binary', 0o700)
            model = plistlib.loads((source / (LABEL + '.plist')).read_bytes())
            model['ProgramArguments'] = [str(self.home / '.local/libexec/abbey-bot/abbey-bot'), '--managed-service']
            model['WorkingDirectory'] = str(self.home / '.local/share/abbey-bot')
            self.write('Library/LaunchAgents/' + LABEL + '.plist', plistlib.dumps(model))
            self.write('.local/share/abbey-bot/readiness.json', self.old_ready())
    def old_ready(self):
        return (json.dumps({'schema_version':1,'pid':4242,'run_nonce':'a'*64,
                 'executable_sha256':hashlib.sha256(b'old binary').hexdigest(),'phase':'draining',
                 'published_at_unix_ms':0,'discord':'stopped','scheduler':'stopped','telegram':'disabled',
                 'slack':'disabled','last_persistence':'complete'}, separators=(',', ':'))+'\n').encode()
    def write(self, rel, raw, mode=0o600):
        path = self.home / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        for directory in path.parents:
            if directory == self.root: break
            directory.chmod(0o700)
        path.write_bytes(raw); path.chmod(mode)
    def run(self, *args):
        child = subprocess.Popen(['/bin/sh', str(self.repo / 'deploy/install-launchd.sh'), *args],
                                 cwd='/', env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        value = self.state(); value['shell_pid'] = child.pid; self.statefile.write_text(json.dumps(value))
        try:
            stdout, stderr = child.communicate(timeout=20)
        except BaseException:
            child.kill(); child.wait(); raise
        return subprocess.CompletedProcess(child.args, child.returncode, stdout, stderr)
    def state(self):
        return json.loads(self.statefile.read_text())
    def binary(self):
        return self.home / '.local/libexec/abbey-bot/abbey-bot'


class Tests(unittest.TestCase):
    def check_private(self, result, harness):
        output = result.stdout + result.stderr
        for canary in (CANARY, 'a'*64, 'b'*64, 'd'*64, str(harness.home), 'private-build-canary'):
            self.assertNotIn(canary.encode(), output)
        self.assertEqual((harness.home / 'Library/Logs/abbey-bot/abbey-bot.log').read_bytes(), b'legacy untouched')
        self.assertTrue(all(command in ('bootstrap', 'bootout') for command in harness.state()['commands']))
    def test_fresh_and_update(self):
        for prior in (False, True):
            with tempfile.TemporaryDirectory() as temp:
                h = Harness(temp, prior=prior); result = h.run(); self.check_private(result,h)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(h.binary().read_bytes(), b'candidate binary')
                self.assertFalse((h.home / '.local/share/abbey-bot/install.lock').exists())
                self.assertEqual(h.state()['ns'], 5_000_000_000)
                if prior:
                    backups = list((h.home / '.local/share/abbey-bot/rollback/abbey').glob('*/binary'))
                    self.assertEqual(backups[0].read_bytes(), b'old binary')
    def test_candidate_failure_and_verified_rollback(self):
        for scenario in ('wrong_sha','wrong_pid','wrong_nonce','stale','future','starting','discord',
                         'scheduler','pid_change','nonce_change','stop_scheduler','bootstrap_failure',
                         'missing','exit78','slow_bootstrap'):
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as temp:
                h=Harness(temp, scenario, prior=True); result=h.run(); self.check_private(result,h)
                self.assertEqual(result.returncode,1)
                self.assertEqual(h.binary().read_bytes(), b'old binary')
                self.assertIn(b'rollback_ready', result.stderr)
                self.assertFalse((h.home / '.local/share/abbey-bot/install.lock').exists())
    def test_unsafe_candidate_retains_recovery_and_lock(self):
        for scenario in ('malformed','duplicate','unknown','missing_key','symlink','wrong_mode','missing_pid'):
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as temp:
                h=Harness(temp,scenario,prior=True); result=h.run(); self.check_private(result,h)
                self.assertEqual(result.returncode,1)
                self.assertIn(b'recovery_retained',result.stderr)
                self.assertTrue((h.home / '.local/share/abbey-bot/install.lock').is_dir())
    def test_incomplete_child_cleanup_never_starts_competing_rollback_or_releases_lock(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp,'cleanup_incomplete',prior=True)
            result=h.run();self.check_private(result,h)
            self.assertEqual(result.returncode,1)
            self.assertIn(b'cleanup_incomplete',result.stderr)
            self.assertIn(b'recovery_retained',result.stderr)
            self.assertEqual(h.state()['commands'],['bootout','bootstrap'])
            self.assertEqual(h.binary().read_bytes(),b'candidate binary')
            self.assertTrue((h.home/'.local/share/abbey-bot/install.lock/owner').is_file())

    def test_rollback_failure_retains_recovery(self):
        for mode in ('bootstrap_failure','starting','wrong_sha'):
            with tempfile.TemporaryDirectory() as temp:
                h=Harness(temp,'wrong_sha',prior=True,rollback=mode); result=h.run(); self.check_private(result,h)
                self.assertEqual(result.returncode,1)
                self.assertIn(b'recovery_retained',result.stderr)
    def test_bootstrap_closed_failure_codes(self):
        for code in ('readiness_file','log_directory','log_file','log_writer'):
            with tempfile.TemporaryDirectory() as temp:
                h=Harness(temp,'bootstrap_'+code,prior=True); result=h.run(); self.check_private(result,h)
                self.assertEqual(result.returncode,1)
                self.assertIn(('bootstrap_'+code).encode(), result.stderr)
    def test_prior_bootstrap_only_failure_cannot_poison_fresh_same_binary(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp)
            boot={'schema_version':1,'pid':4242,'run_nonce':'a'*64,
                  'executable_sha256':hashlib.sha256(b'candidate binary').hexdigest(),
                  'phase':'failed','code':'log_directory'}
            h.write('.local/share/abbey-bot/bootstrap-status.json',json.dumps(boot).encode())
            result=h.run();self.check_private(result,h)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(h.binary().read_bytes(),b'candidate binary')

    def test_same_binary_rollback_excludes_failed_bootstrap_identity(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp,'bootstrap_log_directory',prior=True)
            (h.repo/'target/release/abbey-bot').write_bytes(b'old binary')
            result=h.run();self.check_private(result,h)
            self.assertEqual(result.returncode,1)
            self.assertIn(b'rollback_ready',result.stderr)
            self.assertFalse((h.home/'.local/share/abbey-bot/install.lock').exists())

    def test_loaded_failed_start_without_pid_can_update_or_uninstall(self):
        for uninstall in (False,True):
            with tempfile.TemporaryDirectory() as temp:
                h=Harness(temp,prior=True)
                value=h.state();value['active']=False;value['loaded']=True
                h.statefile.write_text(json.dumps(value))
                result=h.run(*(['--uninstall'] if uninstall else []));self.check_private(result,h)
                self.assertEqual(result.returncode,0,result.stderr)
                self.assertEqual(h.state()['commands'][0],'bootout')

    def test_malformed_baseline_fails_before_stop(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp,prior=True)
            h.write('.local/share/abbey-bot/readiness.json',CANARY.encode())
            result=h.run();self.check_private(result,h)
            self.assertEqual(result.returncode,1)
            self.assertEqual(h.state()['commands'],[])
            self.assertEqual(h.binary().read_bytes(),b'old binary')
    def test_lock_contention_does_not_rewrite_owner(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp); h.write('.local/share/abbey-bot/install.lock/owner', b'foreign')
            result=h.run();self.check_private(result,h)
            self.assertEqual(result.returncode,1)
            self.assertEqual((h.home / '.local/share/abbey-bot/install.lock/owner').read_bytes(),b'foreign')
    def test_unsafe_published_targets_and_environment_fail_before_stop(self):
        for target in ('.local/libexec/abbey-bot/abbey-bot', 'Library/LaunchAgents/'+LABEL+'.plist', '.config/abbey-bot/env'):
            with tempfile.TemporaryDirectory() as temp:
                h=Harness(temp,prior=True); path=h.home/target
                path.unlink();path.symlink_to(h.home/'Library/Logs/abbey-bot/abbey-bot.log')
                result=h.run();self.check_private(result,h)
                self.assertEqual(result.returncode,1)
                self.assertEqual(h.state()['commands'],[])
    def test_uninstall_retains_binary_environment_logs_and_data(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp,prior=True);h.write('.local/share/abbey-bot/data/keep',b'data')
            result=h.run('--uninstall');self.check_private(result,h)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(h.binary().read_bytes(),b'old binary')
            self.assertEqual((h.home/'.local/share/abbey-bot/data/keep').read_bytes(),b'data')
            self.assertFalse((h.home/'.local/share/abbey-bot/readiness.json').exists())
            self.assertFalse((h.home/'Library/LaunchAgents'/ (LABEL+'.plist')).exists())
    def test_uninstall_absent_installation_is_idempotent(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp)
            result=h.run('--uninstall');self.check_private(result,h)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(h.state()['commands'],[])

    def test_initial_stop_failure_never_publishes_or_rolls_back(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp,'stop_failure',prior=True);result=h.run();self.check_private(result,h)
            self.assertEqual(result.returncode,1)
            self.assertEqual(h.binary().read_bytes(),b'old binary')
            self.assertEqual(h.state()['commands'],['bootout'])

    def test_signal_cleanup_uses_verified_rollback(self):
        # Signals target only this harness's owned shell child, never a service PID.
        for signal_name in ('SIGHUP', 'SIGINT', 'SIGTERM', 'publish_SIGHUP', 'publish_SIGINT', 'publish_SIGTERM'):
            with self.subTest(signal=signal_name), tempfile.TemporaryDirectory() as temp:
                h=Harness(temp,'signal_'+signal_name,prior=True)
                result=h.run();self.check_private(result,h)
                self.assertEqual(result.returncode,1)
                self.assertEqual(h.binary().read_bytes(),b'old binary')
                self.assertIn(b'rollback_ready',result.stderr)

    def test_successor_readiness_survives_uninstall(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp,'successor_uninstall',prior=True)
            result=h.run('--uninstall');self.check_private(result,h)
            self.assertEqual(result.returncode,0,result.stderr)
            value=json.loads((h.home/'.local/share/abbey-bot/readiness.json').read_text())
            self.assertEqual(value['run_nonce'],'f'*64)

    def test_same_identity_bootstrap_without_readiness_is_removed_on_uninstall(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp,prior=True)
            (h.home/'.local/share/abbey-bot/readiness.json').unlink()
            boot={'schema_version':1,'pid':4242,'run_nonce':'a'*64,
                  'executable_sha256':hashlib.sha256(b'old binary').hexdigest(),'phase':'starting','code':'none'}
            h.write('.local/share/abbey-bot/bootstrap-status.json',json.dumps(boot).encode())
            result=h.run('--uninstall');self.check_private(result,h)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertFalse((h.home/'.local/share/abbey-bot/bootstrap-status.json').exists())

    def test_legacy_rollback_has_no_pid_only_exception(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp,'wrong_sha',prior=True)
            path=h.home/('Library/LaunchAgents/'+LABEL+'.plist')
            value=plistlib.loads(path.read_bytes());value['ProgramArguments']=['/bin/sh','-c','legacy']
            path.write_bytes(plistlib.dumps(value))
            result=h.run();self.check_private(result,h)
            self.assertEqual(result.returncode,1)
            self.assertIn(b'recovery_retained',result.stderr)
            self.assertEqual(h.binary().read_bytes(),b'old binary')

    def test_replaced_lock_owner_is_not_removed(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp,'lock_replaced',prior=True)
            result=h.run();self.check_private(result,h)
            self.assertEqual(result.returncode,1)
            self.assertEqual((h.home/'.local/share/abbey-bot/install.lock/owner').read_text(),'f'*64)

    def test_invalid_environment_and_usage_are_fixed_failures(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp,prior=True);h.write('.config/abbey-bot/env',b'DISCORD_TOKEN=\n')
            result=h.run();self.check_private(result,h)
            self.assertEqual(result.returncode,1)
            self.assertEqual(h.state()['commands'],[])
            result=h.run('--unknown');self.check_private(result,h)
            self.assertEqual(result.returncode,2)

    def test_missing_complete_bundle_fails_without_raw_path(self):
        with tempfile.TemporaryDirectory() as temp:
            h=Harness(temp)
            (h.repo/'deploy/service-protocol-v1.json').unlink()
            result=h.run();self.check_private(result,h)
            self.assertEqual(result.returncode,1)
            self.assertIn(b'installation: bundle',result.stderr)

    def test_production_control_uses_fixed_argv_and_observes_timeout_cleanup(self):
        effects=transaction.Production()
        child=Mock();child.poll.return_value=None
        child.wait.side_effect=[subprocess.TimeoutExpired('private',1),0]
        with patch.object(transaction.subprocess,'Popen',return_value=child) as spawn, \
             patch.object(effects,'monotonic',return_value=0):
            with self.assertRaises(transaction.TransactionError) as error:
                effects.control('bootstrap',Path('/fixture'),30_000_000_000)
            self.assertEqual(error.exception.code,'timeout')
            self.assertEqual(spawn.call_args.args[0],['/bin/launchctl','bootstrap',
                             f'gui/{os.getuid()}', '/fixture/Library/LaunchAgents/'+LABEL+'.plist'])
            child.kill.assert_called_once()
            self.assertEqual(child.wait.call_count,2)
            self.assertLessEqual(child.wait.call_args.kwargs['timeout'],2)

    def test_production_status_capture_cap_kills_and_joins_owned_child(self):
        effects=transaction.Production()
        child=Mock();child.poll.return_value=None;child.wait.return_value=0
        with patch.object(transaction.subprocess,'Popen',return_value=child), \
             patch.object(transaction.select,'select',return_value=([child.stdout],[],[])), \
             patch.object(transaction.os,'read',return_value=b'x'*4096), \
             patch.object(effects,'monotonic',return_value=0):
            with self.assertRaises(transaction.TransactionError) as error:
                effects.status(30_000_000_000)
            self.assertEqual(error.exception.code,'launchd')
            child.kill.assert_called_once();child.wait.assert_called_once()
            child.stdout.close.assert_called_once();child.stderr.close.assert_called_once()

    def test_absence_requires_exact_domain_error_and_exit78_is_not_historical(self):
        effects=transaction.Production()
        expected=f'Could not find service "{LABEL}" in domain for user gui: {os.getuid()}'.encode()
        with patch.object(effects,'status',return_value=(113,b'',expected)):
            self.assertTrue(effects.absent(Path('/fixture'),1))
        with patch.object(effects,'status',return_value=(113,b'',b'private other failure')):
            with self.assertRaises(transaction.TransactionError):
                effects.absent(Path('/fixture'),1)
        root=f'gui/{os.getuid()}/{LABEL} = {{\n'.encode()
        with patch.object(effects,'status',return_value=(0,root+b'\tlast exit code = 78\n}\n',b'')):
            self.assertTrue(effects.exit78(1))
        with patch.object(effects,'status',return_value=(0,root+b'\tpid = 4242\n\tlast exit code = 78\n}\n',b'')):
            self.assertFalse(effects.exit78(1))


if __name__ == '__main__':
    unittest.main()
