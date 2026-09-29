#!/usr/bin/env python3
"""Set one docs/claims.json row: set_claim.py "<capability prefix>" STATUS [--note N] [--test T ...]."""
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "docs/claims.json"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("capability")
    parser.add_argument("status")
    parser.add_argument("--note")
    parser.add_argument("--test", action="append", default=[])
    args = parser.parse_args()
    data = json.loads(SOURCE.read_text(encoding="utf-8"))
    matches = [row for row in data["rows"] if row["capability"].startswith(args.capability)]
    if len(matches) != 1:
        raise SystemExit(f"expected exactly one row starting with {args.capability!r}, found {len(matches)}")
    row = matches[0]
    row["status"] = args.status
    row["tests"] = args.test
    if args.note is not None:
        row["note"] = args.note
    SOURCE.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"{row['capability']}: {args.status} ({len(args.test)} tests)")


if __name__ == "__main__":
    main()
