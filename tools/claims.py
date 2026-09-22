#!/usr/bin/env python3
"""Render and verify docs/claims.md from docs/claims.json.

Rules enforced by `--check` (the gate runs it):
- docs/claims.md is exactly the render of docs/claims.json.
- status is one of Current, Partial, Proposed, Out-of-scope.
- a Current or Partial row names at least one test, and every named test is
  declared as `test "<name>"` somewhere under src/. A capability with no test
  is Proposed, whatever the code looks like.
- a Proposed or Out-of-scope row names no tests (it would be a claim).
- the voice/music/image rows carry the dispatcher's exact note.
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "docs/claims.json"
TARGET = ROOT / "docs/claims.md"
STATUSES = ("Current", "Partial", "Proposed", "Out-of-scope")
MEDIA_NOTE = "requires Opus + MLS; C-link decision pending (Donald)"
TEST_DECL = re.compile(r'^\s*test\s+"((?:[^"\\]|\\.)*)"', re.MULTILINE)


def declared_tests():
    names = set()
    for path in (ROOT / "src").rglob("*.zig"):
        for match in TEST_DECL.finditer(path.read_text(encoding="utf-8")):
            names.add(match.group(1))
    return names


def cell(value):
    return value.replace("|", "\\|").replace("\n", " ")


def render(data):
    lines = [
        "# Claims",
        "",
        "GENERATED from `docs/claims.json` by `tools/claims.py`; edit the JSON, then run",
        "`python3 tools/claims.py`. The gate (`tools/check.sh`) fails when this file is stale,",
        "when a Current/Partial row names no test, or when a named test is not declared in `src/`.",
        "",
        "Status meanings: **Current** = implemented and exercised by the named tests;",
        "**Partial** = implemented and tested with a stated gap; **Proposed** = not implemented",
        "(or implemented without a test, which counts as not implemented); **Out-of-scope** =",
        "deliberately not part of this rewrite.",
        "",
        f"Oracle: `{data['oracle']}`. Phase in scope for this run: {data['phase_in_scope']}.",
        "",
    ]
    counts = {status: 0 for status in STATUSES}
    for row in data["rows"]:
        counts[row["status"]] += 1
    lines.append("Totals: " + ", ".join(f"{s} {counts[s]}" for s in STATUSES) + ".")
    lines.append("")
    groups = []
    for row in data["rows"]:
        if row["group"] not in groups:
            groups.append(row["group"])
    for group in groups:
        lines += [f"## {group}", "", "| Capability | Phase | Status | Tests | Note |", "|---|---|---|---|---|"]
        for row in data["rows"]:
            if row["group"] != group:
                continue
            tests = "<br>".join(f"`{cell(t)}`" for t in row.get("tests", [])) or "none"
            lines.append(f"| {cell(row['capability'])} | {row['phase']} | {row['status']} | {tests} | {cell(row.get('note', ''))} |")
        lines.append("")
    return "\n".join(lines)


def validate(data):
    errors = []
    names = declared_tests()
    seen = set()
    for row in data["rows"]:
        cap = row["capability"]
        if cap in seen:
            errors.append(f"duplicate capability: {cap}")
        seen.add(cap)
        status = row["status"]
        tests = row.get("tests", [])
        if status not in STATUSES:
            errors.append(f"{cap}: bad status {status}")
        if status in ("Current", "Partial"):
            if not tests:
                errors.append(f"{cap}: {status} without a test")
            for test in tests:
                if test not in names:
                    errors.append(f"{cap}: test not declared in src/: {test}")
        elif tests:
            errors.append(f"{cap}: {status} row must not name tests")
        if status == "Partial" and not row.get("note"):
            errors.append(f"{cap}: Partial needs a note naming the gap")
        if row.get("media") and MEDIA_NOTE not in row.get("note", ""):
            errors.append(f"{cap}: media row must carry the note '{MEDIA_NOTE}'")
    return errors


def main():
    data = json.loads(SOURCE.read_text(encoding="utf-8"))
    errors = validate(data)
    rendered = render(data) + "\n"
    if sys.argv[1:] == ["--check"]:
        if not TARGET.is_file() or TARGET.read_text(encoding="utf-8") != rendered:
            errors.append("docs/claims.md is stale: run python3 tools/claims.py")
        if errors:
            for error in errors:
                print(error, file=sys.stderr)
            return 1
        current = sum(1 for r in data["rows"] if r["status"] == "Current")
        partial = sum(1 for r in data["rows"] if r["status"] == "Partial")
        print(f"claims: {len(data['rows'])} rows in sync; {current} Current, {partial} Partial, every named test declared")
        return 0
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    TARGET.write_text(rendered, encoding="utf-8")
    print(f"wrote {TARGET.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
