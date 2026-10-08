#!/usr/bin/env python3
"""Credential-free package tests; optional real Node uses only ephemeral loopback."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from urllib.request import ProxyHandler, Request, build_opener

SPEC = importlib.util.spec_from_file_location(
    'court_release', Path(__file__).with_name('prepare-court-release.py'))
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='abbey-court-package-')
        self.root = Path(self.temp.name).resolve()
        self.source = self.root / 'source'
        (self.source / 'server').mkdir(parents=True)
        for name in package.ASSETS:
            (self.source / name).write_bytes(('fixture:' + name).encode())
        self.node = self.root / 'node'
        self.node.write_bytes(b'synthetic-node-identity')
        self.identity = {**package.file_hash(self.node), 'version': 'v26.10.0'}
        self.output = self.root / 'packages'
        self.inspect = patch.object(package, 'inspect_node', return_value=self.identity)
        self.inspect.start()
        self.addCleanup(self.inspect.stop)
        self.addCleanup(self.cleanup_tree)

    def cleanup_tree(self):
        # Undo fixture readonly modes solely within our owned temp tree.
        for root, dirs, files in os.walk(self.root):
            Path(root).chmod(0o700)
            for name in files:
                path = Path(root) / name
                if not path.is_symlink():
                    path.chmod(0o600)
        self.temp.cleanup()

    def prepare(self, **kwargs):
        return package.prepare(self.source, self.output, self.node, **kwargs)

    def mutate_file(self, path, content):
        path.chmod(0o600)
        path.write_bytes(content)
        path.chmod(0o444)

    def assert_empty_publication(self):
        if self.output.exists():
            self.assertEqual(list(self.output.iterdir()), [])

    def test_exact_fixed_allowlist_digest_and_no_secret_or_oauth_copy(self):
        (self.source / '.env').write_text('SECRET_CANARY_NEVER_COPY')
        (self.source / 'server/token-exchange.example.mjs').write_text('OAUTH_CANARY')
        release = self.prepare()
        manifest = package.verify(release)
        self.assertEqual(manifest['status'], 'prepared-source-only')
        self.assertEqual(manifest['activation'], 'operator-required')
        self.assertEqual(manifest['ingress'], package.default_ingress())
        self.assertEqual(manifest['node'], self.identity)
        self.assertEqual([row['path'] for row in manifest['assets']], list(package.ASSETS))
        expected = hashlib.sha256()
        for name in package.ASSETS:
            data = (self.source / name).read_bytes()
            expected.update(f'{name}\0{len(data)}\0'.encode())
            expected.update(data)
            self.assertEqual((release / name).read_bytes(), data)
        self.assertEqual(manifest['court_digest'], expected.hexdigest())
        all_files = {p.relative_to(release).as_posix() for p in release.rglob('*') if p.is_file()}
        self.assertEqual(all_files, {*package.ASSETS, 'release-manifest.json'})
        for path in release.rglob('*'):
            if path.is_file():
                self.assertNotIn(b'CANARY', path.read_bytes())

    def test_missing_asset_refuses_before_publication(self):
        (self.source / 'court.js').unlink()
        with self.assertRaises(FileNotFoundError):
            self.prepare()
        self.assert_empty_publication()

    def test_oversized_asset_refuses_before_publication(self):
        with patch.object(package, 'MAX_ASSET_BYTES', 8):
            with self.assertRaisesRegex(package.PackageError, 'unsafe-file'):
                self.prepare()
        self.assert_empty_publication()

    def test_source_symlink_and_output_inside_source_are_refused(self):
        with self.assertRaisesRegex(package.PackageError, 'output-inside-source'):
            package.prepare(self.source, self.source / 'packages', self.node)
        target = self.source / 'court.js'
        target.unlink()
        try:
            target.symlink_to(self.source / 'app.js')
        except OSError:
            self.skipTest('symlink creation unavailable on this host')
        with self.assertRaisesRegex(package.PackageError, 'symlink-path'):
            self.prepare()
        self.assert_empty_publication()

    def test_source_change_during_preparation_cleans_owned_stage(self):
        actual = package.read_assets
        calls = 0

        def moving(root):
            nonlocal calls
            calls += 1
            if calls == 2:
                (self.source / 'court.js').write_bytes(b'new source revision')
            return actual(root)

        with patch.object(package, 'read_assets', side_effect=moving):
            with self.assertRaisesRegex(package.PackageError, 'source-changed'):
                self.prepare()
        self.assert_empty_publication()

    def test_prepare_replay_verifies_without_changing_bytes_or_mtime(self):
        release = self.prepare()
        before = {str(p): (p.read_bytes(), p.stat().st_mtime_ns)
                  for p in release.rglob('*') if p.is_file()}
        self.assertEqual(self.prepare(), release)
        self.assertEqual(before, {str(p): (p.read_bytes(), p.stat().st_mtime_ns)
                                 for p in release.rglob('*') if p.is_file()})
        self.assertEqual(len(list(self.output.iterdir())), 1)

    def test_held_writer_lock_is_never_removed_or_bypassed(self):
        self.output.mkdir(mode=0o700)
        lock = self.output / '.prepare.lock'
        lock.write_bytes(b'OTHER_OWNER')
        with self.assertRaisesRegex(package.PackageError, 'output-busy'):
            self.prepare()
        self.assertEqual(lock.read_bytes(), b'OTHER_OWNER')
        self.assertEqual(list(self.output.iterdir()), [lock])

    def test_asset_tamper_refuses_verify_and_matching_replay(self):
        release = self.prepare()
        self.mutate_file(release / 'court.js', b'changed artifact')
        with self.assertRaisesRegex(package.PackageError, 'asset-identity-mismatch'):
            package.verify(release)
        with self.assertRaisesRegex(package.PackageError, 'asset-identity-mismatch'):
            self.prepare()

    def test_extra_file_or_extra_empty_directory_refuses_inventory(self):
        release = self.prepare()
        release.chmod(0o700)
        extra = release / 'extra-directory'
        extra.mkdir()
        extra.chmod(0o555)
        release.chmod(0o555)
        with self.assertRaisesRegex(package.PackageError, 'release-inventory-mismatch'):
            package.verify(release)
        release.chmod(0o700)
        extra.rmdir()
        (release / 'unexpected').write_bytes(b'extra')
        (release / 'unexpected').chmod(0o444)
        release.chmod(0o555)
        with self.assertRaisesRegex(package.PackageError, 'release-inventory-mismatch'):
            package.verify(release)

    @unittest.skipUnless(os.name == 'posix', 'POSIX readonly-mode assertion')
    def test_writable_artifact_refuses_verification(self):
        release = self.prepare()
        (release / 'app.js').chmod(0o644)
        with self.assertRaisesRegex(package.PackageError, 'writable-release'):
            package.verify(release)

    def test_node_replacement_refuses_exact_runtime_identity(self):
        release = self.prepare()
        self.node.write_bytes(b'replacement-node')
        with self.assertRaisesRegex(package.PackageError, 'node-identity-changed'):
            package.verify(release)

    def test_manifest_tamper_noncanonical_bytes_and_duplicate_keys_refuse(self):
        release = self.prepare()
        manifest = release / 'release-manifest.json'
        before = manifest.read_bytes()
        self.mutate_file(manifest, before + b' ')
        with self.assertRaisesRegex(package.PackageError, 'manifest-identity-mismatch'):
            package.verify(release)
        self.mutate_file(manifest, b'{"schema":1,"schema":1}\n')
        with self.assertRaisesRegex(package.PackageError, 'duplicate-json-key'):
            package.verify(release)

    def test_unsafe_ingress_limits_routes_logging_backend_and_origin_refuse(self):
        mutations = [
            ('backend', 'address', '0.0.0.0'), ('backend', 'port', 8790),
            ('limits', 'body_bytes', 1025), ('limits', 'connections', 0),
            ('limits', 'request_ms', None), ('limits', 'absolute_body_ms', 5001),
            ('logging', 'query', True), ('logging', 'body', True),
            ('logging', 'access', True), ('routes', 'POST', ['/court', '/api/token']),
            ('tls', 'origin', 'http://court.example'),
            ('tls', 'origin', 'https://user:secret@court.example'),
            ('tls', 'origin', 'https://127.0.0.1'),
            ('tls', 'origin', 'https://court.local'),
            ('tls', 'origin', 'https://court.example/?secret=x'),
        ]
        for section, key, value in mutations:
            with self.subTest(section=section, key=key, value=value):
                proposal = package.default_ingress()
                proposal[section][key] = value
                with self.assertRaisesRegex(package.PackageError, 'unsafe-ingress'):
                    self.prepare(ingress=proposal)
                self.assert_empty_publication()
        proposal = package.default_ingress()
        proposal['unknown'] = True
        with self.assertRaisesRegex(package.PackageError, 'unsafe-ingress'):
            self.prepare(ingress=proposal)

    def test_selected_https_is_only_a_bound_proposal_not_live_qualification(self):
        proposal = package.default_ingress()
        proposal['tls']['origin'] = 'https://court.example'
        release = self.prepare(ingress=proposal)
        manifest = package.verify(release)
        self.assertEqual(manifest['ingress']['status'], 'proposal-only')
        self.assertEqual(manifest['ingress']['backend'], {'address': '127.0.0.1', 'port': 8791})
        self.assertEqual(manifest['ingress']['tls']['origin'], 'https://court.example')
        self.assertNotIn('iframe_receipt', manifest)
        self.assertNotIn('readiness', manifest)

    def test_rollback_binds_exact_verified_artifacts_and_receipt_replay_is_read_only(self):
        prior = self.prepare()
        (self.source / 'court.js').write_bytes(b'candidate next revision')
        candidate = self.prepare()
        receipt = self.root / 'plans' / 'rollback.json'
        result = package.rollback_plan(candidate, prior, receipt)
        self.assertEqual(result['status'], 'operator-review-required')
        self.assertEqual(result['actions_executed'], [])
        self.assertEqual(result['candidate']['manifest_sha256'], candidate.name.removeprefix('release-'))
        self.assertEqual(result['rollback']['manifest_sha256'], prior.name.removeprefix('release-'))
        self.assertEqual(result['proposed_steps'][-1]['expected_digest'], package.verify(prior)['court_digest'])
        before = (receipt.read_bytes(), receipt.stat().st_mtime_ns)
        self.assertEqual(package.rollback_plan(candidate, prior, receipt), result)
        self.assertEqual((receipt.read_bytes(), receipt.stat().st_mtime_ns), before)
        self.assertFalse((receipt.parent / '.prepare.lock').exists())
        receipt.chmod(0o600)
        receipt.write_bytes(b'OTHER_RECEIPT')
        with self.assertRaisesRegex(package.PackageError, 'existing-receipt-mismatch'):
            package.rollback_plan(candidate, prior, receipt)
        self.assertEqual(receipt.read_bytes(), b'OTHER_RECEIPT')

    def test_same_artifact_rollback_is_no_change_and_missing_or_tampered_prior_refuses(self):
        release = self.prepare()
        result = package.rollback_plan(release, release, self.root / 'plans/same.json')
        self.assertEqual(result['status'], 'no-change')
        self.assertEqual(result['proposed_steps'], [])
        with self.assertRaises(FileNotFoundError):
            package.rollback_plan(release, self.root / 'missing', self.root / 'plans/missing.json')
        self.mutate_file(release / 'app.js', b'broken')
        with self.assertRaisesRegex(package.PackageError, 'asset-identity-mismatch'):
            package.rollback_plan(release, release, self.root / 'plans/broken.json')
        self.assertFalse((self.root / 'plans/broken.json').exists())


NODE = os.environ.get('ABBEY_COURT_TEST_NODE') or shutil.which('node')


@unittest.skipUnless(NODE and os.name == 'posix', 'real Node loopback requires existing POSIX Node')
class RealNodePackageTest(unittest.TestCase):
    def test_actual_packaged_host_serves_exact_health_assets_and_synthetic_post(self):
        # Installed runtime, configured8791 and production state are untouched.
        with tempfile.TemporaryDirectory(prefix='abbey-court-real-package-') as folder:
            root = Path(folder).resolve()
            repo = Path(__file__).resolve().parents[1]
            source = repo / 'activity'
            self.assertTrue((source / 'server/court.mjs').is_file(), 'test imported into repo scripts/')
            release = package.prepare(source, root / 'packages', Path(NODE))
            manifest = package.verify(release)
            script = (
                f'const {{createCourtServer}}=await import({json.dumps((release / "server/court.mjs").as_uri())});'
                'const server=createCourtServer();'
                'server.listen(0,"127.0.0.1",()=>process.stdout.write(JSON.stringify({port:server.address().port})+"\\n"));'
                'process.on("SIGTERM",()=>server.close(()=>process.exit(0)));'
            )
            process = subprocess.Popen([manifest['node']['path'], '--input-type=module', '-e', script],
                                       stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                       stderr=subprocess.DEVNULL, env={'PATH': str(Path(NODE).parent)})
            try:
                with selectors.DefaultSelector() as ready:
                    ready.register(process.stdout, selectors.EVENT_READ)
                    self.assertTrue(ready.select(3), 'packaged host startup was observed')
                    line = process.stdout.readline(128)
                port = json.loads(line)['port']
                origin = f'http://127.0.0.1:{port}'
                opener = build_opener(ProxyHandler({}))
                with opener.open(origin + '/health', timeout=2) as response:
                    health = json.loads(response.read(1024))
                    self.assertEqual(health['protocol'], 2)
                    self.assertEqual(health['digest'], manifest['court_digest'])
                    self.assertEqual(response.headers['x-abbey-deployed-digest'], manifest['court_digest'])
                with opener.open(origin + '/court.js', timeout=2) as response:
                    self.assertEqual(response.read(package.MAX_ASSET_BYTES + 1), (release / 'court.js').read_bytes())
                request = Request(origin + '/court', data=json.dumps(
                    {'room': 'room-fixture-0000', 'player': 'player-fixture-0000', 'action': 'read'}
                ).encode(), headers={'Content-Type': 'application/json'}, method='POST')
                with opener.open(request, timeout=2) as response:
                    observed = json.loads(response.read(2048))
                    self.assertEqual(observed['yes'], 0)
                    self.assertEqual(observed['no'], 0)
                    self.assertEqual(response.headers['x-abbey-deployed-digest'], manifest['court_digest'])
            finally:
                if process.poll() is None:
                    process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
                process.stdout.close()
                self.assertIsNotNone(process.returncode, 'host termination observed')
                for directory, dirs, files in os.walk(root):
                    Path(directory).chmod(0o700)
                    for name in files:
                        (Path(directory) / name).chmod(0o600)


if __name__ == '__main__':
    unittest.main()
