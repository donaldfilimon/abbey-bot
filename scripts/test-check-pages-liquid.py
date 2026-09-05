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
PagesSelectionError = MODULE.PagesSelectionError
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
    def initialize_repository(self, root: pathlib.Path) -> None:
        subprocess.run(
            ["git", "init", "--quiet"],
            cwd=root,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    def add(self, root: pathlib.Path, *names: str) -> None:
        subprocess.run(
            ["git", "add", "--force", "--", *names],
            cwd=root,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    def test_default_extensions_are_case_insensitive_and_literal(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            self.initialize_repository(root)
            tracked_names = [
                "docs/lower-markdown.markdown",
                "docs/upper-markdown.MARKDOWN",
                "docs/lower-mkdown.mkdown",
                "docs/upper-mkdown.MKDOWN",
                "docs/lower-mkdn.mkdn",
                "docs/upper-mkdn.MKDN",
                "docs/lower-mkd.mkd",
                "docs/upper-mkd.MKD",
                "docs/lower-md.md",
                "space [v1] ünicode.MD",
            ]
            for name in tracked_names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("# tracked\n", encoding="utf-8")
            self.add(root, *tracked_names)

            self.assertEqual(
                [path.relative_to(root) for path in markdown_files(root)],
                [pathlib.Path(name) for name in sorted(tracked_names)],
            )

    def test_markdown_lookalike_extensions_are_not_selected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            self.initialize_repository(root)
            names = ["page.mdx", "page.mdown", "page.md.txt", "page.markdown~", "page", ".md"]
            for name in names:
                (root / name).write_text("{{ not Markdown }}\n", encoding="utf-8")
            self.add(root, *names)

            self.assertEqual(markdown_files(root), [])

    def test_only_git_tracked_files_are_selected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            self.initialize_repository(root)
            (root / "tracked-ignored.md").write_text("# still tracked\n", encoding="utf-8")
            self.add(root, "tracked-ignored.md")
            (root / ".gitignore").write_text(
                "ignored.md\ntracked-ignored.md\n", encoding="utf-8"
            )
            (root / "tracked.md").write_text("# tracked\n", encoding="utf-8")
            (root / "ignored.md").write_text("{{ ignored }}\n", encoding="utf-8")
            (root / "untracked.md").write_text("{% untracked %}\n", encoding="utf-8")
            self.add(root, ".gitignore", "tracked.md")

            self.assertEqual(
                [path.relative_to(root) for path in markdown_files(root)],
                [pathlib.Path("tracked-ignored.md"), pathlib.Path("tracked.md")],
            )

    def test_special_backup_and_default_excluded_paths_are_not_selected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            self.initialize_repository(root)
            names = [
                ".github/template.md",
                "docs/.hidden/example.md",
                "_private/example.md",
                "docs/#draft.md",
                "docs/~scratch.md",
                "docs/backup.md~",
                "docs/backup~/page.md",
                "Gemfile-notes/page.md",
                "CNAME-notes/page.md",
                "gemfiles/readme.md",
                "node_modules/package/readme.md",
                "node_modules-old/package/readme.md",
                "vendor/cache/readme.md",
                "vendor/cache-extra/readme.md",
                "docs/visible/example.md",
            ]
            for name in names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("# fixture\n", encoding="utf-8")
            self.add(root, *names)

            self.assertEqual(
                [path.relative_to(root) for path in markdown_files(root)],
                [
                    pathlib.Path("docs/visible/example.md"),
                    pathlib.Path("gemfiles/readme.md"),
                    pathlib.Path("vendor/cache-extra/readme.md"),
                ],
            )

    def test_optional_front_matter_and_readme_index_rules(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            self.initialize_repository(root)
            files = {
                "README.md": "# rendered as root index\n",
                "activity/README.md": "# shadowed\n",
                "activity/index.html": "<!doctype html>\n",
                "docs/CONTRIBUTING.md": "# excluded without front matter\n",
                "docs/README.en.md": "# not a blacklisted basename\n",
            }
            for name, text in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8")
            self.add(root, *files)

            self.assertEqual(
                [path.relative_to(root) for path in markdown_files(root)],
                [
                    pathlib.Path("README.md"),
                    pathlib.Path("docs/README.en.md"),
                ],
            )

    def test_unmodeled_front_matter_fails_closed_with_jekyll_whitespace(self) -> None:
        fixtures = {
            "ordinary": "---\ntitle: Page\n---\n",
            "vertical tab": "---\v\ntitle: Page\n---\n",
            "form feed": "---\f\ntitle: Page\n---\n",
        }
        for label, text in fixtures.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                self.initialize_repository(root)
                (root / "page.md").write_text(text, encoding="utf-8")
                self.add(root, "page.md")
                with self.assertRaisesRegex(PagesSelectionError, "tracked front matter"):
                    markdown_files(root)

    def test_known_skill_front_matter_shape_is_supported(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            self.initialize_repository(root)
            skill = root / "docs" / "spec" / "SKILL.md"
            skill.parent.mkdir(parents=True)
            skill.write_text(
                "---\nname: discord-abbey\ndescription: >\n  first line\n  ---\n  still scalar text\n---\n# Skill\n",
                encoding="utf-8",
            )
            self.add(root, "docs/spec/SKILL.md")
            self.assertEqual(
                [path.relative_to(root) for path in markdown_files(root)],
                [pathlib.Path("docs/spec/SKILL.md")],
            )

    def test_known_skill_front_matter_rejects_yaml_line_break_bypasses(self) -> None:
        fixtures = {
            "next line": "name: discord-abbey\x85published: false",
            "line separator": "name: discord-abbey\u2028published: false",
            "paragraph separator": "name: discord-abbey\u2029published: false",
            "scalar continuation": "name: discord-abbey",
            "tab indentation": "name: discord-abbey",
        }
        for label, name_line in fixtures.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                self.initialize_repository(root)
                skill = root / "docs" / "spec" / "SKILL.md"
                skill.parent.mkdir(parents=True)
                continuation = (
                    "  harmless\u2028permalink: /escaped/"
                    if label == "scalar continuation"
                    else "\tinvalid YAML indentation"
                    if label == "tab indentation"
                    else "  harmless"
                )
                skill.write_text(
                    f"---\n{name_line}\ndescription: >\n{continuation}\n---\n# Skill\n",
                    encoding="utf-8",
                )
                self.add(root, "docs/spec/SKILL.md")
                with self.assertRaisesRegex(PagesSelectionError, "tracked front matter"):
                    markdown_files(root)

    def test_tracked_pages_configuration_fails_closed(self) -> None:
        for name in ("_config.yml", "_config.yaml", ".nojekyll"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                self.initialize_repository(root)
                (root / name).write_text("# contract change\n", encoding="utf-8")
                self.add(root, name)
                with self.assertRaisesRegex(PagesSelectionError, "update the selector"):
                    markdown_files(root)

    def test_untracked_pages_configuration_does_not_change_index_contract(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            self.initialize_repository(root)
            (root / "page.md").write_text("# tracked\n", encoding="utf-8")
            (root / "_config.yml").write_text("include: ['.github']\n", encoding="utf-8")
            self.add(root, "page.md")

            self.assertEqual(
                [path.relative_to(root) for path in markdown_files(root)],
                [pathlib.Path("page.md")],
            )

    def test_reachable_post_directory_fails_closed_but_excluded_one_does_not(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            self.initialize_repository(root)
            excluded = root / ".github" / "_posts" / "example.md"
            excluded.parent.mkdir(parents=True)
            excluded.write_text("# unreachable post path\n", encoding="utf-8")
            self.add(root, ".github/_posts/example.md")
            self.assertEqual(markdown_files(root), [])

        for name in ("_posts/2026-09-04-example.md", "docs/_posts/example.md"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                self.initialize_repository(root)
                post = root / name
                post.parent.mkdir(parents=True)
                post.write_text("# special reader\n", encoding="utf-8")
                self.add(root, name)
                with self.assertRaisesRegex(PagesSelectionError, "magic directories"):
                    markdown_files(root)

    def test_readme_index_selection_front_matter_fails_closed(self) -> None:
        fixtures = {
            "index permalink": {
                "activity/README.md": "# readme\n",
                "activity/index.md": "---\npermalink: /elsewhere/\n---\n",
            },
            "unpublished index": {
                "activity/README.md": "# readme\n",
                "activity/index.html": "---\npublished: false\n---\n",
            },
            "permalink-created index": {
                "activity/README.md": "# readme\n",
                "activity/landing.md": "---\n'permalink': /activity/\n---\n",
            },
            "flow unpublished index": {
                "activity/README.md": "# readme\n",
                "activity/index.html": "---\n{published: false}\n---\n",
            },
            "flow permalink index": {
                "activity/README.md": "# readme\n",
                "activity/index.md": "---\n{permalink: /elsewhere/}\n---\n",
            },
            "escaped permalink index": {
                "activity/README.md": "# readme\n",
                "activity/index.md": '---\n{"perma\\u006cink": /elsewhere/}\n---\n',
            },
            "escaped permalink landing": {
                "activity/README.md": "# readme\n",
                "activity/landing.md": '---\n{"perma\\u006cink": /activity/}\n---\n',
            },
            "escaped published page": {
                "activity/README.md": "# readme\n",
                "activity/page.md": '---\n{"pub\\x6cished": false}\n---\n',
            },
            "indented scalar delimiter": {
                "activity/README.md": "# readme\n",
                "activity/landing.md": (
                    "---\ndescription: |\n  ---\npermalink: /activity/\n---\n"
                ),
            },
            "continued quoted permalink": {
                "activity/README.md": "# readme\n",
                "activity/landing.md": (
                    '---\n? "perma\\\n  link"\n: /activity/\n---\n'
                ),
            },
        }
        for label, files in fixtures.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                self.initialize_repository(root)
                for name, text in files.items():
                    path = root / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(text, encoding="utf-8")
                self.add(root, *files)
                with self.assertRaisesRegex(PagesSelectionError, "tracked front matter"):
                    markdown_files(root)

    def test_tracked_index_symlink_fails_closed_before_suppressing_readme(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            self.initialize_repository(root)
            readme = root / "docs" / "README.md"
            index = root / "docs" / "index.html"
            readme.parent.mkdir()
            readme.write_text("# should remain visible\n", encoding="utf-8")
            index.write_text("placeholder\n", encoding="utf-8")
            self.add(root, "docs/README.md", "docs/index.html")
            blob = subprocess.run(
                ["git", "hash-object", "-w", "--stdin"],
                cwd=root,
                check=True,
                input=b"../outside.html",
                stdout=subprocess.PIPE,
            ).stdout.decode("ascii").strip()
            subprocess.run(
                [
                    "git",
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    "120000",
                    blob,
                    "docs/index.html",
                ],
                cwd=root,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            self.assertFalse(index.is_symlink())
            with self.assertRaisesRegex(PagesSelectionError, "tracked symlink"):
                markdown_files(root)

    def test_live_repository_selection_matches_current_pages_rendering(self) -> None:
        root = HERE.parent
        selected = {path.relative_to(root).as_posix() for path in markdown_files(root)}
        expected = {
            "AGENTS.md",
            "CLAUDE.md",
            "QUALITY_ASSESSMENT.md",
            "README.md",
            "activity/server/README.md",
            "contracts/abbey/corpus/README.md",
            "contracts/abbey/corpus/compatibility.md",
            "docs/2026-08-10-abbey-ai-backend-proposal.md",
            "docs/MLAI-LIVE-ACCEPTANCE.md",
            "docs/README.md",
            "docs/activities.md",
            "docs/benchmarks/2026-08-19-local-models.md",
            "docs/brand.md",
            "docs/discord-application-api-roadmap.md",
            "docs/live-test-protocol.md",
            "docs/research/2026-08-19-voice-dave-entrypoint-tools.md",
            "docs/spec/SKILL.md",
            "docs/spec/adaptivelearning.md",
            "docs/spec/appleintelligence.md",
            "docs/spec/botarchitecture.md",
            "docs/spec/brain.md",
            "docs/spec/companionapp.md",
            "docs/spec/discordbmapi.md",
            "docs/spec/multiguild.md",
            "docs/spec/platforms.md",
            "docs/spec/vision.md",
            "docs/superpowers/README.md",
            "docs/superpowers/plans/2026-08-19-finishing.md",
            "docs/superpowers/plans/2026-08-19-guild-learning-loop.md",
            "docs/superpowers/plans/2026-09-02-embedded-skills-stabilization.md",
            "docs/superpowers/plans/2026-09-02-provider-routing.md",
            "docs/superpowers/plans/2026-09-03-residual-ops.md",
            "docs/superpowers/plans/2026-09-04-abbey-bot-full-modernization.md",
            "docs/superpowers/specs/2026-08-19-guild-learning-loop-design.md",
            "docs/superpowers/specs/2026-08-19-reply-quality-speed-design.md",
            "docs/superpowers/specs/2026-08-19-tools-design.md",
            "docs/superpowers/specs/2026-08-20-live-voice-design.md",
            "docs/superpowers/specs/2026-08-21-test-module-extraction-design.md",
            "docs/superpowers/specs/2026-09-02-embedded-skills-design.md",
            "docs/superpowers/specs/2026-09-02-provider-routing-design.md",
            "docs/superpowers/specs/2026-09-04-discord-command-center-design.md",
            "docs/superpowers/specs/2026-09-04-mlx-vlm-tool-continuation-diagnosis.md",
            "docs/superpowers/specs/2026-09-04-provider-runtime-modernization-design.md",
            "docs/superpowers/specs/2026-09-04-service-observability-design.md",
            "docs/superpowers/specs/2026-09-04-voice-play-design.md",
            "patches/openmls_rust_crypto-0.5.1/CHANGELOG.md",
            "patches/openmls_rust_crypto-0.5.1/PATCH.md",
            "patches/openmls_rust_crypto-0.5.1/README.md",
            "tasks/goals.md",
            "tasks/todo.md",
        }
        self.assertEqual(selected, expected)


if __name__ == "__main__":
    unittest.main()
