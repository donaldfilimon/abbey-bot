#!/usr/bin/env python3
"""Boundary and scope regressions for the Rust module ratchet."""
import importlib.util
import pathlib
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("module_size", pathlib.Path(__file__).with_name("check-rust-module-size.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ModuleSizeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.source = pathlib.Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def write(self, path, text):
        target = self.source / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")

    def test_exact_size_boundaries_and_review_threshold(self):
        self.write("main.rs", "// line\n" * 800)
        self.assertEqual(MODULE.inspect(self.source), ([], []))
        self.write("main.rs", "// line\n" * 999)
        errors, reviews = MODULE.inspect(self.source)
        self.assertFalse(errors)
        self.assertEqual(len(reviews), 1)
        self.write("main.rs", "// line\n" * 1000)
        self.assertEqual(len(MODULE.inspect(self.source)[0]), 1)

    def test_only_explicit_test_modules_and_their_descendants_are_excluded(self):
        self.write("main.rs", "#[cfg(test)]\nmod fixtures;\nmod latest;\nmod service;\n")
        self.write("fixtures.rs", "// test\n" * 1400)
        self.write("fixtures/nested.rs", "// test\n" * 1400)
        self.write("latest.rs", "// production\n" * 1000)
        self.write("service.rs", "#[cfg(test)]\npub(crate) mod tests;\n")
        self.write("service/tests.rs", "// test\n" * 1400)
        self.write("pipeline.rs", '#[cfg(test)]\n#[path = "pipeline/tests.rs"]\nmod tests;\n')
        self.write("pipeline/tests.rs", "// test\n" * 1400)
        self.assertEqual(len(MODULE.inspect(self.source)[0]), 1)
        self.assertIn("latest.rs", MODULE.inspect(self.source)[0][0])

    def test_module_suppression_is_rejected_without_banning_local_attributes(self):
        for text in ("# [allow(dead_code)]\nmod hidden;\n", "#![allow(dead_code)]\n", "#[allow(unused_imports)]\npub mod adapter;\n", "#[expect(dead_code)]\nmod adapter {}\n"):
            self.write("main.rs", text)
            self.assertEqual(len(MODULE.inspect(self.source)[0]), 1)
        self.write("main.rs", "#[allow(dead_code)]\nfn fixture_helper() {}\n")
        self.assertFalse(MODULE.inspect(self.source)[0])

    def test_comments_and_literals_cannot_hide_production_or_create_suppressions(self):
        self.write("production.rs", "// production\n" * 1000)
        for fake in (
            "/* #[cfg(test)] mod production; */",
            "// #[cfg(test)] mod production;",
            'const EXAMPLE: &str = r###"#[cfg(test)] mod production;"###;',
            'const EXAMPLE: &str = "#[cfg(test)] mod production;";',
        ):
            self.write("main.rs", fake + "\nmod production;\n")
            self.assertEqual(len(MODULE.inspect(self.source)[0]), 1)
        self.write("production.rs", "// small\n")
        self.write("main.rs", '/* nested /* #![allow(dead_code)] */ comment */\nconst EXAMPLE: &str = "#![allow(unused_imports)]";\nmod production;\n')
        self.assertFalse(MODULE.inspect(self.source)[0])

    def test_production_path_reachability_overrides_shared_test_exemption(self):
        self.write("shared.rs", "// shared production\n" * 1000)
        self.write("main.rs", '#[cfg(test)]\n#[path="shared.rs"]\nmod fixtures;\n#[path="shared.rs"]\nmod runtime;\n')
        self.assertEqual(len(MODULE.inspect(self.source)[0]), 1)

    def test_attribute_order_and_normalized_test_path_are_supported(self):
        self.write("fixtures.rs", "// test\n" * 1400)
        self.write("main.rs", '#[path="child/../fixtures.rs"]\n#[cfg(test)]\nmod fixtures;\n')
        self.assertFalse(MODULE.inspect(self.source)[0])

    def test_rust_character_quotes_do_not_mask_following_module_attributes(self):
        self.write("main.rs", "const QUOTE: char = '\"';\n#[allow(dead_code)]\nmod hidden;\n")
        self.assertEqual(len(MODULE.inspect(self.source)[0]), 1)


if __name__ == "__main__":
    unittest.main()
