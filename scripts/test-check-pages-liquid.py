#!/usr/bin/env python3
"""Unit tests for check-pages-liquid.py's scanner."""

from __future__ import annotations

import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "check_pages_liquid", HERE / "check-pages-liquid.py"
)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules["check_pages_liquid"] = MODULE
SPEC.loader.exec_module(MODULE)
scan_text = MODULE.scan_text
markdown_files = MODULE.markdown_files


class ScanTextTests(unittest.TestCase):
    def test_plain_markdown_is_clean(self) -> None:
        self.assertEqual(scan_text("# Title\n\nSome `code` and {braces} but no tags.\n"), [])

    def test_unknown_tag_outside_raw_is_reported_with_its_line(self) -> None:
        findings = scan_text("ok\nparser rejects (`{% set %}`), so\nok\n")
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0][0], 2)
        self.assertIn("'{%'", findings[0][1])

    def test_output_braces_outside_raw_are_reported(self) -> None:
        findings = scan_text("emits {{- '<|turn>model' -}}\n")
        self.assertEqual([line for line, _ in findings], [1])

    def test_tokens_inside_a_raw_span_are_allowed(self) -> None:
        text = "{% raw %}\n```jinja\n{%- if x -%}{{ y }}{%- endif -%}\n```\n{% endraw %}\nafter\n"
        self.assertEqual(scan_text(text), [])

    def test_whitespace_control_raw_markers_count(self) -> None:
        self.assertEqual(scan_text("{%- raw -%}{% set a = 1 %}{%- endraw -%}\n"), [])

    def test_a_raw_span_left_open_is_reported_at_its_opener(self) -> None:
        findings = scan_text("intro\n{% raw %}\n{% set a = 1 %}\n")
        self.assertEqual(findings, [(2, "raw span opened here is never closed")])

    def test_a_token_after_a_closed_span_is_still_reported(self) -> None:
        findings = scan_text("{% raw %}{% set a %}{% endraw %} then {{ b }}\n")
        self.assertEqual(len(findings), 1)
        self.assertIn("'{{'", findings[0][1])

    def test_the_raw_marker_itself_quoted_in_prose_is_reported(self) -> None:
        # The 2026-09-04 ledger regression: quoting `{% raw %}` opens a span
        # that nothing closes, which Jekyll rejects the same way.
        findings = scan_text("blocks sit inside `{% raw %}`.\n")
        self.assertEqual(findings, [(1, "raw span opened here is never closed")])


class MarkdownFilesTests(unittest.TestCase):
    def test_only_git_tracked_markdown_is_returned(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            subprocess.run(
                ["git", "init", "--quiet"],
                cwd=root,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            (root / ".gitignore").write_text("ignored.md\n", encoding="utf-8")
            (root / "tracked space.md").write_text("# tracked\n", encoding="utf-8")
            nested = root / "docs" / "tracked.md"
            nested.parent.mkdir()
            nested.write_text("# nested\n", encoding="utf-8")
            (root / "ignored.md").write_text("{{ ignored }}\n", encoding="utf-8")
            (root / "untracked.md").write_text("{% untracked %}\n", encoding="utf-8")
            subprocess.run(
                ["git", "add", "--", ".gitignore", "tracked space.md", "docs/tracked.md"],
                cwd=root,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )

            self.assertEqual(
                [path.relative_to(root) for path in markdown_files(root)],
                [pathlib.Path("docs/tracked.md"), pathlib.Path("tracked space.md")],
            )


if __name__ == "__main__":
    unittest.main()
