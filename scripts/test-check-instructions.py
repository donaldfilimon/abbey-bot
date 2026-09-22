#!/usr/bin/env python3
"""Drift, escape and pointer regressions for the instruction-surface gate."""
from __future__ import annotations

import importlib.util
import pathlib
import shutil
import tempfile
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("check_instructions", ROOT / "scripts/check-instructions.py")
if SPEC is None or SPEC.loader is None:  # pragma: no cover - import machinery guard
    raise RuntimeError("unable to load scripts/check-instructions.py")
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)

SURFACES = ("AGENTS.md", "CLAUDE.md", str(CHECKER.CURSOR), str(CHECKER.CODEX))


class InstructionSurfaceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        for surface in SURFACES:
            target = self.root / surface
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / surface, target)

    def tearDown(self):
        self.temporary.cleanup()

    def path(self, surface):
        return self.root / surface

    def edit(self, surface, old, new):
        text = self.path(surface).read_text(encoding="utf-8")
        self.assertIn(old, text)
        self.path(surface).write_text(text.replace(old, new, 1), encoding="utf-8")

    def test_the_repository_surfaces_agree(self):
        self.assertEqual(CHECKER.check(ROOT), [])

    def test_a_rule_changed_only_in_agents_md_is_drift_until_regenerated(self):
        self.edit("AGENTS.md", "- Keep decisions in pure modules", "- Keep every decision in pure modules")
        problems = CHECKER.check(self.root)
        self.assertEqual(len(problems), 1)
        self.assertIn("Boundaries block differs", problems[0])
        self.assertEqual(CHECKER.check(self.root, write=True), [])
        self.assertEqual(CHECKER.check(self.root), [])
        codex = tomllib.loads(self.path(str(CHECKER.CODEX)).read_text(encoding="utf-8"))
        self.assertIn("- Keep every decision in pure modules", codex["developer_instructions"])

    def test_a_hand_edit_inside_either_mirror_is_drift(self):
        self.edit(str(CHECKER.CURSOR), "- Keep decisions in pure modules", "- Keep most decisions in pure modules")
        self.assertTrue(any("Boundaries block differs" in p for p in CHECKER.check(self.root)))
        self.tearDown()
        self.setUp()
        self.edit(str(CHECKER.CODEX), "- Keep decisions in pure modules", "- Keep most decisions in pure modules")
        self.assertTrue(any("developer_instructions differs" in p for p in CHECKER.check(self.root)))

    def test_reviewer_only_text_must_match_between_editors(self):
        self.edit(str(CHECKER.CODEX), "## Review Process", "## Review Steps")
        self.assertTrue(any("developer_instructions differs" in p for p in CHECKER.check(self.root)))
        self.edit(str(CHECKER.CODEX), "## Review Steps", "## Review Process")
        self.edit(str(CHECKER.CODEX), 'name = "abbey-reviewer"', 'name = "abbey-review"')
        self.assertTrue(any("name differs" in p for p in CHECKER.check(self.root)))

    def test_an_invalid_toml_escape_fails_instead_of_loading_silently(self):
        self.edit(str(CHECKER.CODEX), "Review Process", r"Review Process \s")
        problems = CHECKER.check(self.root)
        self.assertEqual(len(problems), 1)
        self.assertIn("invalid TOML", problems[0])

    def test_backslashes_and_triple_quotes_round_trip_through_write(self):
        rule = '- Match `\\bword\\b` and quote `"""` literally.\n'
        self.edit("AGENTS.md", "- Keep decisions in pure modules", rule + "- Keep decisions in pure modules")
        self.assertEqual(CHECKER.check(self.root, write=True), [])
        self.assertEqual(CHECKER.check(self.root), [])
        codex = tomllib.loads(self.path(str(CHECKER.CODEX)).read_text(encoding="utf-8"))
        self.assertIn(rule.rstrip("\n"), codex["developer_instructions"])

    def test_claude_md_cannot_grow_back_into_a_copy(self):
        agents = self.path("AGENTS.md").read_text(encoding="utf-8")
        self.path("CLAUDE.md").write_text(agents.replace("# AGENTS.md", "# CLAUDE.md", 1), encoding="utf-8")
        self.assertEqual(CHECKER.check(self.root), ["CLAUDE.md must be exactly the AGENTS.md pointer (run --write)"])
        self.path("CLAUDE.md").unlink()
        self.assertEqual(len(CHECKER.check(self.root)), 1)
        self.assertEqual(CHECKER.check(self.root, write=True), [])
        self.assertEqual(self.path("CLAUDE.md").read_text(encoding="utf-8"), CHECKER.POINTER)

    def test_missing_markers_or_an_ambiguous_section_fail_closed(self):
        self.edit(str(CHECKER.CURSOR), CHECKER.END, "")
        with self.assertRaises(CHECKER.Drift):
            CHECKER.check(self.root)
        self.tearDown()
        self.setUp()
        self.edit("AGENTS.md", "## Learned User Preferences", "## Boundaries\n\n## Learned User Preferences")
        with self.assertRaises(CHECKER.Drift):
            CHECKER.check(self.root)

    def test_a_crlf_checkout_compares_the_same_text(self):
        for surface in SURFACES:
            text = self.path(surface).read_text(encoding="utf-8")
            self.path(surface).write_bytes(text.replace("\n", "\r\n").encode("utf-8"))
        self.assertEqual(CHECKER.check(self.root), [])


if __name__ == "__main__":
    unittest.main()
