#!/usr/bin/env python3
"""Offline parity fixtures for the runtime and pre-stop installer validator."""
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location('service_environment', ROOT / 'deploy/service_environment.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class EnvironmentTests(unittest.TestCase):
    def test_shared_literal_corpus(self):
        cases = json.loads((ROOT / 'tests/fixtures/managed-environment-v1.json').read_text())
        self.assertEqual(len(cases), 34)
        for case in cases:
            with self.subTest(case=case['name']):
                raw = case['document'].encode() if 'document' in case else bytes.fromhex(case['bytes_hex'])
                try:
                    MODULE.validate_environment(raw)
                    valid = True
                except MODULE.EnvironmentError as error:
                    valid = False
                    self.assertIn(str(error), ('syntax', 'required_configuration'))
                self.assertEqual(valid, case['valid'])

    def test_error_does_not_retain_secret_values(self):
        try:
            MODULE.validate_environment(b'DISCORD_TOKEN=PRIVATE_CANARY\nDISCORD_TOKEN=again')
        except MODULE.EnvironmentError as error:
            self.assertNotIn('PRIVATE', str(error))
            self.assertNotIn('PRIVATE', repr(error))
        else:
            self.fail('duplicate assignment accepted')


if __name__ == '__main__':
    unittest.main()
