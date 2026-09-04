#!/usr/bin/env python3
"""Reject Liquid-looking tokens in Markdown that GitHub Pages would try to parse.

This repository has no `_config.yml` and no `.nojekyll`, so the Pages build
runs Jekyll over every tracked Markdown file, including `tasks/` and `docs/`.
Liquid treats `{%` and `{{` as template syntax: an unknown tag such as a quoted
Jinja `set` fails the whole build, and a stray `{{` renders as nothing. Both
broke the Pages build on 2026-09-04, once from a spec that quoted a chat
template and once from a ledger bullet that quoted the fix.

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


def markdown_files(root: pathlib.Path) -> list[pathlib.Path]:
    """Return Markdown paths present in Git's index, not local scratch files."""
    result = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    relative_paths = result.stdout.decode("utf-8", errors="surrogateescape").split("\0")
    return sorted(
        root / pathlib.Path(path)
        for path in relative_paths
        if path and pathlib.Path(path).suffix == ".md"
    )


def main() -> int:
    failures = 0
    for path in markdown_files(ROOT):
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
