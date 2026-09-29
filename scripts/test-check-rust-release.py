#!/usr/bin/env python3
"""Mutations must not turn missing dependencies or retired lanes into green CI."""
import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest
ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('release', ROOT/'scripts/check-rust-release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
class ReleaseTests(unittest.TestCase):
    def fixture(self, root):
        for relative in ['.github/workflows/rust.yml','check.sh','check.ps1','Cargo.toml','Dockerfile','.dockerignore']:
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
            ('persist-credentials: false','persist-credentials: true'),
            ('timeout-minutes: 60','timeout-minutes: 60\n    continue-on-error: true'),
        ]
        for old,new in mutations:
            with self.subTest(old=old),tempfile.TemporaryDirectory() as temp:
                root=Path(temp);self.fixture(root);path=root/'.github/workflows/rust.yml'
                path.write_text(path.read_text().replace(old,new))
                self.assertTrue(release.errors(root))
    def test_retired_lane_or_build_reference_fails(self):
        for relative in ['zig/build.zig','.github/workflows/zig.yml','check.sh','Dockerfile']:
            with self.subTest(path=relative),tempfile.TemporaryDirectory() as temp:
                root=Path(temp);self.fixture(root);path=root/relative;path.parent.mkdir(parents=True,exist_ok=True)
                path.write_text('zig build\n')
                self.assertTrue(release.errors(root))
if __name__=='__main__':unittest.main()
