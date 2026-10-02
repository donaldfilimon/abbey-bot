#!/usr/bin/env python3
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('ops_control', Path(__file__).with_name('community-operations-control.py'))
control = importlib.util.module_from_spec(spec)
spec.loader.exec_module(control)


class PolicyControls(unittest.TestCase):
    def test_stop_preserves_exact_policy_and_private_mode(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'policy.json'
            policy = {'version': 1, 'mode': 'apply', 'guild': 1, 'actions': [{'key': 'retain-me'}]}
            path.write_text(json.dumps(policy)); path.chmod(0o600)
            result = control.control(path, 'stopped')
            self.assertEqual(result['mode'], 'stopped')
            self.assertEqual(json.loads(path.read_text()), dict(policy, mode='stopped'))
            self.assertEqual(path.stat().st_mode & 0o077, 0)

    @unittest.skipUnless(os.name == 'posix', 'POSIX permissions')
    def test_shared_or_symlink_policy_cannot_authorize_updates(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'policy.json'; path.write_text('{}'); path.chmod(0o644)
            with self.assertRaises(ValueError): control.control(path, 'apply')
            path.chmod(0o600); link = Path(directory) / 'link'; link.symlink_to(path)
            with self.assertRaises(ValueError): control.control(link, 'apply')

    def test_unknown_policy_version_is_not_rewritten(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'policy.json'; path.write_text('{"version":2,"mode":"propose"}'); path.chmod(0o600)
            before = path.read_bytes()
            with self.assertRaises(ValueError): control.control(path, 'apply')
            self.assertEqual(path.read_bytes(), before)

    def test_newer_policy_is_preserved_during_mode_publication(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'policy.json'
            path.write_text('{"version":1,"mode":"propose","actions":[]}'); path.chmod(0o600)
            newer = '{"version":1,"mode":"propose","actions":[{"key":"new"}]}'
            original = control.os.fsync
            def concurrent_update(handle):
                path.write_text(newer)
                original(handle)
            with patch.object(control.os, 'fsync', side_effect=concurrent_update):
                with self.assertRaises(ValueError): control.control(path, 'apply')
            self.assertEqual(path.read_text(), newer)
            self.assertFalse(path.with_suffix('.mode-lock').exists())


if __name__ == '__main__': unittest.main()
