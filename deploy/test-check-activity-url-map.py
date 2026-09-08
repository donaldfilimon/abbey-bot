#!/usr/bin/env python3
"""Unit tests for the Activity Portal URL-map contract helper."""

from __future__ import annotations

import importlib.util
import pathlib
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from io import StringIO


ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "deploy" / "check-activity-url-map.py"

SPEC = importlib.util.spec_from_file_location("check_activity_url_map", SCRIPT)
if SPEC is None or SPEC.loader is None:  # pragma: no cover
    raise RuntimeError(f"unable to load {SCRIPT}")
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules["check_activity_url_map"] = MODULE
SPEC.loader.exec_module(MODULE)


class MappingContractTests(unittest.TestCase):
    def test_canonical_prefix_and_target_pass(self) -> None:
        self.assertEqual(MODULE.validate_prefix(MODULE.CANONICAL_PREFIX), [])
        self.assertEqual(MODULE.validate_target(MODULE.CANONICAL_TARGET), [])
        self.assertTrue(
            MODULE.is_canonical_mapping(
                MODULE.CANONICAL_PREFIX, MODULE.CANONICAL_TARGET
            )
        )

    def test_prefix_must_be_exact_slash(self) -> None:
        for bad in ("", "//", "/activity", "/*", "/.", "\\"):
            with self.subTest(bad=bad):
                reasons = MODULE.validate_prefix(bad)
                self.assertTrue(reasons, bad)
                self.assertTrue(any("PREFIX" in reason for reason in reasons))

    def test_target_rejects_https_scheme(self) -> None:
        bad = "https://donaldfilimon.github.io/abbey-bot/activity"
        reasons = MODULE.validate_target(bad)
        self.assertTrue(any("scheme" in reason for reason in reasons))

    def test_target_rejects_index_html(self) -> None:
        bad = "donaldfilimon.github.io/abbey-bot/activity/index.html"
        reasons = MODULE.validate_target(bad)
        self.assertTrue(any("index.html" in reason for reason in reasons))

    def test_target_rejects_trailing_slash(self) -> None:
        bad = "donaldfilimon.github.io/abbey-bot/activity/"
        reasons = MODULE.validate_target(bad)
        self.assertTrue(any("trailing slash" in reason for reason in reasons))

    def test_target_rejects_wrong_path(self) -> None:
        for bad in (
            "donaldfilimon.github.io/abbey-bot",
            "donaldfilimon.github.io/abbey-bot/",
            "donaldfilimon.github.io/other/activity",
            "example.com/abbey-bot/activity",
        ):
            with self.subTest(bad=bad):
                reasons = MODULE.validate_target(bad)
                self.assertTrue(reasons, bad)

    def test_public_pages_url_is_browser_check_not_iframe(self) -> None:
        self.assertEqual(
            MODULE.public_pages_url(),
            "https://donaldfilimon.github.io/abbey-bot/activity/",
        )
        self.assertNotEqual(
            MODULE.public_pages_url(), MODULE.discordsays_iframe_origin()
        )
        self.assertIn("discordsays.com", MODULE.discordsays_iframe_origin())
        self.assertIn(MODULE.APPLICATION_ID, MODULE.discordsays_iframe_origin())

    def test_whitespace_around_canonical_values_is_tolerated(self) -> None:
        self.assertEqual(MODULE.validate_prefix(" / "), [])
        self.assertEqual(
            MODULE.validate_target("  donaldfilimon.github.io/abbey-bot/activity  "),
            [],
        )


class AssetAndInventoryTests(unittest.TestCase):
    def test_repo_assets_pass(self) -> None:
        results = MODULE.check_required_assets(ROOT)
        self.assertTrue(results)
        self.assertTrue(all(item.ok for item in results), results)

    def test_pages_markdown_inventory_present(self) -> None:
        result = MODULE.check_pages_markdown_inventory(ROOT)
        self.assertTrue(result.ok, result.detail)

    def test_missing_assets_fail(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            results = MODULE.check_required_assets(root)
            self.assertTrue(any(not item.ok for item in results))
            inventory = MODULE.check_pages_markdown_inventory(root)
            self.assertFalse(inventory.ok)


class CliTests(unittest.TestCase):
    def test_main_passes_on_repo(self) -> None:
        stdout = StringIO()
        stderr = StringIO()
        with redirect_stdout(stdout), redirect_stderr(stderr):
            code = MODULE.main([])
        self.assertEqual(code, 0, stderr.getvalue())
        self.assertIn("PASS:", stdout.getvalue())
        self.assertIn("operator-gated", stdout.getvalue())
        self.assertIn("all local contract checks PASS", stdout.getvalue())

    def test_main_help(self) -> None:
        stdout = StringIO()
        with redirect_stdout(stdout):
            code = MODULE.main(["--help"])
        self.assertEqual(code, 0)
        self.assertIn("PREFIX", stdout.getvalue())
        self.assertIn(MODULE.CANONICAL_TARGET, stdout.getvalue())

    def test_main_rejects_unknown_args(self) -> None:
        stderr = StringIO()
        with redirect_stderr(stderr):
            code = MODULE.main(["--boom"])
        self.assertEqual(code, 2)

    def test_run_checks_reports_contract_and_assets(self) -> None:
        names = [item.name for item in MODULE.run_checks(ROOT)]
        self.assertIn("canonical TARGET constant", names)
        self.assertIn("asset activity/app.js", names)
        self.assertIn("pages markdown inventory", names)


if __name__ == "__main__":
    unittest.main()
