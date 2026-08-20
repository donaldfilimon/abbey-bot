#!/usr/bin/env python3
"""Parse every checked-in deployment/gate Python source without importing it."""

from __future__ import annotations

import ast
import pathlib


ROOT = pathlib.Path(__file__).resolve().parents[1]


def main() -> None:
    paths = sorted((ROOT / "deploy").glob("*.py"))
    paths.extend(sorted((ROOT / "scripts").glob("*.py")))
    if not paths:
        raise SystemExit("no deployment/gate Python sources found")
    for path in paths:
        ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        print(f"python syntax: {path.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
