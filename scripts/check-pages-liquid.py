#!/usr/bin/env python3
"""Reject Liquid-looking tokens in Markdown that GitHub Pages would try to parse.

This repository has no `_config.yml`, `_config.yaml`, or `.nojekyll`, so the
Pages build runs Jekyll over tracked, publishable Markdown sources, including
`tasks/` and `docs/`. Jekyll's default Markdown suffixes and special-entry
filtering are mirrored below so local-only files and non-site metadata such as
`.github/` cannot make the gate disagree with Pages. The model is pinned to
github-pages 232, Jekyll 3.10.0, jekyll-optional-front-matter 0.3.2, and
jekyll-readme-index 0.3.0. Liquid treats `{%` and `{{` as template syntax: an
unknown tag such as a quoted Jinja `set` fails the whole build, and a stray
`{{` renders as nothing. Both broke the Pages build on 2026-09-04, once from a
spec that quoted a chat template and once from a ledger bullet that quoted the
fix.

Tokens are allowed only inside a `raw` ... `endraw` span. The span markers are
matched loosely so `{%- raw -%}` counts too. A span left open is an error.
"""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]

RAW_OPEN = re.compile(r"\{%-?\s*raw\s*-?%\}")
RAW_CLOSE = re.compile(r"\{%-?\s*endraw\s*-?%\}")
TOKEN = re.compile(r"\{%|\{\{")
FRONT_MATTER_CLOSE = re.compile(r"(?:---|\.\.\.)\s*")
MARKDOWN_SUFFIXES = frozenset({".markdown", ".mkdown", ".mkdn", ".mkd", ".md"})
SPECIAL_LEADING_CHARACTERS = frozenset("._#~")
OPTIONAL_FRONT_MATTER_EXCLUSIONS = frozenset(
    {
        "README",
        "LICENSE",
        "LICENCE",
        "COPYING",
        "CODE_OF_CONDUCT",
        "CONTRIBUTING",
        "ISSUE_TEMPLATE",
        "PULL_REQUEST_TEMPLATE",
    }
)
INDEX_SUFFIXES = frozenset({".htm", ".html", ".xhtml", ".xml", *MARKDOWN_SUFFIXES})
DEFAULT_EXCLUDED_PREFIXES = (
    "Gemfile",
    "CNAME",
    "node_modules",
    "vendor/bundle/",
    "vendor/cache/",
    "vendor/gems/",
    "vendor/ruby/",
)
ROOT_MAGIC_DIRECTORIES = frozenset({"_data", "_includes", "_layouts", "_sass"})
POST_DIRECTORIES = frozenset({"_drafts", "_posts"})
CONTRACT_FILES = frozenset(
    {
        pathlib.PurePosixPath("_config.yml"),
        pathlib.PurePosixPath("_config.yaml"),
        pathlib.PurePosixPath(".nojekyll"),
    }
)
SUPPORTED_FRONT_MATTER_PATH = pathlib.PurePosixPath("docs/spec/SKILL.md")


class PagesSelectionError(RuntimeError):
    """The repository no longer matches this checker's pinned Pages model."""


def scan_text(text: str) -> list[tuple[int, str]]:
    """Return (1-indexed line, message) for every violation in `text`."""
    findings: list[tuple[int, str]] = []
    in_raw = False
    opened_at = 0
    for number, line in enumerate(text.split("\n"), start=1):
        position = 0
        while position < len(line):
            if in_raw:
                close = RAW_CLOSE.search(line, position)
                if close is None:
                    break
                in_raw = False
                position = close.end()
                continue
            open_ = RAW_OPEN.search(line, position)
            token = TOKEN.search(line, position)
            if open_ is not None and (token is None or open_.start() <= token.start()):
                in_raw = True
                opened_at = number
                position = open_.end()
                continue
            if token is None:
                break
            findings.append(
                (
                    number,
                    f"Liquid token {token.group(0)!r} outside a raw span: "
                    f"{line.strip()[:100]}",
                )
            )
            break
    if in_raw:
        findings.append((opened_at, "raw span opened here is never closed"))
    return findings


def tracked_entries(root: pathlib.Path) -> list[tuple[pathlib.PurePosixPath, str]]:
    """Return (path, mode) entries from Git's platform-neutral index format."""
    result = subprocess.run(
        ["git", "-C", str(root), "ls-files", "--stage", "-z"],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    records = result.stdout.decode("utf-8", errors="surrogateescape").split("\0")
    entries: list[tuple[pathlib.PurePosixPath, str]] = []
    for record in records:
        if not record:
            continue
        metadata, name = record.split("\t", maxsplit=1)
        mode, _object_id, stage = metadata.split(" ")
        if stage != "0":
            raise PagesSelectionError(
                f"unmerged Git index entry needs resolution before Pages selection: {name}"
            )
        entries.append((pathlib.PurePosixPath(name), mode))
    return entries


def tracked_paths(root: pathlib.Path) -> list[pathlib.PurePosixPath]:
    """Return paths present in Git's index."""
    return [path for path, _mode in tracked_entries(root)]


def has_front_matter(path: pathlib.Path) -> bool:
    """Match the opening line accepted by Jekyll's YAML-front-matter check."""
    with path.open("rb") as source:
        return re.fullmatch(rb"---\s*\r?\n", source.readline()) is not None


def front_matter_body(path: pathlib.Path) -> list[str] | None:
    """Return front-matter lines while preserving YAML's column-sensitive content."""
    if not has_front_matter(path):
        return None
    lines = path.read_text(encoding="utf-8", errors="replace").split("\n")[1:]
    for index, line in enumerate(lines):
        if FRONT_MATTER_CLOSE.fullmatch(line) is not None:
            return lines[:index]
    raise PagesSelectionError(f"front matter never closes: {path.name}")


def supported_front_matter(
    relative: pathlib.PurePosixPath, body: list[str]
) -> bool:
    """Accept only the repository's fixed skill metadata shape."""
    if relative != SUPPORTED_FRONT_MATTER_PATH or len(body) < 2:
        return False
    if body[0] != "name: discord-abbey":
        return False
    if re.fullmatch(r"description:[ \t]+[>|][+-]?", body[1]) is None:
        return False
    if any(
        separator in line
        for line in body
        for separator in ("\r", "\x85", "\u2028", "\u2029")
    ):
        return False
    return all(not line or line.startswith("  ") for line in body[2:])


def survives_default_entry_filter(path: pathlib.PurePosixPath) -> bool:
    """Mirror Jekyll 3.10's config-free ordinary-entry filtering."""
    if any(
        part[0] in SPECIAL_LEADING_CHARACTERS or part.endswith("~")
        for part in path.parts
        if part
    ):
        return False
    return not any(path.as_posix().startswith(prefix) for prefix in DEFAULT_EXCLUDED_PREFIXES)


def has_directory_index(
    readme: pathlib.PurePosixPath, paths: list[pathlib.PurePosixPath]
) -> bool:
    """Whether readme-index would find an existing index in this directory."""
    return any(
        candidate.parent == readme.parent
        and candidate.stem.lower() == "index"
        and candidate.suffix.lower() in INDEX_SUFFIXES
        and survives_default_entry_filter(candidate)
        for candidate in paths
    )


def markdown_files(root: pathlib.Path) -> list[pathlib.Path]:
    """Return indexed Markdown that the pinned GitHub Pages pipeline renders."""
    entries = tracked_entries(root)
    paths = [path for path, _mode in entries]
    contract_changes = CONTRACT_FILES.intersection(paths)
    if contract_changes:
        changed = ", ".join(sorted(str(path) for path in contract_changes))
        raise PagesSelectionError(
            f"tracked Pages configuration changed ({changed}); update the selector and tests"
        )
    magic: set[str] = set()
    for path in paths:
        if len(path.parts) > 1 and path.parts[0] in ROOT_MAGIC_DIRECTORIES:
            magic.add(path.parts[0])
        for index, part in enumerate(path.parts[:-1]):
            if part not in POST_DIRECTORIES:
                continue
            ancestor = pathlib.PurePosixPath(*path.parts[:index])
            if not ancestor.parts or survives_default_entry_filter(ancestor):
                magic.add(pathlib.PurePosixPath(*path.parts[: index + 1]).as_posix())
    if magic:
        raise PagesSelectionError(
            "Jekyll magic directories need an explicit rendering model: "
            + ", ".join(sorted(magic))
        )
    for relative, mode in entries:
        if not survives_default_entry_filter(relative):
            continue
        concrete = root.joinpath(*relative.parts)
        if mode == "120000" or concrete.is_symlink():
            raise PagesSelectionError(
                f"tracked symlink needs an explicit safe-mode model: {relative}"
            )
        body = front_matter_body(concrete)
        if body is not None and not supported_front_matter(relative, body):
            raise PagesSelectionError(
                f"tracked front matter needs an explicit rendering model: {relative}"
            )

    markdown: list[pathlib.Path] = []
    for relative in paths:
        if relative.suffix.lower() not in MARKDOWN_SUFFIXES:
            continue
        if not survives_default_entry_filter(relative):
            continue
        concrete = root.joinpath(*relative.parts)
        stem = relative.stem.upper()
        if has_front_matter(concrete):
            markdown.append(concrete)
        elif stem not in OPTIONAL_FRONT_MATTER_EXCLUSIONS:
            markdown.append(concrete)
        elif stem == "README" and not has_directory_index(relative, paths):
            markdown.append(concrete)
    return sorted(markdown, key=lambda path: path.relative_to(root).as_posix())


def main() -> int:
    failures = 0
    try:
        paths = markdown_files(ROOT)
    except PagesSelectionError as error:
        print(f"check-pages-liquid: {error}", file=sys.stderr)
        return 1
    for path in paths:
        text = path.read_text(encoding="utf-8", errors="replace")
        for line, message in scan_text(text):
            print(f"{path.relative_to(ROOT)}:{line}: {message}")
            failures += 1
    if failures:
        print(
            f"check-pages-liquid: {failures} token(s) would break or corrupt the GitHub Pages "
            "build; wrap quoted template syntax in a raw span or rewrite it as prose",
            file=sys.stderr,
        )
        return 1
    print("check-pages-liquid: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
