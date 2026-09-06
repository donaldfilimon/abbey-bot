#!/usr/bin/env python3
"""Keep production Rust modules below 1,000 lines and report review-sized files.

External modules explicitly declared under cfg(test) are excluded, including
their descendants. Inline tests still count toward their containing source file.
The gate does not permit module-wide dead-code/import suppression to hide seams.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import re

EXTERNAL_MODULE = re.compile(
    r"(?P<attributes>(?:#\s*\[[^]]*\]\s*)*)"
    r"(?:pub(?:\([^)]*\))?\s+)?mod\s+(?P<name>\w+)\s*;"
)
INNER_SUPPRESSION = re.compile(
    r"#\s*!\s*\[\s*(?:allow|expect)\s*\([^]]*\b(?:dead_code|unused_imports)\b"
)
MODULE_ATTRIBUTES = re.compile(
    r"((?:#\s*\[[^]]*\]\s*)+)(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+"
)
SUPPRESSION = re.compile(r"\b(?:allow|expect)\s*\([^)]*\b(?:dead_code|unused_imports)\b")
RAW_STRING = re.compile(r'r(#+)?"')
CHAR_LITERAL = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'")


def mask_non_code(text: str) -> tuple[str, dict[int, str]]:
    """Mask comments/literals without moving offsets; retain only path-string data."""
    masked = list(text)
    strings: dict[int, str] = {}
    index = 0
    while index < len(text):
        start = index
        if text.startswith("//", index):
            index = text.find("\n", index)
            if index < 0:
                index = len(text)
        elif text.startswith("/*", index):
            index += 2
            depth = 1
            while index < len(text) and depth:
                if text.startswith("/*", index):
                    depth += 1
                    index += 2
                elif text.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
        elif match := CHAR_LITERAL.match(text, index):
            index = match.end()
        elif match := RAW_STRING.match(text, index):
            delimiter = '"' + (match.group(1) or "")
            content = index + len(match.group())
            end = text.find(delimiter, content)
            index = len(text) if end < 0 else end + len(delimiter)
            if end >= 0:
                strings[start] = text[content:end]
        elif text[index] == '"':
            index += 1
            while index < len(text):
                if text[index] == "\\":
                    index += 2
                elif text[index] == '"':
                    index += 1
                    break
                else:
                    index += 1
            try:
                strings[start] = json.loads(text[start:index])
            except (ValueError, TypeError):
                pass  # Unsupported path escapes cannot establish a test exemption.
        else:
            index += 1
            continue
        for offset in range(start, min(index, len(text))):
            if text[offset] != "\n":
                masked[offset] = " "
    return "".join(masked), strings


def module_edges(path: pathlib.Path, code: str, strings: dict[int, str]):
    directory = path.parent if path.name in {"main.rs", "lib.rs", "mod.rs"} else path.with_suffix("")
    for declaration in EXTERNAL_MODULE.finditer(code):
        attributes = declaration.group("attributes")
        test_only = bool(re.search(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]", attributes))
        override = None
        for attribute in re.finditer(r"#\s*\[\s*path\s*=\s*\]", attributes):
            first = declaration.start("attributes") + attribute.start()
            last = declaration.start("attributes") + attribute.end()
            literals = [value for offset, value in strings.items() if first <= offset < last]
            if len(literals) == 1:
                override = literals[0]
        module = path.parent / override if override is not None else directory / (declaration.group("name") + ".rs")
        if override is None and not module.exists():
            module = directory / declaration.group("name") / "mod.rs"
        yield module.resolve(), test_only


def inspect(source: pathlib.Path) -> tuple[list[str], list[str]]:
    source = source.resolve()
    files = {path: path.read_text(encoding="utf-8") for path in source.rglob("*.rs")}
    masked = {path: mask_non_code(text) for path, text in files.items()}
    edges = {path: list(module_edges(path, *parts)) for path, parts in masked.items()}
    excluded: set[pathlib.Path] = set()
    excluded_trees: set[pathlib.Path] = set()
    for declarations in edges.values():
        for module, test_only in declarations:
            if test_only:
                excluded.add(module)
                excluded_trees.add(module.parent if module.name == "mod.rs" else module.with_suffix(""))
    production: set[pathlib.Path] = set()
    pending = [source / "main.rs", source / "lib.rs"]
    while pending:
        path = pending.pop()
        if path in production:
            continue
        production.add(path)
        pending.extend(module for module, test_only in edges.get(path, []) if not test_only)
    errors, reviews = [], []
    for path, text in sorted(files.items()):
        relative = path.relative_to(source).as_posix()
        if path not in production and (path in excluded or any(tree in path.parents for tree in excluded_trees)):
            continue
        count = len(text.splitlines())
        if count >= 1000:
            errors.append(f"{relative}: {count} lines; production modules must have fewer than 1000")
        elif count > 800:
            reviews.append(f"{relative}: {count} lines; review responsibility boundaries")
        code = masked[path][0]
        if INNER_SUPPRESSION.search(code) or any(
            SUPPRESSION.search(attributes) for attributes in MODULE_ATTRIBUTES.findall(code)
        ):
            errors.append(f"{relative}: module-wide dead_code/unused_imports suppression is forbidden")
    return errors, reviews


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    source = args.root / "src"
    if not source.is_dir():
        parser.error("root must contain a src directory")
    errors, reviews = inspect(source)
    for note in reviews:
        print(f"review: {note}")
    for error in errors:
        print(f"error: {error}")
    if not errors:
        print("Rust production module size and suppression checks passed")
    return int(bool(errors))


if __name__ == "__main__":
    raise SystemExit(main())
