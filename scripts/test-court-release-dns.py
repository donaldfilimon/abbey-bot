#!/usr/bin/env python3
"""Actual ingress validator regressions, imported beside the source preparer."""
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location(
    'court_release_dns_subject', Path(__file__).with_name('prepare-court-release.py'))
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


class OriginDnsLabelTests(unittest.TestCase):
    def proposal(self, host):
        value = package.default_ingress()
        value['tls']['origin'] = 'https://' + host
        return value

    def test_64_character_dns_labels_are_refused(self):
        # This is the intended attributable RED in the current implementation:
        # total origin length passes, while one DNS label is over63characters.
        for host in ['a' * 64 + '.example', 'court.' + 'b' * 64 + '.example']:
            with self.subTest(host=host):
                with self.assertRaisesRegex(package.PackageError, '^unsafe-ingress$'):
                    package.validate_ingress(self.proposal(host))

    def test_labels_with_leading_or_trailing_hyphens_are_refused(self):
        # These protect the per-label repair without assuming they currently
        # fail: the present whole-host regex appears already to refuse them.
        for host in ['-court.example', 'court-.example', 'court.-example',
                     'court.example-', 'court.-stage.example', 'court.stage-.example']:
            with self.subTest(host=host):
                with self.assertRaisesRegex(package.PackageError, '^unsafe-ingress$'):
                    package.validate_ingress(self.proposal(host))

    def test_empty_dns_labels_and_trailing_root_dot_are_refused(self):
        for host in ['.court.example', 'court..example', 'court.example.']:
            with self.subTest(host=host):
                with self.assertRaisesRegex(package.PackageError, '^unsafe-ingress$'):
                    package.validate_ingress(self.proposal(host))

    def test_valid_dns_label_controls_preserve_the_proposal_only_contract(self):
        for host in ['court.example', 'court-room.stage-2.example',
                     'a' * 63 + '.example', '1court.c3.example',
                     'xn--bcher-kva.example', 'court--room.example']:
            with self.subTest(host=host):
                original = self.proposal(host)
                result = package.validate_ingress(original)
                self.assertEqual(result, original)
                self.assertEqual(result['status'], 'proposal-only')
                self.assertEqual(result['backend'], {'address': '127.0.0.1', 'port': 8791})
                self.assertNotIn('iframe_receipt', result)
                self.assertNotIn('readiness', result)


if __name__ == '__main__':
    unittest.main()
