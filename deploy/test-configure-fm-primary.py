#!/usr/bin/env python3
"""Offline FM cutover fixtures; all restarts use in-memory fakes."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('fm_config', Path(__file__).with_name('configure-fm-primary.py'))
assert spec and spec.loader
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

@unittest.skipUnless(hasattr(os, 'getuid'), 'POSIX-only configurator')
class CutoverTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = self.root/'env'
        self.original = b'DISCORD_TOKEN=private-fixture\nABBEY_FM_FALLBACK=1\nABBEY_FM_ENDPOINT=http://127.0.0.1:9999\nOTHER="$(private)"\nABBEY_FM_MODE=off\n'
        self.env.write_bytes(self.original)
        self.env.chmod(0o600)
        self.binary = self.root/'abbey'
        self.cli = self.root/'fm'
        for path in (self.binary, self.cli):
            path.write_bytes(b'#!/bin/sh\nexit 0\n')
            path.chmod(0o700)
        self.binary.write_text("""#!/usr/bin/python3
import hashlib,json,pathlib,sys,os
assert not any(k.endswith('TOKEN') or k.endswith('KEY') for k in os.environ)
assert sys.argv[1]=='--fm-manifest-identity' and sys.argv[2]=='--cli' and sys.argv[4]=='--json'
print(json.dumps({'abbey_binary_sha256':hashlib.sha256(pathlib.Path(sys.argv[0]).read_bytes()).hexdigest(),'provider_binary_sha256':hashlib.sha256(pathlib.Path(sys.argv[3]).read_bytes()).hexdigest(),'os_sha256':hashlib.sha256(b'synthetic-os').hexdigest(),'tool_schema_sha256':'a'*64}))
""")
        hashes = {'abbey_binary_sha256':module.transaction.sha256(self.binary), 'provider_binary_sha256':module.transaction.sha256(self.cli), 'os_sha256':hashlib.sha256(b'synthetic-os').hexdigest(), 'tool_schema_sha256':'a'*64}
        self.records = [{'version':2,'fixture_version':module.transaction.FIXTURE_VERSION,'provider_id':provider,'provider_class':'os_managed_local','identity':hashes,'qualification_status':'qualified','declared_capabilities':{k:k in {'text','structured_output','tools'} for k in ('text','streaming','structured_output','tools','vision','ocr')},'isolation_capabilities':{k:False for k in ('environment_cleared','absolute_no_shell_execution','process_tree_contained','private_runtime_state','loopback_only','sandbox_attested')}} for provider in ('foundation-models-pcc','foundation-models')]
        self.records[1]['declared_capabilities']['streaming']=True
        self.manifest = self.root/'manifest.json'
        self.write_manifest()
        self.args = argparse.Namespace(env_file=self.env, manifest=self.manifest, binary=self.binary, cli=self.cli, backup_dir=self.root/'backups', apply=False, launchctl=None)
        self.os_patch = patch.object(module.transaction,'current_os_build',return_value='synthetic-os')
        self.os_patch.start()
        self.addCleanup(self.os_patch.stop)
    def write_manifest(self):
        self.manifest.write_text(json.dumps(self.records))
        self.manifest.chmod(0o600)
    def test_default_dry_run_leaves_environment_and_backups_untouched(self):
        module.run_locked(self.args)
        self.assertEqual(self.env.read_bytes(), self.original)
        self.assertFalse(self.args.backup_dir.exists())
    def apply(self, restart):
        self.args.apply=True
        with patch.object(module,'validate_launchctl',return_value=self.cli), patch.object(module,'service_pid',return_value=111), patch.object(module,'restart_and_require_stable',side_effect=restart):
            module.run_locked(self.args)
    def test_apply_preserves_every_unrelated_key_and_private_backup(self):
        self.apply([222])
        content=self.env.read_text()
        self.assertIn('DISCORD_TOKEN=private-fixture\n',content)
        self.assertIn('ABBEY_FM_FALLBACK=1\n',content)
        self.assertIn('ABBEY_FM_ENDPOINT=http://127.0.0.1:9999\n',content)
        self.assertIn('OTHER="$(private)"\n',content)
        self.assertIn('ABBEY_FM_MODE=pcc,system\n',content)
        self.assertEqual(stat.S_IMODE(self.env.stat().st_mode),0o600)
        backup=next(self.args.backup_dir.iterdir())
        self.assertEqual(backup.read_bytes(),self.original)
        self.assertEqual(stat.S_IMODE(backup.stat().st_mode),0o600)
    def test_failed_candidate_rolls_back_and_restarts_previous_environment(self):
        with self.assertRaisesRegex(SystemExit,'environment and service were restored'):
            self.apply([RuntimeError('synthetic refusal'),333])
        self.assertEqual(self.env.read_bytes(),self.original)
    def test_failed_rollback_reports_preserved_environment(self):
        with self.assertRaisesRegex(SystemExit,'service restart failed'):
            self.apply([RuntimeError('candidate'),RuntimeError('rollback')])
        self.assertEqual(self.env.read_bytes(),self.original)
    def test_refused_pcc_keeps_qualified_system_admissible(self):
        self.records[0]['qualification_status']='failed'
        self.write_manifest()
        module.run_locked(self.args)
    def test_no_qualified_mode_refuses(self):
        for record in self.records: record['qualification_status']='failed'
        self.write_manifest()
        with self.assertRaisesRegex(SystemExit,'at least one qualified'):
            module.run_locked(self.args)
        self.assertEqual(self.env.read_bytes(),self.original)
    def test_wrong_binary_cli_os_and_capability_refuse(self):
        for key in ('abbey_binary_sha256','provider_binary_sha256','os_sha256'):
            original=self.records[0]['identity'][key]
            self.records[0]['identity'][key]='b'*64
            self.write_manifest()
            with self.subTest(key=key), self.assertRaises(SystemExit): module.run_locked(self.args)
            self.records[0]['identity'][key]=original
        for record in self.records: record['declared_capabilities']['tools']=False
        self.write_manifest()
        with self.assertRaises(SystemExit): module.run_locked(self.args)
    def test_stale_qualified_pcc_does_not_hide_matching_system(self):
        self.records[0]['identity']=dict(self.records[0]['identity'])
        self.records[0]['identity']['abbey_binary_sha256']='b'*64
        self.write_manifest()
        module.run_locked(self.args)
    def test_system_endpoint_requires_streaming_when_pcc_is_refused(self):
        self.records[0]['qualification_status']='failed'
        self.records[1]['declared_capabilities']['streaming']=False
        self.write_manifest()
        with self.assertRaisesRegex(SystemExit,'required capabilities'): module.run_locked(self.args)
    def test_valid_but_wrong_tool_schema_hash_refuses(self):
        for record in self.records: record['identity']['tool_schema_sha256']='b'*64
        self.write_manifest()
        with self.assertRaisesRegex(SystemExit,'exact candidate identity'): module.run_locked(self.args)
    def test_identity_command_failure_refuses_before_publish(self):
        self.binary.write_bytes(b'#!/bin/sh\nexit 2\n')
        with self.assertRaisesRegex(SystemExit,'identity inspection failed'): module.run_locked(self.args)
        self.assertEqual(self.env.read_bytes(),self.original)
    def test_duplicate_managed_key_refuses(self):
        self.env.write_bytes(self.original+b'export ABBEY_FM_MODE=system\n')
        with self.assertRaisesRegex(SystemExit,'duplicate managed key'): module.run_locked(self.args)
    def test_symlink_and_public_manifest_refuse(self):
        self.manifest.chmod(0o644)
        with self.assertRaises(SystemExit): module.run_locked(self.args)
        self.manifest.chmod(0o600)
        link=self.root/'manifest-link';link.symlink_to(self.manifest)
        self.args.manifest=link
        with self.assertRaises(SystemExit): module.run_locked(self.args)
    def test_fallback_conflict_refuses_without_altering_unmanaged_setting(self):
        self.env.write_bytes(self.original.replace(b'ABBEY_FM_FALLBACK=1', b'ABBEY_FM_FALLBACK=0'))
        before=self.env.read_bytes()
        with self.assertRaisesRegex(SystemExit,'conflicts with primary'): module.run_locked(self.args)
        self.assertEqual(self.env.read_bytes(),before)
    def test_root_owned_system_cli_is_accepted(self):
        metadata=self.cli.lstat()
        root_owned=type('RootOwnedMetadata', (), {'st_mode':metadata.st_mode, 'st_uid':0})()
        original_lstat=Path.lstat
        def observed(path):
            return root_owned if path==self.cli else original_lstat(path)
        with patch.object(Path,'lstat',observed): module.run_locked(self.args)
    def test_unknown_fields_class_and_optional_hashes_refuse(self):
        for key,value in (('unknown_field',True),('provider_class','invalid'),('score_policy',1)):
            self.records[0][key]=value;self.write_manifest()
            with self.subTest(key=key),self.assertRaises(SystemExit): module.run_locked(self.args)
            if key=='provider_class': self.records[0][key]='os_managed_local'
            else: self.records[0].pop(key)
        self.records[0]['identity']['model_sha256']='invalid';self.write_manifest()
        with self.assertRaises(SystemExit): module.run_locked(self.args)
    def test_exact_manifest_and_parent_modes_are_required(self):
        self.manifest.chmod(0o400)
        with self.assertRaisesRegex(SystemExit,'exact modes'): module.run_locked(self.args)
        self.manifest.chmod(0o600)
        self.root.chmod(0o500)
        try:
            with self.assertRaisesRegex(SystemExit,'exact modes'): module.run_locked(self.args)
        finally:
            self.root.chmod(0o700)
    def test_fallback_truth_aliases_are_preserved(self):
        for alias in (b'true',b'on'):
            before=self.original.replace(b'ABBEY_FM_FALLBACK=1',b'ABBEY_FM_FALLBACK='+alias)
            self.env.write_bytes(before)
            module.run_locked(self.args)
            self.assertEqual(self.env.read_bytes(),before)
    def test_existing_install_lock_refuses(self):
        lock=self.root/'install.lock';lock.mkdir(mode=0o700);(lock/'pid').write_text('999999\n')
        with self.assertRaises(module.LockFailure):
            with module.InstallLock(lock): module.run_locked(self.args)
        self.assertEqual(self.env.read_bytes(),self.original)
    def test_unhashable_record_id_refuses_cleanly(self):
        self.records[0]['provider_id']=[];self.write_manifest()
        with self.assertRaises(SystemExit): module.run_locked(self.args)

if __name__=='__main__': unittest.main()
